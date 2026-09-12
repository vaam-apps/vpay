//! `/v1/invoices` — create, retrieve, update, list, delete, and the four
//! transitions.
//!
//! The merchant's bill to a payer (S4b). It is Stripe's `invoice`, narrowed to
//! the subset a Cameroon merchant needs to bill a phone:
//! `draft → open → paid | void | uncollectible`, a per-merchant document
//! number, lines, and a payment through the checkout that already exists.
//! `docs/flows/invoices.md` is the long version of everything below.
//!
//! **Tenancy.** Every query takes the [`MerchantScope`] the authentication
//! middleware resolved. A merchant asking for another merchant's `in_…` gets
//! the same 404, byte for byte, as one asking for an id that never existed.
//!
//! # The four rules that are this resource and not the others
//!
//! 1. **The state machine is the `WHERE` clause.** Every transition here calls
//!    a `vpay_db::invoices` method whose `UPDATE` carries `AND status =
//!    '<from>'`. This module reads an invoice *after* a refusal to decide
//!    whether the answer is a `404` or a `409` — never before one, to decide
//!    whether to write. The difference is the whole reason a paid invoice
//!    cannot be voided.
//! 2. **Finalize is the money transition.** It assigns the number out of the
//!    merchant's own sequence under a row lock, freezes the lines, computes
//!    the amounts once and opens the invoice — all in one transaction, with
//!    `invoice.finalized` inside it.
//! 3. **A draft is deleted; an issued invoice is voided.** Stripe's own
//!    shape, and here it is forced by migration `0036`'s
//!    `number_is_assigned_at_finalize`: a voided draft would be a non-draft
//!    row with no number, which the database refuses.
//! 4. **One payment at a time, and cancelling the intent is the way back.**
//!    See [`pay`].
//!
//! # What is *not* here
//!
//! Lines live in [`super::invoice_items`] — one resource, one file, and the
//! guard that freezes them belongs beside them. There is no PDF, no e-mail,
//! no tax, no credit note, no dunning and no subscription; every one of those
//! is listed with a date in `docs/flows/invoices.md`.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Map, Value};
use time::OffsetDateTime;
use vpay_core::{IntentStatus, InvoiceStatus, ids};
use vpay_db::{
    CheckoutSessions, InvoiceListPage, InvoicePatch, InvoiceRow, Invoices, NewCheckoutSession,
    NewInvoice, NewPaymentIntent, PaymentIntents, Repositories, ResponseSubject, TxOutcome,
    UnitOfWork,
};

use crate::error::ApiError;
use crate::form::VpayQuery;
use crate::model::{DeletedObject, DeletedTrue, InvoiceObject, InvoiceTag, ListObject};
use crate::v1::paging::{self, CursorKind};
use crate::v1::payment_intents::{ClaimOutcome, PostRequest, json_response};
use crate::v1::{MerchantScope, ResourceConfig};

/// The object type this module speaks about, in the API's own vocabulary.
/// One constant so a 404 for an invoice can never be spelled two ways.
pub(crate) const RESOURCE: &str = "invoice";

/// The list envelope's `url`, and the path a cursor page is read from.
const LIST_URL: &str = "/v1/invoices";

/// This resource's cursor vocabulary — `in_…`.
const CURSOR: CursorKind = CursorKind {
    prefix: ids::INVOICE_PREFIX,
    noun: "an invoice id",
};

/// `metadata` bounds — Stripe's own limits and `docs/api/README.md`'s,
/// restated here for [`super::customers`]' reason.
const METADATA_MAX_KEYS: usize = 50;
/// See [`METADATA_MAX_KEYS`].
const METADATA_MAX_KEY_CHARS: usize = 40;
/// See [`METADATA_MAX_KEYS`].
const METADATA_MAX_VALUE_CHARS: usize = 500;

/// `description`'s ceiling — migration `0036`'s `description_length`, at the
/// boundary where it can name the parameter.
///
/// The CHECK is the backstop: trip it and an over-long description comes back
/// as a `500` telling a merchant vpay is broken; trip this and it comes back
/// as a `400` naming the parameter, which is the truth.
pub(crate) const DESCRIPTION_MAX_CHARS: usize = 1000;

// ----------------------------------------------------------------- create

/// `POST /v1/invoices`'s fields, as the form decoder produces them.
///
/// Every scalar is `Option<String>` for [`super::customers::CreateParams`]'
/// reason: the wire is form-encoded, so every value arrives as text, and
/// typing them here would hand "not a timestamp" to serde — which answers
/// with `param: "body"` rather than naming the field.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CreateParams {
    customer: Option<String>,
    currency: Option<String>,
    description: Option<String>,
    /// Unix **seconds**, as Stripe spells it. Advisory — see
    /// [`vpay_db::InvoiceRow::due_date`].
    due_date: Option<String>,
    /// Bracket-encoded (`metadata[order_id]=1234`) by
    /// [`crate::form::parse_form`], exactly as on an intent.
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

/// `POST /v1/invoices` — creates a draft.
///
/// # One transaction, and `invoice.created` inside it
///
/// The insert and the event are one transaction, deliberately, and this is
/// the first resource in this repository where that is true of a plain
/// create. `customer.created` and `customer.updated` are the standing
/// counter-example (issue #66): `POST /v1/customers` is a single statement on
/// the pool, so those two events have no writer and are deliberately absent
/// from the database's event vocabulary. This resource does not copy that
/// gap — migration `0036` adds four labels and every one has a writer in the
/// same commit.
///
/// The transaction is opened *here* rather than inside `vpay-db` because the
/// event's `data` is the rendered wire object and only this crate knows that
/// shape: the handler writes the row, renders it, and appends the event
/// beside it. `vpay_db::invoices`' module header says why an invoice cannot
/// be *projected* the way a settlement's intent can.
///
/// # Ordering: the key is claimed first, then the request
///
/// `payment_intents::create`'s rule, and every failure releases the key
/// because nothing is committed before the transaction ends.
pub(crate) async fn create(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;

    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = create_once(&post, repositories.as_ref(), &config, &scope).await;
    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Verbatim,
    )
    .await
}

/// The create itself, split out of [`create`] so "a failure from here still
/// ends the claim" is one call with one error path.
async fn create_once(
    post: &PostRequest,
    repositories: &dyn Repositories,
    config: &ResourceConfig,
    scope: &MerchantScope,
) -> Result<Response, ApiError> {
    let params: CreateParams = post.form().await?;

    // The customer is resolved through the S4a helper, so a `cus_…` that is
    // not this merchant's answers the same `400` an unknown one does — never
    // a 404 and never a different sentence, or the parameter becomes an
    // oracle for another tenant's ids. It also stamps the retention clock,
    // which is what makes an invoice the *third* thing that counts as using a
    // customer (migration `0034`'s definition, widened by `0036`).
    let customer_id = super::customers::resolve_for_attachment(
        repositories,
        scope,
        Some(params.customer.as_deref().unwrap_or_default()),
    )
    .await?
    .ok_or_else(|| {
        ApiError::invalid_param(
            "customer",
            "`customer` is required on an invoice: an invoice is a bill to somebody, and \
             vpay has no way to present one that names no payer.",
        )
    })?;

    let currency = super::payment_intents::parse_currency(params.currency.as_deref(), config)?;
    let description = checked_description(present(params.description))?;
    let due_date = parse_due_date(params.due_date.as_deref())?;
    let metadata = validated_metadata(&params.metadata)?;

    let created_at = OffsetDateTime::now_utc();
    let new = NewInvoice {
        id: ids::invoice_id(),
        merchant_id: scope.merchant_id().to_owned(),
        livemode: config.livemode(),
        customer_id,
        currency_code: currency.code().to_owned(),
        due_date,
        description,
        metadata: Value::Object(metadata),
        created_at,
    };

    let row = write_with_event(
        repositories,
        EVENT_CREATED,
        InvoiceWrite::Create(Box::new(new.clone())),
    )
    .await?
    // Unreachable: the closure above returns `Some` on every path that does
    // not error, and an insert either produces a row or fails. Spelled as an
    // `ok_or_else` rather than an `expect` (ADR-0007) and answering the
    // shared refusal, so an impossible state is a `409` a merchant can retry
    // rather than a panic.
    .ok_or_else(|| not_a_draft(&new.id))?;

    // A brand new draft has no lines and no intent, so neither of the two
    // extra reads `invoice_response` makes can find anything — but it is
    // called anyway rather than short-cut, because a second rendering path
    // is how `lines` ends up shaped differently on create than on retrieve.
    invoice_response(StatusCode::CREATED, repositories, config, &row).await
}

// --------------------------------------------------------------- retrieve

/// `GET /v1/invoices/{id}`.
pub(crate) async fn retrieve(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let row = Invoices::get_for_merchant(repositories.as_ref(), scope.merchant_id(), &id)
        .await?
        .ok_or_else(|| not_found(&id))?;
    invoice_response(StatusCode::OK, repositories.as_ref(), &config, &row).await
}

// ----------------------------------------------------------------- update

/// `POST /v1/invoices/{id}`'s fields.
///
/// `Option<String>` carries three states here and two on create, for
/// [`super::customers::UpdateParams`]' reason: an absent key leaves the field
/// alone and `description=` **clears** it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct UpdateParams {
    description: Option<String>,
    due_date: Option<String>,
    /// Absent means "leave metadata alone". Present means "merge these keys",
    /// per key — Stripe's semantics.
    metadata: Option<BTreeMap<String, String>>,
}

/// `POST /v1/invoices/{id}` (and `PATCH`, which is mounted on the same
/// handler) — **draft only**.
///
/// # Two verbs, one handler, and why both are mounted
///
/// Stripe's API has no `PATCH`: a merchant's existing client, and the real
/// `stripe` package the compat suite drives, both send `POST`. That is the
/// one that has to work. `PATCH` is mounted beside it because it is the verb
/// this change was asked for and because a partial update *is* what the
/// method means — mounting only one would make somebody guess. They are the
/// same function, so the two cannot answer differently.
///
/// # Why the refusal is a `409` and not a `404`
///
/// `Invoices::update_draft`'s `UPDATE` carries `AND status = 'draft'`, so a
/// finalized invoice matches no row and comes back `None` — indistinguishable
/// from "no such invoice" and from "not yours". This handler then re-reads to
/// tell them apart: a row that exists and is not a draft is a `409` naming
/// the status, and anything else is the uniform `404`. **That read is a
/// diagnosis of a refusal that has already happened; it is never what decides
/// the write.**
pub(crate) async fn update(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = update_once(&post, repositories.as_ref(), &config, &scope, &id).await;
    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Verbatim,
    )
    .await
}

/// The update itself. See [`update`].
async fn update_once(
    post: &PostRequest,
    repositories: &dyn Repositories,
    config: &ResourceConfig,
    scope: &MerchantScope,
    id: &str,
) -> Result<Response, ApiError> {
    let params: UpdateParams = post.form().await?;

    // Scoped, so a foreign invoice is indistinguishable from a missing one.
    // Read for the *merge* — `metadata` is merged key-wise against the stored
    // map, which is what Stripe's contract says and what needs the stored
    // value. The status is checked by the `UPDATE`, not here.
    let current = Invoices::get_for_merchant(repositories, scope.merchant_id(), id)
        .await?
        .ok_or_else(|| not_found(id))?;

    let patch = InvoicePatch {
        description: patch_description(params.description)?,
        due_date: match params.due_date {
            None => None,
            Some(raw) if raw.trim().is_empty() => Some(None),
            Some(raw) => Some(parse_due_date(Some(&raw))?),
        },
        metadata: match params.metadata {
            None => None,
            Some(sent) => Some(Value::Object(merged_metadata(&current.metadata, &sent)?)),
        },
    };

    if patch.is_empty() {
        // A bodiless `POST` answers the object unchanged rather than moving
        // `updated_at` — Stripe's behaviour, and a merchant diffing on
        // timestamps depends on it. It is answered even for a *finalized*
        // invoice, deliberately: nothing is being changed, so there is
        // nothing to refuse, and a `409` for a no-op would break the
        // "touch this object" call every SDK makes.
        return invoice_response(StatusCode::OK, repositories, config, &current).await;
    }

    let row = Invoices::update_draft(
        repositories,
        scope.merchant_id(),
        id,
        &patch,
        OffsetDateTime::now_utc(),
    )
    .await?;

    match row {
        Some(row) => invoice_response(StatusCode::OK, repositories, config, &row).await,
        None => Err(refusal_reason(repositories, scope, id, RefusedBy::NotADraft).await),
    }
}

// ------------------------------------------------------------------- list

/// `GET /v1/invoices`'s query parameters — text for [`CreateParams`]' reason.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ListParams {
    limit: Option<String>,
    starting_after: Option<String>,
    ending_before: Option<String>,
    customer: Option<String>,
    status: Option<String>,
}

/// `GET /v1/invoices`.
///
/// # Two filters, and what each refuses
///
/// `status=` is checked against [`InvoiceStatus::from_wire`] and answers a
/// `400` for a label vpay does not have. That matters more than it looks: an
/// unknown status silently ignored would return the merchant's *whole* first
/// page, and one silently applied as a literal would return an empty page
/// that reads as "you have no invoices like that".
///
/// `customer=` is checked for **shape** and not for existence. An unknown but
/// well-formed `cus_…` yields an empty page rather than a `404`, for
/// `checkout_sessions`' `payment_intent` filter's reason: a filter that
/// answered `404` for an id that exists under another tenant would be an
/// existence oracle. Both are applied in the same `WHERE` as `merchant_id`,
/// so neither can widen the scope it is applied inside.
pub(crate) async fn list(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    VpayQuery(params): VpayQuery<ListParams>,
) -> Result<Response, ApiError> {
    let page = paging::list_page(
        params.limit.as_deref(),
        params.starting_after,
        params.ending_before,
        CURSOR,
    )?;

    let status = match params
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(raw) => Some(
            InvoiceStatus::from_wire(raw)
                .ok_or_else(|| {
                    ApiError::invalid_param(
                        "status",
                        "`status` must be one of `draft`, `open`, `paid`, `void` or \
                         `uncollectible`.",
                    )
                })?
                .as_wire_str()
                .to_owned(),
        ),
    };

    let customer = match params
        .customer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(raw) if ids::is_well_formed(ids::CUSTOMER_PREFIX, raw) => Some(raw.to_owned()),
        Some(_) => {
            return Err(ApiError::invalid_param(
                "customer",
                "`customer` must be a Customer id — `cus_` followed by 24 characters.",
            ));
        }
    };

    let page = InvoiceListPage {
        limit: page.limit,
        starting_after: page.starting_after,
        ending_before: page.ending_before,
        customer,
        status,
    };

    let (rows, has_more) =
        Invoices::list_page(repositories.as_ref(), scope.merchant_id(), &page).await?;

    let mut data = Vec::with_capacity(rows.len());
    for row in &rows {
        data.push(rendered(repositories.as_ref(), &config, row).await?);
    }

    json_response(StatusCode::OK, &ListObject::new(data, has_more, LIST_URL))
}

// ----------------------------------------------------------------- delete

/// `DELETE /v1/invoices/{id}` — **draft only** →
/// `{"id": …, "object": "invoice", "deleted": true}`.
///
/// # An issued invoice is never deleted
///
/// Stripe's own rule, and here it is a property of the database rather than
/// of this handler: `Invoices::delete_draft`'s statement carries `AND status =
/// 'draft'`. A finalized invoice is voided instead ([`void`]), which keeps
/// the number and the document — migration `0036` says why a number that
/// vanished is worse than a document that says "cancelled".
///
/// The lines go with it (`ON DELETE CASCADE`), which is the schema's only
/// cascade and is argued for in the migration.
pub(crate) async fn delete(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = delete_once(repositories.as_ref(), &scope, &id).await;
    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Verbatim,
    )
    .await
}

/// The delete itself. See [`delete`].
async fn delete_once(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    id: &str,
) -> Result<Response, ApiError> {
    if Invoices::delete_draft(repositories, scope.merchant_id(), id).await? {
        return json_response(
            StatusCode::OK,
            &DeletedObject {
                id: id.to_owned(),
                object: InvoiceTag,
                deleted: DeletedTrue,
            },
        );
    }
    Err(refusal_reason(repositories, scope, id, RefusedBy::NotADraft).await)
}

// ------------------------------------------------------------ transitions

/// The `type` of the event a create emits. Spelled as a constant here for
/// `vpay_db::settlement`'s reason: the type is a property of *which
/// transition this is*, and a caller free to choose it could report a created
/// invoice as a finalized one.
const EVENT_CREATED: &str = "invoice.created";
/// See [`EVENT_CREATED`].
const EVENT_FINALIZED: &str = "invoice.finalized";
/// See [`EVENT_CREATED`].
const EVENT_VOIDED: &str = "invoice.voided";

/// `POST /v1/invoices/{id}/finalize` — issues the document.
///
/// # This is the money transition
///
/// Four things happen and they happen together, in one transaction:
///
/// 1. the next number is taken out of the merchant's own sequence, under the
///    row lock that `INSERT … ON CONFLICT DO UPDATE` takes;
/// 2. the lines are frozen (every `invoice_items` write carries a
///    draft-parent guard, so the status flip *is* the freeze);
/// 3. `amount_due` is summed from the lines, once, in the same statement;
/// 4. `invoice.finalized` is appended.
///
/// A rolled-back finalize burns no number, because the sequence is an
/// ordinary table row rather than a Postgres `SEQUENCE` — see
/// `vpay_db::invoices`' `next_number_in_tx`, and migration `0036` for why
/// that matters more here than it does for Stripe.
///
/// # An invoice with no lines is refused
///
/// A deliberate divergence from Stripe, which finalizes a zero-amount invoice
/// and immediately marks it paid. vpay has no "paid without a payment"
/// transition and inventing one would be a status a merchant could not
/// explain; a zero-amount `open` invoice would instead be a document nobody
/// can pay, because `POST /v1/invoices/{id}/pay` would have to mint an intent
/// for zero. The `400` says which of the two the merchant probably meant.
/// `docs/flows/invoices.md` records the divergence.
pub(crate) async fn finalize(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    transition(
        repositories,
        config,
        scope,
        id,
        request,
        Transition::Finalize,
    )
    .await
}

/// `POST /v1/invoices/{id}/void` — cancels an issued document.
///
/// **Open only.** A draft is deleted rather than voided ([`delete`]); an
/// invoice whose payment intent has not been canceled is refused with a `409`
/// that says so, because voiding a document somebody is in the middle of
/// paying is how money moves against a bill that says nothing is owed. See
/// `vpay_db::invoices`' `NO_LIVE_INTENT` for the rule and its recovery path.
///
/// The number is **kept**. Migration `0036`'s
/// `number_is_assigned_at_finalize` makes that unavoidable rather than a
/// choice this handler could get wrong.
pub(crate) async fn void(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    transition(repositories, config, scope, id, request, Transition::Void).await
}

/// `POST /v1/invoices/{id}/mark_uncollectible` — writes an issued document
/// off.
///
/// **Open only**, and it emits **no event**. `invoice.marked_uncollectible`
/// is Stripe's fifth invoice event and is deliberately outside migration
/// `0036`'s vocabulary: this transition is a single statement on the pool, so
/// adding the label would put a value in a closed vocabulary that no code
/// produces — which is exactly what that mechanism exists to prevent. The gap
/// is recorded in `docs/flows/invoices.md`, `docs/flows/webhooks.md` and
/// `docs/status.md` rather than left to be discovered.
///
/// A merchant who needs to know still can: the transition is visible on the
/// object, and `GET /v1/invoices?status=uncollectible` lists exactly the
/// invoices it happened to.
pub(crate) async fn mark_uncollectible(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = mark_uncollectible_once(repositories.as_ref(), &config, &scope, &id).await;
    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Verbatim,
    )
    .await
}

/// The write-off itself. See [`mark_uncollectible`].
async fn mark_uncollectible_once(
    repositories: &dyn Repositories,
    config: &ResourceConfig,
    scope: &MerchantScope,
    id: &str,
) -> Result<Response, ApiError> {
    // The live-intent guard is checked here and not in the statement, because
    // `mark_uncollectible` goes through CrateStack and a correlated
    // sub-select over `payment_intents` is not expressible in a
    // `cratestack::Filter`. The window that leaves is closed by the
    // settlement's own compare-and-swap rather than by this read: a
    // settlement that lands first flips the invoice to `paid`, and the
    // `status = 'open'` filter below then matches nothing. Neither ordering
    // can produce an invoice that is both paid and written off.
    let current = Invoices::get_for_merchant(repositories, scope.merchant_id(), id)
        .await?
        .ok_or_else(|| not_found(id))?;
    refuse_if_being_paid(repositories, &current, "written off").await?;

    if !Invoices::mark_uncollectible(
        repositories,
        scope.merchant_id(),
        id,
        OffsetDateTime::now_utc(),
    )
    .await?
    {
        return Err(refusal_reason(repositories, scope, id, RefusedBy::NotOpen).await);
    }

    let row = Invoices::get_for_merchant(repositories, scope.merchant_id(), id)
        .await?
        .ok_or_else(|| not_found(id))?;
    invoice_response(StatusCode::OK, repositories, config, &row).await
}

/// Which transition a request is, for the two that share [`transition`]'s
/// shape.
///
/// An enum rather than two near-identical handlers, because what differs
/// between finalize and void is three values — the repository call, the event
/// type and the refusal's sentence — and everything else (the idempotency
/// claim, the transaction, the render) is the same seven lines. Two copies of
/// those seven lines is how one of them stops emitting an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transition {
    /// `draft → open`, with a number.
    Finalize,
    /// `open → void`.
    Void,
}

/// The two transitions that write an event, sharing one body.
async fn transition(
    repositories: Arc<dyn Repositories>,
    config: Arc<ResourceConfig>,
    scope: MerchantScope,
    id: String,
    request: Request,
    which: Transition,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = transition_once(repositories.as_ref(), &config, &scope, &id, which).await;
    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Verbatim,
    )
    .await
}

/// The transition itself. See [`transition`].
async fn transition_once(
    repositories: &dyn Repositories,
    config: &ResourceConfig,
    scope: &MerchantScope,
    id: &str,
    which: Transition,
) -> Result<Response, ApiError> {
    // The two rules that are decidable from the object rather than from the
    // statement, each refused before the write with a message that says what
    // to do. Neither replaces the compare-and-swap below.
    let current = Invoices::get_for_merchant(repositories, scope.merchant_id(), id)
        .await?
        .ok_or_else(|| not_found(id))?;

    match which {
        Transition::Finalize => {
            if current.status == InvoiceStatus::Draft.as_wire_str() {
                if Invoices::items_for_invoice(repositories, id)
                    .await?
                    .is_empty()
                {
                    return Err(ApiError::invalid_param(
                        "invoice",
                        "This invoice has no lines, so there is nothing to bill. Add at least \
                         one line with `POST /v1/invoice_items` before finalizing, or delete \
                         the draft.",
                    ));
                }
                if current.amount_due > super::payment_intents::MAX_AMOUNT {
                    return Err(too_large_to_issue(current.amount_due));
                }
            }
        }
        Transition::Void => refuse_if_being_paid(repositories, &current, "voided").await?,
    }

    let merchant_id = scope.merchant_id().to_owned();
    let (event_type, write) = match which {
        Transition::Finalize => (
            EVENT_FINALIZED,
            InvoiceWrite::Finalize {
                merchant_id,
                id: current.id.clone(),
                prefix: ids::invoice_number_prefix(),
            },
        ),
        Transition::Void => (
            EVENT_VOIDED,
            InvoiceWrite::Void {
                merchant_id,
                id: current.id.clone(),
            },
        ),
    };

    let row = write_with_event(repositories, event_type, write).await?;

    match row {
        Some(row) => invoice_response(StatusCode::OK, repositories, config, &row).await,
        None => {
            let refused = match which {
                Transition::Finalize => RefusedBy::NotADraft,
                Transition::Void => RefusedBy::NotOpen,
            };
            Err(refusal_reason(repositories, scope, &current.id, refused).await)
        }
    }
}

// -------------------------------------------------------------------- pay

/// `POST /v1/invoices/{id}/pay` — mints a payment intent for what is left and
/// hands back the invoice with a `hosted_invoice_url`.
///
/// # No new rail code, and no invoice page
///
/// The intent is an ordinary `pi_…` for `amount_remaining`, and the URL is an
/// ordinary hosted checkout session for it. A payer following it sees the
/// existing checkout page — the merchant's name and the amount — and pays
/// through the rails that already exist. There is deliberately no invoice
/// page: building one would be a second payer surface to keep correct, and
/// the one that exists is the one the browser tests already drive.
///
/// # The order is intent, session, attach — and why
///
/// The attach is a compare-and-swap on the invoice
/// (`Invoices::attach_intent`), and it needs the intent to exist first
/// because `invoices.payment_intent_id` is a real foreign key. The failure
/// mode of that order is an orphan intent nobody paid: `requires_payment_method`,
/// never confirmed, cancellable, costing nobody anything. The other order's
/// failure mode is an invoice pointing at an intent that does not exist,
/// which the foreign key refuses anyway.
///
/// A read refuses the common cases (not open, already being paid) *before*
/// the intent is minted, so the orphan is rare rather than routine — but the
/// compare-and-swap is what actually enforces it, and reaching this handler's
/// `409` is never permission to skip it.
///
/// # It takes `success_url` and `cancel_url`, and Stripe's `pay` does not
///
/// A deliberate divergence, and it is forced by what the two calls actually
/// do. Stripe's `POST /v1/invoices/{id}/pay` charges a payment method the
/// merchant already has on file, so there is nowhere to send anybody. vpay
/// has no stored payment methods on this market — a mobile-money payment is
/// a payer approving a prompt on their own handset — so paying an invoice
/// means creating a **hosted checkout session**, and migration `0028`'s
/// `urls_match_ui_mode` requires both URLs on one.
///
/// They may be **omitted** when the merchant registration carries
/// `merchant_clients[].invoices.success_url` / `.cancel_url` (issue #91, D2),
/// and a request that sends its own still wins. vpay never invents one:
/// a merchant that configured neither and sent neither gets a `400` naming
/// both, because guessing a "thank you" page would send a paying customer to
/// a `404`. `docs/flows/invoices.md` records the divergence and the default.
///
/// # A second `pay` is refused
///
/// Partial payments are out of scope. While an intent is attached and not
/// canceled, this answers `409`; cancel the intent and try again. See
/// `vpay_db::invoices`' `NO_LIVE_INTENT` for why the condition is `canceled`
/// and not "not processing".
pub(crate) async fn pay(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = pay_once(&post, repositories.as_ref(), &config, &scope, &id).await;
    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Verbatim,
    )
    .await
}

/// `POST /v1/invoices/{id}/pay`'s fields.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct PayParams {
    success_url: Option<String>,
    cancel_url: Option<String>,
}

/// The payment itself. See [`pay`].
async fn pay_once(
    post: &PostRequest,
    repositories: &dyn Repositories,
    config: &ResourceConfig,
    scope: &MerchantScope,
    id: &str,
) -> Result<Response, ApiError> {
    // The invoice is resolved **before** the body is validated, deliberately.
    // Another merchant's `in_…` must answer the uniform `404` whatever the
    // body says: a request with no `success_url` that came back `400` for a
    // foreign invoice and `404` for a missing one would let a caller tell the
    // two apart with a body they knew was incomplete.
    // `another_merchants_invoice_is_byte_identical_to_one_that_never_existed`
    // posts to this route with an empty body for exactly that reason.
    let current = Invoices::get_for_merchant(repositories, scope.merchant_id(), id)
        .await?
        .ok_or_else(|| not_found(id))?;

    let params: PayParams = post.form().await?;
    let (success_url, cancel_url) = forward_urls(&params, config, scope.merchant_id())?;

    if current.status != InvoiceStatus::Open.as_wire_str() {
        return Err(not_open(&current));
    }
    refuse_if_being_paid(repositories, &current, "paid again").await?;

    // Refused here rather than left to the rail: an intent for zero is a
    // charge no rail accepts, and the message a rail would give for it names
    // nothing a merchant can act on. `finalize` refuses a lineless invoice,
    // so reaching this needs a finalized invoice whose lines all cost
    // nothing.
    if current.amount_remaining <= 0 {
        return Err(ApiError::Conflict {
            message: "This invoice has nothing left to pay.".to_owned(),
        });
    }

    let now = OffsetDateTime::now_utc();
    let intent = PaymentIntents::insert(repositories, &intent_for(&current, scope, now)).await?;
    CheckoutSessions::create(
        repositories,
        &session_for(
            &current,
            scope,
            config,
            &intent.id,
            success_url,
            cancel_url,
            now,
        )?,
    )
    .await?;

    // The compare-and-swap. It is what enforces "one payment at a time", and
    // it is re-evaluated here even though the read above already refused the
    // same cases — the read can be stale by now, and this cannot.
    let row = Invoices::attach_intent(repositories, scope.merchant_id(), id, &intent.id, now)
        .await?
        .ok_or_else(|| {
            tracing::warn!(
                invoice_id = %id,
                payment_intent_id = %intent.id,
                "an invoice changed under a pay request; the intent that was minted for it is \
                 unattached and will never be confirmed"
            );
            ApiError::Conflict {
                message: "This invoice changed while it was being paid. Retrieve it and try \
                          again."
                    .to_owned(),
            }
        })?;

    invoice_response(StatusCode::OK, repositories, config, &row).await
}

/// The payment intent [`pay`] mints for an invoice.
///
/// `livemode` comes from the **invoice**, not from the deployment's
/// configuration read at request time: the invoice was created under whatever
/// mode was live then, and an intent that disagreed with the document it pays
/// would be a test payment against a real bill or the reverse.
///
/// `payment_method_types` is empty, exactly as `POST /v1/payment_intents`
/// with none produces: the rail is chosen at confirm time, and an invoice
/// does not narrow that choice.
///
/// `metadata` is empty and is deliberately **not** copied from the invoice.
/// A merchant's keys are theirs, attached to the object they attached them
/// to; duplicating them onto an object vpay created would put a merchant's
/// vocabulary somewhere they cannot see they put it.
fn intent_for(
    invoice: &InvoiceRow,
    scope: &MerchantScope,
    now: OffsetDateTime,
) -> NewPaymentIntent {
    NewPaymentIntent {
        id: ids::payment_intent_id(),
        merchant_id: scope.merchant_id().to_owned(),
        livemode: invoice.livemode,
        amount: invoice.amount_remaining,
        currency_code: invoice.currency_code.clone(),
        status: IntentStatus::INITIAL.as_wire_str().to_owned(),
        last_payment_error_code: None,
        last_payment_error_message: None,
        payment_method_types: Value::Array(Vec::new()),
        metadata: Value::Object(Map::new()),
        // The invoice's own number, so an operator reading a
        // `payment_intents` row knows what it is for without a join.
        description: invoice
            .number
            .as_ref()
            .map(|number| format!("Invoice {number}")),
        customer_id: Some(invoice.customer_id.clone()),
        client_secret_suffix: ids::client_secret_suffix(),
        created_at: now,
    }
}

/// The hosted checkout session [`pay`] mints for that intent.
///
/// `hosted` and not `embedded`: an invoice URL is followed by a payer in
/// their own browser, from an e-mail or a message, with no merchant page
/// around it to mount an iframe in. Migration `0028`'s `urls_match_ui_mode`
/// is what makes the two URL fields required and `return_url` refused.
///
/// # Errors
///
/// [`ApiError::CheckoutNotConfigured`] when this tenant has no registered
/// publishable key — the same refusal `POST /v1/checkout/sessions` gives, by
/// the same function, so a deployment with no checkout configured cannot
/// answer differently on the two routes.
fn session_for(
    invoice: &InvoiceRow,
    scope: &MerchantScope,
    config: &ResourceConfig,
    payment_intent_id: &str,
    success_url: String,
    cancel_url: String,
    now: OffsetDateTime,
) -> Result<NewCheckoutSession, ApiError> {
    Ok(NewCheckoutSession {
        id: ids::checkout_session_id(),
        merchant_id: scope.merchant_id().to_owned(),
        payment_intent_id: payment_intent_id.to_owned(),
        livemode: invoice.livemode,
        ui_mode: super::checkout_sessions::UI_MODE_HOSTED.to_owned(),
        success_url: Some(success_url),
        cancel_url: Some(cancel_url),
        // `NULL` for a hosted session — `urls_match_ui_mode` refuses it here
        // and requires it for `embedded`, which an invoice never is.
        return_url: None,
        customer_id: Some(invoice.customer_id.clone()),
        publishable_key: super::checkout_sessions::first_publishable_key(
            config,
            scope.merchant_id(),
        )?,
        client_secret_suffix: ids::client_secret_suffix(),
        return_token: ids::return_token(),
        expires_at: now.saturating_add(super::checkout_sessions::SESSION_LIFETIME),
        created_at: now,
    })
}

// --------------------------------------------------------------- plumbing

/// Which write a [`write_with_event`] call is making, with everything that
/// write needs.
///
/// An enum carrying owned values rather than a closure, and the reason is the
/// transaction API rather than taste: `UnitOfWork::transaction` takes a
/// `for<'t> FnOnce(&'t mut dyn TxRepositories) -> TxFuture<'t, …>`, so
/// anything the body touches has to outlive a higher-ranked borrow. Three
/// variants of owned `String` are what that costs, and they buy a body with
/// no lifetime parameters at all.
#[derive(Debug, Clone)]
enum InvoiceWrite {
    /// `POST /v1/invoices`.
    Create(Box<NewInvoice>),
    /// `POST /v1/invoices/{id}/finalize`.
    Finalize {
        /// The authenticated tenant. In the statement's `WHERE`, never
        /// compared in Rust.
        merchant_id: String,
        /// The invoice to issue.
        id: String,
        /// Used only the first time this merchant ever finalizes anything;
        /// afterwards the stored prefix wins.
        prefix: String,
    },
    /// `POST /v1/invoices/{id}/void`.
    Void {
        /// See [`Self::Finalize::merchant_id`].
        merchant_id: String,
        /// The invoice to cancel.
        id: String,
    },
}

/// Runs one invoice write inside a transaction and appends its event beside
/// it.
///
/// # Why this is one function and not three copies
///
/// Create, finalize and void differ only in the statement they run. What they
/// share is the part that is easy to get wrong and impossible to see from a
/// call site: the event has to be inside the same transaction as the write,
/// its `data` has to be the object *as written* rather than as requested, and
/// a write that matched no row must produce **no event at all**. That last
/// one is why the match below returns `Option`: `None` means the
/// compare-and-swap refused, and this function then abandons the transaction
/// rather than committing an event about a transition that did not happen.
///
/// `TxOutcome::Abandon` and not an error, because a refused transition is a
/// normal answer this API turns into a `409` — and abandoning is also what
/// makes a refused *finalize* burn no invoice number: the sequence row it
/// advanced is rolled back with everything else.
///
/// # Errors
///
/// Whatever the write returns, or [`ApiError::Internal`] if the written row
/// will not render — which migration `0036`'s CHECKs make impossible.
async fn write_with_event(
    repositories: &dyn Repositories,
    event_type: &'static str,
    write: InvoiceWrite,
) -> Result<Option<InvoiceRow>, ApiError> {
    let outcome: TxOutcome<Option<InvoiceRow>> = repositories
        .transaction(move |tx| {
            let write = write.clone();
            Box::pin(async move {
                let now = OffsetDateTime::now_utc();
                let row = match write {
                    InvoiceWrite::Create(new) => Some(tx.insert_invoice_in_tx(&new).await?),
                    InvoiceWrite::Finalize {
                        merchant_id,
                        id,
                        prefix,
                    } => {
                        tx.finalize_invoice_in_tx(&merchant_id, &id, &prefix, now)
                            .await?
                    }
                    InvoiceWrite::Void { merchant_id, id } => {
                        tx.void_invoice_in_tx(&merchant_id, &id, now).await?
                    }
                };

                let Some(row) = row else {
                    return Ok::<_, ApiError>(TxOutcome::Abandon(None));
                };

                // The event's `data` is the row this transaction just wrote,
                // rendered here — never a projection of what the request
                // asked for. A finalized invoice's `number` and `amount_due`
                // are produced *by* the statement, so a projection would be a
                // second implementation of the assignment, and the first
                // thing it would get wrong is the number.
                //
                // `lines` is rendered EMPTY and `hosted_invoice_url` `None`
                // inside an event body, deliberately. Both are second
                // queries, and issuing them on this transaction's own
                // connection while it holds the sequence row's lock would put
                // a read inside the window a concurrent finalize is waiting
                // on. The consequence is stated in `docs/flows/invoices.md`
                // rather than hidden: an `invoice.*` webhook carries the
                // invoice's own fields with `lines.data` empty, and a
                // merchant who needs the lines reads
                // `GET /v1/invoices/{id}`.
                let object = InvoiceObject::render(&row, &[], None)?;
                let data =
                    serde_json::to_value(&object).map_err(ApiError::internal_serialization)?;

                tx.insert_in_tx(&vpay_db::NewEvent {
                    id: ids::event_id(),
                    merchant_id: row.merchant_id.clone(),
                    livemode: row.livemode,
                    event_type: event_type.to_owned(),
                    object_id: row.id.clone(),
                    data,
                })
                .await?;

                Ok(TxOutcome::Commit(Some(row)))
            })
        })
        .await?;

    Ok(outcome.into_inner())
}

/// Renders a stored invoice with its lines and its hosted URL.
///
/// Two extra reads, always: the lines, and the checkout session for whichever
/// intent is paying. Both are made even when the invoice obviously has
/// neither, because a second rendering path is how `lines` ends up shaped
/// differently on create than on retrieve.
///
/// # Errors
///
/// Whatever the reads return, or [`ApiError::Internal`] if the object will
/// not render.
async fn rendered(
    repositories: &dyn Repositories,
    config: &ResourceConfig,
    row: &InvoiceRow,
) -> Result<InvoiceObject, ApiError> {
    let lines = Invoices::items_for_invoice(repositories, &row.id).await?;

    let hosted_invoice_url = match row.payment_intent_id.as_deref() {
        None => None,
        Some(intent_id) => CheckoutSessions::find_latest_by_intent(repositories, intent_id)
            .await?
            .and_then(|session| {
                super::checkout_sessions::hosted_url(&session, config.checkout_public_base_url())
            }),
    };

    InvoiceObject::render(row, &lines, hosted_invoice_url)
}

/// [`rendered`], as a response.
///
/// # Errors
///
/// See [`rendered`].
pub(crate) async fn invoice_response(
    status: StatusCode,
    repositories: &dyn Repositories,
    config: &ResourceConfig,
    row: &InvoiceRow,
) -> Result<Response, ApiError> {
    json_response(status, &rendered(repositories, config, row).await?)
}

/// Which guard turned a write away, so [`refusal_reason`] can say the right
/// thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefusedBy {
    /// The statement carried `AND status = 'draft'`.
    NotADraft,
    /// The statement carried `AND status = 'open'`.
    NotOpen,
}

/// Turns a compare-and-swap's zero-rows answer into the refusal a merchant
/// should see: a `409` naming the status if the invoice exists, and the
/// uniform `404` otherwise.
///
/// # This read never decides a write
///
/// It runs **after** the `UPDATE` has already matched nothing. Its only job
/// is to tell three indistinguishable causes apart for the error message:
/// no such invoice, another merchant's invoice, and the wrong status. If the
/// row has moved again by the time this reads it, the worst outcome is a
/// message that names a status one transition out of date — and the write it
/// describes was still correctly refused.
async fn refusal_reason(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    id: &str,
    refused: RefusedBy,
) -> ApiError {
    match Invoices::get_for_merchant(repositories, scope.merchant_id(), id).await {
        Ok(Some(row)) => match refused {
            RefusedBy::NotADraft => not_a_draft_with_status(&row),
            RefusedBy::NotOpen => not_open(&row),
        },
        Ok(None) => not_found(id),
        // The diagnosis failed. The refusal is still real, so answer the
        // shape-independent one rather than reporting a storage error for a
        // read the merchant did not ask for.
        Err(error) => ApiError::from(error),
    }
}

/// The refusal for a write that needed a draft, when the invoice's status is
/// not known.
fn not_a_draft(id: &str) -> ApiError {
    ApiError::Conflict {
        message: format!(
            "Invoice {id} is no longer a draft. Only a draft invoice can be changed or \
             deleted; an issued one is voided with POST /v1/invoices/{{id}}/void."
        ),
    }
}

/// The refusal for a write that needed a draft, naming the status it found.
fn not_a_draft_with_status(row: &InvoiceRow) -> ApiError {
    ApiError::Conflict {
        message: format!(
            "This invoice is `{}`, not `draft`. Only a draft invoice can be changed, deleted \
             or finalized; an issued one is voided with POST /v1/invoices/{{id}}/void.",
            row.status
        ),
    }
}

/// The refusal for a draft whose total is past the largest amount this API
/// represents exactly.
///
/// # Why the bound is here and not on the line, on `pay`, or in the database
///
/// It is the bound `POST /v1/payment_intents` already enforces on `amount`
/// ([`super::payment_intents::MAX_AMOUNT`]) — `2^53 - 1`, the point past
/// which a JSON number stops round-tripping through the IEEE-754 double every
/// JavaScript client parses a body with, and the same value both merchant
/// SDKs check client-side. `POST /v1/invoices/{id}/pay` does not go through
/// `parse_amount`: it mints its intent for `amount_remaining` straight off
/// the invoice. Without this check the invoice route is a second door into an
/// `amount` the intent route refuses, and the invoice's own `amount_due` is
/// past it too — on `GET /v1/invoices/{id}` and inside every `invoice.*`
/// webhook body. Measured on 2026-09-07 before the check existed: ninety-one
/// lines at both `invoice_items` ceilings finalized `200` at
/// 9,100,000,000,000,000.
///
/// **At finalize** and not on the line write, because `Invoices::add_item`
/// re-sums and commits in one transaction — by the time the new total is
/// back, the row is written, and refusing then would mean a second statement
/// to undo it. **At finalize** and not at `pay`, because a refusal there
/// leaves an `open` invoice nobody can ever pay and no route can edit. A
/// draft is still editable, so this is the only refusal a merchant can act
/// on.
///
/// **Not in the statement**, unlike every transition guard in this module,
/// and the difference is deliberate: this is a bound on how the number is
/// *rendered*, not a state-machine edge. A line added between this read and
/// the `UPDATE` could still carry the total past it — a race a merchant can
/// only win against themselves, on their own draft — where a status read
/// before a write is a race a *concurrent finalize* wins. The two are not the
/// same kind of check and this one does not pretend to be.
fn too_large_to_issue(amount_due: i64) -> ApiError {
    ApiError::invalid_param(
        "invoice",
        format!(
            "This invoice totals {amount_due}, which is more than this API represents exactly \
             ({max}). Remove or reduce a line before finalizing: a larger number is silently \
             rounded by any JSON client, including both vpay SDKs.",
            max = super::payment_intents::MAX_AMOUNT,
        ),
    )
}

/// The refusal for a transition that needed an open invoice.
fn not_open(row: &InvoiceRow) -> ApiError {
    ApiError::Conflict {
        message: format!(
            "This invoice is `{}`, not `open`. Only an open invoice can be paid, voided or \
             marked uncollectible.",
            row.status
        ),
    }
}

/// Refuses a transition on an invoice somebody is in the middle of paying.
///
/// `action` is the verb, so one function serves void, mark-uncollectible and
/// a second pay with a sentence that names what was asked for.
///
/// # Why this reads the intent rather than trusting the column
///
/// `invoices.payment_intent_id` being set is not the question — a *canceled*
/// intent leaves the column set and the invoice payable again. The question
/// is the intent's status, which lives on the other table. The same condition
/// is in SQL in `vpay_db::invoices`' `NO_LIVE_INTENT` for the two writes that
/// can carry it; this is the message half, and it is the only half for
/// `mark_uncollectible` (which goes through CrateStack and cannot carry a
/// correlated sub-select).
///
/// # Errors
///
/// [`ApiError::Conflict`] naming the intent, or a storage error from the
/// read.
async fn refuse_if_being_paid(
    repositories: &dyn Repositories,
    row: &InvoiceRow,
    action: &str,
) -> Result<(), ApiError> {
    let Some(intent_id) = row.payment_intent_id.as_deref() else {
        return Ok(());
    };
    let Some(intent) = PaymentIntents::get_by_id(repositories, intent_id).await? else {
        // The foreign key makes this impossible. Treated as "not being paid"
        // rather than as an error: the invoice is what the merchant asked
        // about, and refusing their void because a diagnostic read found
        // nothing would be a refusal they cannot act on.
        return Ok(());
    };
    if intent.status == IntentStatus::Canceled.as_wire_str() {
        return Ok(());
    }

    Err(ApiError::Conflict {
        message: format!(
            "This invoice is being paid by PaymentIntent {intent_id} and cannot be {action}. \
             Cancel that intent with POST /v1/payment_intents/{{id}}/cancel first — vpay \
             will not void a document while money may still move against it."
        ),
    })
}

/// The refusal for an invoice this merchant does not have.
pub(crate) fn not_found(id: &str) -> ApiError {
    ApiError::NotFound {
        resource: RESOURCE,
        id: id.to_owned(),
    }
}

/// `pay`'s two forwarding URLs: whatever the request sent, else whatever this
/// merchant configured, validated by exactly the rules
/// `POST /v1/checkout/sessions` applies to its own.
///
/// # Request-then-default, in that order
///
/// A request that carries a URL **wins** over the configured one (D2). A
/// merchant may legitimately want one bill to land somewhere else — a
/// one-off, a campaign page — and a configured default that could not be
/// overridden would make that impossible without an operator editing the
/// deployment's YAML.
///
/// It also makes the key safe to add to a running deployment: every request
/// that worked before this existed carries both URLs and behaves identically.
///
/// # Both are validated, whichever they came from
///
/// Through [`super::checkout_sessions::checked_forward_url`] rather than a
/// copy, and on **both** paths rather than only on the request's: the scheme
/// allow-list, the livemode `https` rule and the length bound are properties
/// of *where a payer's browser may be sent*, not of which route asked or of
/// which document the value came out of. `vpay_config`'s
/// `validate_invoice_urls` refuses a malformed configured value at boot; this
/// is what stops the two rule sets from ever diverging, and a second copy is
/// how one of the surfaces ends up accepting `javascript:`.
///
/// # Missing values are named together
///
/// A merchant that configured neither and sent neither gets **one** `400`
/// naming both, rather than one about `success_url` followed — after they fix
/// it — by another about `cancel_url`. Two round trips to learn two halves of
/// one mistake is the shape this repository's error messages avoid.
///
/// The message is kept under `ApiError`'s 200-character ceiling on purpose:
/// past it the envelope truncates with an ellipsis, and the first casualty
/// would be the sentence that says a merchant may configure these once
/// instead of sending them every time — the whole point of D2.
///
/// # Errors
///
/// [`ApiError::invalid_param`] naming every parameter that is absent, or the
/// one that is malformed.
fn forward_urls(
    params: &PayParams,
    config: &ResourceConfig,
    merchant_id: &str,
) -> Result<(String, String), ApiError> {
    let defaults = config.invoice_url_defaults(merchant_id);

    let resolve = |sent: Option<String>, configured: Option<&String>| {
        present(sent).or_else(|| present(configured.cloned()))
    };
    let success_url = resolve(
        params.success_url.clone(),
        defaults.and_then(|urls| urls.success_url.as_ref()),
    );
    let cancel_url = resolve(
        params.cancel_url.clone(),
        defaults.and_then(|urls| urls.cancel_url.as_ref()),
    );

    let missing: Vec<&'static str> = [
        ("success_url", success_url.is_none()),
        ("cancel_url", cancel_url.is_none()),
    ]
    .into_iter()
    .filter_map(|(param, absent)| absent.then_some(param))
    .collect();

    if let Some(first) = missing.first() {
        let named = missing
            .iter()
            .map(|param| format!("`{param}`"))
            .collect::<Vec<_>>()
            .join(" and ");
        return Err(ApiError::invalid_param(
            *first,
            format!(
                "{named} must be sent, or configured as `merchant_clients[].invoices`: paying \
                 an invoice creates a hosted checkout session and vpay cannot guess where to \
                 send your payer."
            ),
        ));
    }

    // `Vec::first` above proved both are `Some`; these two `ok_or_else` arms
    // are unreachable and are spelled as a fallible resolve rather than an
    // `expect` because `clippy.toml` denies `expect` outside tests, and
    // because an unreachable branch that answers the honest refusal is
    // cheaper than one that panics a request thread.
    let success_url = success_url.ok_or_else(|| missing_url("success_url"))?;
    let cancel_url = cancel_url.ok_or_else(|| missing_url("cancel_url"))?;

    super::checkout_sessions::checked_forward_url(&success_url, "success_url", config.livemode())?;
    super::checkout_sessions::checked_forward_url(&cancel_url, "cancel_url", config.livemode())?;
    Ok((success_url, cancel_url))
}

/// The refusal for a forwarding URL that is neither sent nor configured.
///
/// Its own function because [`forward_urls`] produces it from two places —
/// the message that names every absent parameter at once, and the
/// unreachable arms that keep the resolve total.
fn missing_url(param: &'static str) -> ApiError {
    ApiError::invalid_param(
        param,
        format!(
            "`{param}` must be sent, or configured as `merchant_clients[].invoices`: paying an \
             invoice creates a hosted checkout session and vpay cannot guess where to send \
             your payer."
        ),
    )
}

/// Blank is absent, on create and in a value position everywhere else —
/// [`super::customers::present`]'s rule.
fn present(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty())
}

/// One bounded description, or a `400` naming it.
///
/// Characters, not bytes: the column's CHECK is `char_length`, and counting
/// bytes would refuse a legal description whose letters are not ASCII.
pub(crate) fn checked_description(value: Option<String>) -> Result<Option<String>, ApiError> {
    if let Some(value) = value.as_deref()
        && value.chars().count() > DESCRIPTION_MAX_CHARS
    {
        return Err(ApiError::invalid_param(
            "description",
            format!("`description` must be at most {DESCRIPTION_MAX_CHARS} characters."),
        ));
    }
    Ok(value)
}

/// `description` on update: three states, bounded.
fn patch_description(raw: Option<String>) -> Result<Option<Option<String>>, ApiError> {
    match raw {
        None => Ok(None),
        Some(raw) => Ok(Some(checked_description(present(Some(raw)))?)),
    }
}

/// `due_date` as unix **seconds**, or a `400` naming it.
///
/// Seconds and not milliseconds, and not RFC 3339: it is the format every
/// timestamp on this API uses in both directions, and Stripe's own
/// `due_date`.
///
/// A date in the past is **accepted**, deliberately. It is the shape a
/// merchant produces when they back-date a bill they are entering late, and
/// vpay acts on this column for nothing — refusing it would be vpay having an
/// opinion about a merchant's paperwork.
fn parse_due_date(raw: Option<&str>) -> Result<Option<OffsetDateTime>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let seconds: i64 = raw.parse().map_err(|_error| {
        ApiError::invalid_param(
            "due_date",
            "`due_date` must be a Unix timestamp in seconds.",
        )
    })?;
    OffsetDateTime::from_unix_timestamp(seconds)
        .map(Some)
        .map_err(|_error| {
            ApiError::invalid_param("due_date", "`due_date` is not a representable instant.")
        })
}

/// The merchant's `metadata` on create, bounded.
pub(crate) fn validated_metadata(
    sent: &BTreeMap<String, String>,
) -> Result<Map<String, Value>, ApiError> {
    if sent.len() > METADATA_MAX_KEYS {
        return Err(metadata_too_many_keys());
    }
    let mut out = Map::new();
    for (key, value) in sent {
        if key.chars().count() > METADATA_MAX_KEY_CHARS {
            return Err(metadata_key_too_long());
        }
        if value.chars().count() > METADATA_MAX_VALUE_CHARS {
            return Err(metadata_value_too_long());
        }
        out.insert(key.clone(), Value::String(value.clone()));
    }
    Ok(out)
}

/// Stripe's key-wise metadata merge — [`super::customers`]' function, and the
/// same bound-the-result rule: a merchant adding one key to an invoice that
/// already has fifty must be refused, and bounding the delta would let them
/// add one key fifty times.
///
/// # Errors
///
/// [`ApiError::invalid_param`] naming `metadata`, or [`ApiError::Internal`]
/// for a stored `metadata` that is not an object — which
/// `metadata_is_object` makes impossible.
fn merged_metadata(
    stored: &Value,
    sent: &BTreeMap<String, String>,
) -> Result<Map<String, Value>, ApiError> {
    let mut merged = stored
        .as_object()
        .cloned()
        .ok_or_else(|| ApiError::Internal("invoices.metadata is not an object".to_owned()))?;

    for (key, value) in sent {
        if key.chars().count() > METADATA_MAX_KEY_CHARS {
            return Err(metadata_key_too_long());
        }
        if value.is_empty() {
            // Stripe's per-key delete. Not `Value::Null`, which would read
            // back as a key the merchant thinks they removed.
            merged.remove(key);
            continue;
        }
        if value.chars().count() > METADATA_MAX_VALUE_CHARS {
            return Err(metadata_value_too_long());
        }
        merged.insert(key.clone(), Value::String(value.clone()));
    }

    if merged.len() > METADATA_MAX_KEYS {
        return Err(metadata_too_many_keys());
    }
    Ok(merged)
}

/// See [`METADATA_MAX_KEYS`].
fn metadata_too_many_keys() -> ApiError {
    ApiError::invalid_param(
        "metadata",
        format!("`metadata` may hold at most {METADATA_MAX_KEYS} keys."),
    )
}

/// See [`METADATA_MAX_KEY_CHARS`].
fn metadata_key_too_long() -> ApiError {
    ApiError::invalid_param(
        "metadata",
        format!("A `metadata` key may be at most {METADATA_MAX_KEY_CHARS} characters."),
    )
}

/// See [`METADATA_MAX_VALUE_CHARS`].
fn metadata_value_too_long() -> ApiError {
    ApiError::invalid_param(
        "metadata",
        format!("A `metadata` value may be at most {METADATA_MAX_VALUE_CHARS} characters."),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{METADATA_MAX_KEYS, PayParams, forward_urls, parse_due_date, validated_metadata};
    use crate::ResourceConfig;
    use crate::test_fixtures::config_with_invoice_defaults;

    const MERCHANT: &str = "acme-cameroon-tenant";

    /// `forward_urls` over a `ResourceConfig` built from `config`.
    fn resolve(
        config: &vpay_config::Config,
        success_url: Option<&str>,
        cancel_url: Option<&str>,
    ) -> Result<(String, String), crate::ApiError> {
        let resource_config =
            ResourceConfig::from_config(config).expect("the fixture projects onto the port");
        forward_urls(
            &PayParams {
                success_url: success_url.map(str::to_owned),
                cancel_url: cancel_url.map(str::to_owned),
            },
            &resource_config,
            MERCHANT,
        )
    }

    /// A **malformed URL on the request does not fall back to the configured
    /// one.** It is refused, naming the parameter the caller sent.
    ///
    /// This is the shape of the bug D2 could have introduced and no delivered
    /// case covered: `present(sent).or_else(|| present(configured))` resolves
    /// the *request's* value whenever it is non-blank, so a
    /// `success_url=javascript:alert(1)` reaches `checked_forward_url` and is
    /// refused there — rather than being quietly replaced by the merchant's
    /// configured page, which would turn a caller's mistake into a silent
    /// redirect somewhere else and hide the mistake forever.
    ///
    /// The blank case is the deliberate exception and is asserted beside it:
    /// `success_url=` is what a client templating an optional field emits,
    /// and `present` treats it as absent, so the configured value wins.
    #[test]
    fn a_malformed_url_on_the_request_is_refused_rather_than_replaced_by_the_default() {
        let config = config_with_invoice_defaults(
            false,
            Some("https://shop.acme.example/thanks"),
            Some("https://shop.acme.example/basket"),
        );

        let error = resolve(&config, Some("javascript:alert(1)"), None)
            .expect_err("a scheme a payer may not be sent to is refused");
        assert_eq!(
            error.param(),
            Some("success_url"),
            "the refusal names what the caller sent, not what the operator configured"
        );

        // Blank is absent — the one case where the configured value wins over
        // something the request carried.
        assert_eq!(
            resolve(&config, Some("   "), None).expect("blank falls back"),
            (
                "https://shop.acme.example/thanks".to_owned(),
                "https://shop.acme.example/basket".to_owned()
            )
        );
    }

    /// One configured and one sent resolve **independently**, and the `400`
    /// names only what is genuinely absent.
    #[test]
    fn each_url_resolves_on_its_own_and_the_refusal_names_only_what_is_missing() {
        let only_success = config_with_invoice_defaults(false, Some("https://a.example/ok"), None);
        assert_eq!(
            resolve(&only_success, None, Some("https://b.example/cancelled"))
                .expect("one from the file, one from the request"),
            (
                "https://a.example/ok".to_owned(),
                "https://b.example/cancelled".to_owned()
            )
        );

        let error = resolve(&only_success, None, None).expect_err("cancel_url is nowhere");
        assert_eq!(error.param(), Some("cancel_url"));
        let message = format!("{error}");
        assert!(
            message.contains("`cancel_url`") && !message.contains("`success_url`"),
            "a merchant that configured half is told about the other half only: {message}"
        );
    }

    /// The refusal naming **both** parameters survives `ApiError`'s
    /// 200-character ceiling — asserted on the **envelope**, not on `Display`.
    ///
    /// `bounded_message` truncates the public `error.message` at 200
    /// characters and appends `…`. The clause that says these may be
    /// configured once instead of sent every time is the last one in the
    /// sentence, so it is the first casualty if the message ever grows — and
    /// the whole point of D2 is what it says. `Display` carries a
    /// 41-character `invalid request parameter …` prefix that never reaches a
    /// caller, so this reads the JSON a merchant actually receives.
    #[tokio::test]
    async fn the_refusal_naming_both_urls_reaches_the_wire_untruncated() {
        use axum::response::IntoResponse as _;

        let error = resolve(&config_with_invoice_defaults(false, None, None), None, None)
            .expect_err("neither configured nor sent");
        let bytes = axum::body::to_bytes(error.into_response().into_body(), usize::MAX)
            .await
            .expect("reading the envelope succeeds");
        let envelope: serde_json::Value =
            serde_json::from_slice(&bytes).expect("the envelope is JSON");
        let message = envelope
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(serde_json::Value::as_str)
            .expect("a 400 carries a message")
            .to_owned();

        assert!(
            message.contains("`success_url`") && message.contains("`cancel_url`"),
            "one refusal names both missing parameters: {message}"
        );
        assert!(
            message.contains("`merchant_clients[].invoices`"),
            "the clause that says a merchant may configure these once must survive: {message}"
        );
        assert!(
            !message.contains('…'),
            "the message is past the 200-character ceiling and lost its tail ({} characters): \
             {message}",
            message.chars().count()
        );
        assert_eq!(
            envelope
                .get("error")
                .and_then(|error| error.get("param"))
                .and_then(serde_json::Value::as_str),
            Some("success_url"),
            "`param` names the first absent one, so a client can still point at a field"
        );
    }

    /// **A configured URL is validated on the request path too**, and
    /// livemode is where that second check is visible.
    ///
    /// `vpay_config`'s `validate_invoice_urls` refuses `http://` under
    /// `deployment.livemode` at boot, so a deployment carrying one never
    /// starts and this belt can never fire in production. That is exactly why
    /// it needs a test: the argument for keeping it is that a *configured*
    /// value and a *passed* one must be admitted by one rule, and the cheapest
    /// way to lose that is for someone to notice the boot check and delete the
    /// request-time one as redundant.
    ///
    /// **The decisive mutation:** drop the two `checked_forward_url` calls in
    /// `forward_urls` — or apply them only to the value the request carried —
    /// and this case passes an `http://` page to a livemode payer.
    #[test]
    fn a_configured_url_is_held_to_the_livemode_https_rule_at_request_time_as_well() {
        let livemode = config_with_invoice_defaults(
            true,
            Some("http://shop.acme.example/thanks"),
            Some("https://shop.acme.example/basket"),
        );
        let error = resolve(&livemode, None, None)
            .expect_err("a livemode payer is not forwarded over plaintext");
        assert_eq!(error.param(), Some("success_url"));

        // And the same pair is fine off livemode, so this is the https rule
        // firing rather than the fixture being malformed.
        assert!(
            resolve(
                &config_with_invoice_defaults(
                    false,
                    Some("http://shop.acme.example/thanks"),
                    Some("https://shop.acme.example/basket"),
                ),
                None,
                None
            )
            .is_ok()
        );
    }

    /// `due_date` is unix seconds, and the three ways it can be wrong each
    /// name the parameter rather than the body.
    #[test]
    fn a_due_date_is_unix_seconds_or_a_four_hundred() {
        assert_eq!(
            parse_due_date(Some("1757222400")).expect("a plain timestamp parses"),
            Some(
                time::OffsetDateTime::from_unix_timestamp(1_757_222_400)
                    .expect("a representable instant")
            )
        );
        // Absent and blank are the same answer: `due_date=` is what a client
        // templating an optional field emits when it has none.
        assert_eq!(parse_due_date(None).expect("absent is None"), None);
        assert_eq!(parse_due_date(Some("  ")).expect("blank is None"), None);

        for wrong in ["2026-09-07", "1757222400.0", "not-a-date"] {
            let error = parse_due_date(Some(wrong)).expect_err("only integers parse");
            assert_eq!(error.param(), Some("due_date"), "for {wrong}");
        }
        // A date in the past is accepted, deliberately — see the function.
        assert!(
            parse_due_date(Some("0"))
                .expect("the epoch is a date")
                .is_some()
        );
    }

    /// The metadata bounds refuse at the boundary and not one past it.
    #[test]
    fn metadata_is_bounded_by_key_count_key_length_and_value_length() {
        let at_the_limit: BTreeMap<String, String> = (0..METADATA_MAX_KEYS)
            .map(|index| (format!("k{index}"), "v".to_owned()))
            .collect();
        assert_eq!(
            validated_metadata(&at_the_limit)
                .expect("fifty keys is legal")
                .len(),
            METADATA_MAX_KEYS
        );

        let one_too_many: BTreeMap<String, String> = (0..=METADATA_MAX_KEYS)
            .map(|index| (format!("k{index}"), "v".to_owned()))
            .collect();
        assert_eq!(
            validated_metadata(&one_too_many)
                .expect_err("fifty-one keys is not")
                .param(),
            Some("metadata")
        );

        let long_key = BTreeMap::from([("k".repeat(41), "v".to_owned())]);
        assert_eq!(
            validated_metadata(&long_key)
                .expect_err("a 41-character key is refused")
                .param(),
            Some("metadata")
        );

        let long_value = BTreeMap::from([("k".to_owned(), "v".repeat(501))]);
        assert_eq!(
            validated_metadata(&long_value)
                .expect_err("a 501-character value is refused")
                .param(),
            Some("metadata")
        );
    }
}
