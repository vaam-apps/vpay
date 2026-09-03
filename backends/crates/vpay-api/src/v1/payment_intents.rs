//! `/v1/payment_intents` — create, retrieve, list, confirm, cancel.
//!
//! Everything a merchant can do to a payment intent through this API. As of
//! Step 3 that includes a confirm that a rail *accepts*: `confirm` submits
//! through the adapter and, on success, commits what the rail said before
//! answering — so a `200` here is a payment a rail is genuinely working on,
//! not a rendered optimism (`docs/status.md`, `AGENTS.md`'s second rule).
//!
//! # Three orderings this file exists to get right
//!
//! **Tenancy.** Every query takes the [`MerchantScope`] the authentication
//! middleware resolved, and there is no code path here that reads a payment
//! intent without one. A merchant asking for another merchant's `pi_…` gets
//! the same 404, byte for byte, as one asking for an id that never existed —
//! see [`ApiError::NotFound`].
//!
//! **Idempotency.** Every `POST` needs an `Idempotency-Key` (D7). The key is
//! claimed *atomically* — one `INSERT … ON CONFLICT`, in
//! `vpay_db::idempotency::claim` — so two concurrent requests carrying one
//! key cannot both proceed, and the loser is told "in progress" rather than
//! being allowed to create a second intent. A claim is always ended, on
//! every path: stored (the response is replayable) or released (the retry
//! must re-execute) — see `PostRequest::finish` below for which is which,
//! and why a key that is merely left behind is the dangerous third option.
//! "Every path" is meant literally, including the ones that fail *after* the
//! work was done: a body that cannot be read back, a body that is not JSON,
//! and a failed write to `idempotency_keys` each release before returning,
//! because the alternative is the key staying `in_flight` until it expires
//! and every retry under it being answered "still in progress".
//!
//! The claim is carried as the `claim_id` `vpay_db::idempotency::claim`
//! minted, never as the key alone: an expired claim is reclaimable, so
//! addressing the row by key would let a request that stalled past its
//! window overwrite or delete the claim that replaced it.
//!
//! **Write before network.** `confirm` commits the charge row, with the
//! `provider_reference_id` it will submit under, *before* the adapter is
//! called, and records the attempt in `provider_requests` before that call
//! too. `docs/flows/crash-safety.md`: never let a payer act on a transaction
//! you cannot name. The rows left behind by a submission that never happened
//! are deliberate — they are what a recovery pass will find.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{FromRequest as _, FromRequestParts as _, Path, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse as _, Response};
use serde::Deserialize;
use serde_json::{Map, Value};
use time::OffsetDateTime;
use uuid::Uuid;
use vpay_core::state::{IntentStatus, Transition, next_status};
use vpay_core::{Currency, Money, ProviderFlow, ids};
use vpay_db::{
    ChargeRow, IdempotencyClaim, IdempotencyRecord, IdempotencyStoreOutcome, NewCharge,
    NewPaymentIntent, PgPool, charges, idempotency, payment_intents, provider_requests,
};
use vpay_provider::{ChargeRef, ProviderAdapter, ProviderError, Submitted};

use crate::error::ApiError;
use crate::form::{VpayForm, VpayQuery};
use crate::idempotency::{IdempotencyKey, request_hash};
use crate::model::{ListObject, NextAction, PaymentIntentObject, RedirectToUrl};
use crate::v1::paging::{self, CursorKind};
use crate::v1::{MerchantScope, RailConfig, ResourceConfig};

/// The object type this module speaks about, in the API's own vocabulary.
/// One constant so a 404 for an intent can never be spelled two ways.
const RESOURCE: &str = "payment_intent";

/// The list envelope's `url`, and the path a cursor page is read from.
const LIST_URL: &str = "/v1/payment_intents";

/// `metadata` bounds. Stripe's own limits, and the same numbers the
/// `metadata_is_object` CHECK leaves the API responsible for: a JSONB column
/// will happily store a megabyte of it, so the bound has to be here.
const METADATA_MAX_KEYS: usize = 50;
const METADATA_MAX_KEY_CHARS: usize = 40;
const METADATA_MAX_VALUE_CHARS: usize = 500;

/// `description` bound, mirroring the `description_length` CHECK in migration
/// 0014. Checked here as well so a merchant gets a named parameter and a
/// sentence rather than a database constraint violation rendered as a 500.
const DESCRIPTION_MAX_CHARS: usize = 1000;

/// The largest amount this API accepts, in minor units.
///
/// `2^53 - 1`, matching both SDKs' own client-side check
/// (`sdks/rust/src/validate.rs`, `sdks/nodejs/src/validate.ts`): every value
/// on this wire has to survive a JSON number in a JavaScript client, and one
/// past this bound would be silently rounded there. The column is `BIGINT`
/// and could hold more; agreeing with the SDKs matters more than using the
/// column's full range for amounts no rail would accept anyway.
const MAX_AMOUNT: i64 = (1_i64 << 53) - 1;

/// This resource's cursor vocabulary — `pi_…`, per `vpay_core::ids`.
///
/// `pub(crate)` so `crate::v1::paging`'s tests can prove that this list
/// refuses an `evt_…` cursor and the event list refuses a `pi_…` one, which
/// is the reason the prefix is a parameter of `validated_cursor` rather than
/// a constant inside it.
pub(crate) const CURSOR: CursorKind = CursorKind {
    prefix: ids::PAYMENT_INTENT_PREFIX,
    noun: "a payment intent id",
};

// ----------------------------------------------------------------- create

/// `POST /v1/payment_intents`'s fields, as the form decoder produces them.
///
/// Every field is `Option<String>` (or a collection of them) although
/// `amount` is a number and `payment_method_types` is a list of rail codes:
/// the wire is form-encoded, so *every* value arrives as text
/// (`crate::form`), and typing them here would hand the "not a number" case
/// to serde — which would answer with `param: "body"` and a sentence about
/// the request's shape. Parsing them below instead is what lets the answer
/// name `amount` and say what is wrong with it, which is the whole point of
/// Stripe's `param` field.
#[derive(Debug, Deserialize)]
struct CreateParams {
    amount: Option<String>,
    currency: Option<String>,
    #[serde(default)]
    payment_method_types: Vec<String>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
    description: Option<String>,
    /// Stripe's `confirm`, accepted **only so it can be refused**.
    ///
    /// vpay has no confirm-on-create: creation and confirmation are two
    /// requests (`docs/flows/payment-lifecycle.md`), and `CreateParams` has
    /// no `deny_unknown_fields`, so before this field existed a merchant who
    /// copied a Stripe snippet with `confirm: true` got a `200`, an intent in
    /// `requires_payment_method`, and the belief that they had charged
    /// someone. A field that is dropped silently is the one shape of
    /// incompatibility a merchant cannot debug from the response
    /// (`docs/plans/2026-09-03-step5b-stripe-sdk.md` §2.6).
    ///
    /// `Option<String>` like every other field here: the wire is
    /// form-encoded, so the value arrives as text and
    /// [`reject_confirm_on_create`] decides what to do with it.
    confirm: Option<String>,
    /// The Stripe fields that decide **where or when money moves**, accepted
    /// only so they can be refused. See [`UnsupportedStripeParams`].
    #[serde(flatten)]
    unsupported: UnsupportedStripeParams,
}

/// The Stripe request fields vpay refuses on both `POST` bodies, because
/// silently ignoring them would change where or when a merchant's money
/// moves.
///
/// # Why these four and not every field vpay ignores
///
/// Neither [`CreateParams`] nor [`ConfirmParams`] has `deny_unknown_fields`,
/// and that is deliberate: a Stripe SDK adds fields of its own accord
/// (`expand`, telemetry), Stripe's own API gains fields between versions, and
/// a `400` for each of them would make vpay unusable from the SDKs this
/// compatibility work exists to support. So the default is
/// *accepted-and-ignored*, and this struct is the exception list.
///
/// The line between the two is **what a merchant believes happened**. These
/// are accepted-and-ignored, and stay that way:
///
/// * `setup_future_usage`, `confirmation_method`, `receipt_email`,
///   `statement_descriptor`, `customer` — vpay does not implement them, and a
///   merchant who sends one gets the payment they asked for anyway, taken
///   from the payer they asked, for the amount they asked;
/// * `expand`, `metadata` — `expand` is not implemented (the response simply
///   has no expanded field, which is visible in the response itself);
///   `metadata` is implemented and stored.
///
/// These four are different. Each of them says money should move somewhere
/// else, or at a different time, than it otherwise would:
///
/// * `capture_method` other than `automatic` asks vpay to *authorise now and
///   capture later*. vpay has no authorise/capture split — a confirm is the
///   charge (`docs/flows/payment-lifecycle.md`) — so ignoring it would take
///   the payer's money at a moment the merchant believes it is only being
///   held;
/// * `application_fee_amount`, `transfer_data` and `on_behalf_of` are
///   Stripe Connect: they route part or all of a payment to a *different*
///   account. vpay has no Connect at all, so ignoring them settles the whole
///   amount to the merchant who called — which is neither what was asked for
///   nor something they can see in the response.
///
/// `capture_method=automatic` is accepted, for
/// [`reject_confirm_on_create`]'s reason: it asks for exactly what this API
/// does, so refusing it would refuse a request vpay can satisfy as written.
///
/// # Not an ADR-0002 violation
///
/// These are *request fields* — the wire contract this API publishes — not
/// rails. Nothing here branches on a provider code, and adding a rail does
/// not change this list.
#[derive(Debug, Default, Deserialize)]
struct UnsupportedStripeParams {
    /// Text like every other form field. Only `automatic` is accepted.
    capture_method: Option<String>,
    application_fee_amount: Option<String>,
    /// A [`Value`] rather than a `String` because it arrives nested —
    /// `transfer_data[destination]=acct_x` decodes to an object
    /// (`crate::form`) — and this code never reads inside it. Its
    /// *presence* is the whole question.
    transfer_data: Option<Value>,
    on_behalf_of: Option<String>,
}

/// The sentence every Connect field shares. One string, because three
/// hand-written variations on "vpay has no Connect" would drift — and
/// because every public message has to fit `MESSAGE_MAX_CHARS` (200), which
/// three separately-edited sentences would each have to be checked against.
const NO_CONNECT: &str = "vpay does not support Stripe Connect: a payment cannot be split, have a fee \
                          taken from it, or be settled to an account other than the merchant that \
                          created it. Remove this field.";

/// The `capture_method` refusal, which is about *when* rather than *where*.
const NO_MANUAL_CAPTURE: &str = "vpay does not support authorising now and capturing later: confirming a PaymentIntent is \
     what charges the payer. `capture_method` accepts only `automatic`.";

impl UnsupportedStripeParams {
    /// Refuses the first field this request asked for that vpay cannot do.
    ///
    /// One table rather than four `if let`s: the rows are the specification,
    /// and a fifth field is a line here instead of another copy of the same
    /// three lines of error construction. The order is fixed, so a request
    /// carrying two of them is always refused with the same one and a
    /// merchant's second attempt makes progress rather than trading one 400
    /// for another at random.
    ///
    /// # Errors
    /// [`ApiError::InvalidParam`] (`400`, `invalid_request_error`) naming the
    /// field, because `error.param` is what a Stripe SDK points its user at.
    fn reject_unsupported(&self) -> Result<(), ApiError> {
        // (the wire name, whether this request asked for it, why vpay cannot)
        let asked: [(&str, bool, &str); 4] = [
            (
                "capture_method",
                self.capture_method
                    .as_deref()
                    .is_some_and(|method| method != "automatic"),
                NO_MANUAL_CAPTURE,
            ),
            (
                "application_fee_amount",
                self.application_fee_amount.is_some(),
                NO_CONNECT,
            ),
            ("transfer_data", self.transfer_data.is_some(), NO_CONNECT),
            ("on_behalf_of", self.on_behalf_of.is_some(), NO_CONNECT),
        ];
        for (param, present, message) in asked {
            if present {
                return Err(ApiError::invalid_param(param, message));
            }
        }
        Ok(())
    }
}

/// `POST /v1/payment_intents`.
///
/// Order: claim the idempotency key, then validate, then insert, then store
/// the response for replay.
///
/// # Why the claim comes first
///
/// Validating first reads as harmless — validation has no side effects, so
/// claiming afterwards keeps the whole concurrency guarantee — and it is
/// not, because *what is valid changes*. Every rule below is a question
/// about the deployment's current configuration: which currencies it
/// admits, which rails are enabled. An operator disabling a rail between a
/// merchant's original request and their retry would make the retry a `400`
/// **for a payment intent that already exists**, since the retry never gets
/// far enough to discover it is a replay. A replay must answer what the
/// original answered, whatever has changed since; that is the entire promise
/// of the header.
///
/// So the claim runs first, and a [`IdempotencyClaim::Replay`] short-circuits
/// before a single rule is evaluated.
///
/// # Why a validation failure releases the key instead of storing the 400
///
/// This is the one place a `4xx` is *not* stored (`PostRequest::finish`,
/// below, explains the rule it departs from). The
/// symmetric argument applies: a merchant who is told their currency is not
/// configured, gets it configured, and retries under the same key must get
/// the intent they asked for — not a 24-hour-old refusal that is no longer
/// true. Nothing has been written at that point, so re-executing is exactly
/// equivalent to the request never having been made.
pub(crate) async fn create(
    State(pool): State<PgPool>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;

    let claim_id = match post.claim_or_answer(&pool, &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    // From here the key is claimed, so every path out of this function has
    // to end it: `finish` stores or releases, and the `?`-free `match`
    // below releases before returning the caller's mistake.
    let validated = match validate_create(&post, &config).await {
        Ok(validated) => validated,
        Err(error) => {
            post.release(&pool, &scope, claim_id).await;
            return Err(error);
        }
    };
    let ValidCreate {
        amount,
        currency,
        payment_method_types,
        metadata,
        description,
    } = validated;

    let new = NewPaymentIntent {
        id: ids::payment_intent_id(),
        merchant_id: scope.merchant_id().to_owned(),
        livemode: config.livemode(),
        amount,
        currency_code: currency.code().to_owned(),
        // Never a literal: `IntentStatus::INITIAL` is where the lifecycle
        // says an intent is born (`docs/flows/payment-lifecycle.md`).
        status: IntentStatus::INITIAL.as_wire_str().to_owned(),
        last_payment_error_code: None,
        last_payment_error_message: None,
        payment_method_types: Value::Array(
            payment_method_types
                .into_iter()
                .map(Value::String)
                .collect(),
        ),
        metadata: Value::Object(metadata),
        description,
        created_at: OffsetDateTime::now_utc(),
    };

    let outcome = payment_intents::insert(&pool, &new)
        .await
        .map_err(ApiError::from)
        .and_then(|row| object_response(&row));

    post.finish(&pool, &scope, claim_id, outcome).await
}

/// A create request that has passed every rule, in the types the insert
/// needs.
///
/// A struct rather than a five-tuple because [`create`] destructures it into
/// the `NewPaymentIntent` below, and two `String`-shaped fields next to each
/// other in a tuple are one transposition away from an intent whose
/// description is its currency.
struct ValidCreate {
    amount: i64,
    currency: Currency,
    payment_method_types: Vec<String>,
    metadata: Map<String, Value>,
    description: Option<String>,
}

/// Decodes and checks a create body. Split out of [`create`] so that "the
/// key is claimed, and a failure from here must release it" is one call with
/// one error path, rather than five `?`s that would each need to remember.
async fn validate_create(
    post: &PostRequest,
    config: &ResourceConfig,
) -> Result<ValidCreate, ApiError> {
    let params: CreateParams = post.form().await?;

    reject_confirm_on_create(params.confirm.as_deref())?;
    // Before anything is written, and — like every other rule in this
    // function — while the key is claimed but nothing is stored, so
    // `create` releases the key and the merchant's corrected retry under the
    // same key executes rather than replaying the refusal.
    params.unsupported.reject_unsupported()?;

    Ok(ValidCreate {
        amount: parse_amount(params.amount.as_deref())?,
        currency: parse_currency(params.currency.as_deref(), config)?,
        payment_method_types: validated_rails(&params.payment_method_types, config)?,
        metadata: validated_metadata(&params.metadata)?,
        description: validated_description(params.description)?,
    })
}

// --------------------------------------------------------------- retrieve

/// `GET /v1/payment_intents/{id}`.
///
/// # Why this reads the charge, and only sometimes
///
/// `next_action` is not on the intent row: it is the rail's `redirect_url`
/// plus the merchant's `return_url`, both of which live on the *charge*
/// (`charges.redirect_url`, `charges.return_url` — migration 0019). An
/// intent in `requires_action` therefore costs one extra query, and every
/// other intent costs none: `requires_action` is redirect-only
/// (`docs/flows/payment-lifecycle.md`), so for any other status the answer
/// is `null` and reading the charge could only confirm that.
///
/// Rendering it from the stored row rather than remembering what `confirm`
/// answered is what makes the field *reproducible*: a merchant who lost the
/// confirm's response gets the same `next_action` here, and a merchant who
/// gets one here knows the database already held the URL before anyone was
/// sent to it (`docs/flows/crash-safety.md`, "the commit is the gate on the
/// redirect").
///
/// `last_payment_error` needs no such work — it is a column pair on the
/// intent itself (`0014`), rendered by
/// [`PaymentIntentObject::try_from`].
pub(crate) async fn retrieve(
    State(pool): State<PgPool>,
    scope: MerchantScope,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let row = payment_intents::get_for_merchant(&pool, scope.merchant_id(), &id)
        .await?
        .ok_or_else(|| not_found(&id))?;

    let object = PaymentIntentObject::try_from(&row)?;
    if object.status != IntentStatus::RequiresAction {
        return json_response(StatusCode::OK, &object);
    }

    let charge = charges::get_for_intent(&pool, &row.id).await?;
    let next_action = charge.as_ref().and_then(next_action_of);
    if next_action.is_none() {
        // `requires_action` means "the payer has somewhere to go". An intent
        // in that status whose charge carries no redirect URL is a row pair
        // that cannot both be true, and answering `null` would tell a
        // merchant their payer has nothing to do while a rail holds a live
        // payment.
        return Err(ApiError::Internal(format!(
            "payment intent {} is `{}` but its charge carries no redirect_url",
            row.id,
            IntentStatus::RequiresAction.as_wire_str(),
        )));
    }

    json_response(StatusCode::OK, &object.with_next_action(next_action))
}

/// The `next_action` a charge implies, or `None` when the rail gave the
/// payer nowhere to go.
///
/// Reads `charges`, never a `Submitted` still in memory: see [`retrieve`].
fn next_action_of(charge: &ChargeRow) -> Option<NextAction> {
    charge
        .redirect_url
        .as_ref()
        .map(|url| NextAction::RedirectToUrl {
            redirect_to_url: RedirectToUrl {
                url: url.clone(),
                // Null rather than absent when the column is: the SDKs model
                // `return_url` as an optional field of a required object, and
                // a redirect rail that was given none is a shape this API can
                // still describe honestly.
                return_url: charge.return_url.clone(),
            },
        })
}

// ------------------------------------------------------------------- list

/// `GET /v1/payment_intents`'s query parameters — text for the same reason
/// [`CreateParams`]'s fields are.
#[derive(Debug, Deserialize)]
pub(crate) struct ListParams {
    limit: Option<String>,
    starting_after: Option<String>,
    ending_before: Option<String>,
}

/// `GET /v1/payment_intents`.
pub(crate) async fn list(
    State(pool): State<PgPool>,
    scope: MerchantScope,
    VpayQuery(params): VpayQuery<ListParams>,
) -> Result<Response, ApiError> {
    let page = paging::list_page(
        params.limit.as_deref(),
        params.starting_after,
        params.ending_before,
        CURSOR,
    )?;

    let (rows, has_more) = payment_intents::list_page(&pool, scope.merchant_id(), &page).await?;
    let data = rows
        .iter()
        .map(PaymentIntentObject::try_from)
        .collect::<Result<Vec<_>, _>>()?;

    json_response(StatusCode::OK, &ListObject::new(data, has_more, LIST_URL))
}

// ----------------------------------------------------------------- cancel

/// `POST /v1/payment_intents/{id}/cancel`.
///
/// The cancel is a compare-and-swap, not a read-then-write: between reading
/// an intent and writing `canceled`, a concurrent `confirm` may already have
/// handed the charge to a rail, and cancelling *then* would tell a merchant
/// a payment was withdrawn while the payer's handset was still prompting.
///
/// `vpay_db::payment_intents::cancel`'s `UPDATE` therefore carries **two**
/// guards, and both have to be in the statement for the same reason: the
/// expected status, and `NOT EXISTS` a live charge. The second is not
/// redundant — a `confirm` commits its charge *before* calling the rail and
/// leaves the status at `requires_payment_method` until it knows what
/// happened, so "status is `requires_payment_method`" does not mean "nothing
/// is in flight". It is not a narrow window either: a confirm whose rail
/// never answered leaves exactly that state behind on purpose.
///
/// `Ok(None)` is ambiguous by construction, and [`cancel_once`]'s re-read is
/// what turns it into the right answer: 404 if there is no such intent for
/// this merchant, and one of two 409s otherwise.
pub(crate) async fn cancel(
    State(pool): State<PgPool>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let claim_id = match post.claim_or_answer(&pool, &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = cancel_once(&pool, &scope, &id).await;
    post.finish(&pool, &scope, claim_id, outcome).await
}

async fn cancel_once(pool: &PgPool, scope: &MerchantScope, id: &str) -> Result<Response, ApiError> {
    if let Some(row) = payment_intents::cancel(pool, scope.merchant_id(), id).await? {
        return object_response(&row);
    }

    // Which of the update's guards refused, told apart by one read rather
    // than by asking the database a second question about charges: the
    // statement has exactly two predicates beyond the tenancy filter, so an
    // intent that is *still* `requires_payment_method` can only have been
    // refused by the other one — the live charge. Anything else is the
    // status.
    let current = payment_intents::get_for_merchant(pool, scope.merchant_id(), id)
        .await?
        .ok_or_else(|| not_found(id))?;
    if current.status == IntentStatus::INITIAL.as_wire_str() {
        return Err(charge_in_flight());
    }
    Err(ApiError::Conflict {
        message: format!(
            "A payment intent can only be canceled while its status is \
             `{}`; this one is `{}`.",
            IntentStatus::INITIAL.as_wire_str(),
            current.status,
        ),
    })
}

/// The 409 for a cancel refused because the rail may hold the payment.
///
/// Deliberately not `already_charged()`'s sentence: "create a new payment
/// intent to try again" would be wrong advice here. The charge may still
/// succeed, and a merchant who acts on "this one is finished" by opening a
/// second intent can end up having taken the money twice. What they have to
/// do is wait, so that is what it says.
fn charge_in_flight() -> ApiError {
    ApiError::Conflict {
        message: "This payment intent has a charge in flight and cannot be canceled; wait for \
                  it to reach a terminal state."
            .to_owned(),
    }
}

// ---------------------------------------------------------------- confirm

/// `POST /v1/payment_intents/{id}/confirm`'s fields.
///
/// `payment_method_data` is an untyped map on purpose. The instrument is
/// nested under the *rail's own code* on the wire
/// (`payment_method_data[mtn_momo][msisdn]`), so a typed struct here would
/// have to name `mtn_momo` as a field — and `if provider == "mtn_momo"`
/// outside an adapter crate is exactly what ADR-0002 forbids. Read this way,
/// the code comes from the request and this file never learns a rail's name.
#[derive(Debug, Deserialize)]
struct ConfirmParams {
    payment_method_data: Option<Map<String, Value>>,
    return_url: Option<String>,
    /// The same refusal set `POST /v1/payment_intents` carries. stripe-node's
    /// `paymentIntents.confirm` takes `capture_method` and `application_fee_amount`
    /// too, and a merchant who was refused on create would otherwise get them
    /// silently ignored one request later.
    #[serde(flatten)]
    unsupported: UnsupportedStripeParams,
}

/// `POST /v1/payment_intents/{id}/confirm`.
///
/// The six steps are the order `docs/flows/crash-safety.md` requires, and
/// their order is the whole safety property:
///
/// 1. load the intent for this merchant (404), refuse a status that forbids
///    a confirm (409), refuse an intent that already has a charge (409, and
///    **before** any insert — "one charge per intent, forever");
/// 2. resolve the rail from `payment_method_data[type]` and branch on its
///    [`vpay_provider::Capabilities::flow`] only, never on its code;
/// 3. mint the `provider_reference_id` and commit the charge row in
///    `submitting`, so the reference is durable before anything is sent;
/// 4. record the attempt in `provider_requests` with no status;
/// 5. call the adapter — `submit` is `async`, so the `.await` is what
///    actually sends the request;
/// 6. record what came back and answer.
///
/// Step 6 has three shapes, and which one runs is decided by the *error's*
/// own classification rather than by anything this file knows about rails:
///
/// * **the rail accepted it** — one transaction moves the charge to
///   `submitted` with the rail's key material and the intent to
///   `processing`/`requires_action`, it commits, and only then is a response
///   built (`persist_submitted`, and `docs/flows/crash-safety.md`'s "the
///   commit is the gate on the redirect");
/// * **the rail declined it** (`ProviderError::Rejected`) — one transaction
///   fails the charge with its `failure_code` and stamps
///   `last_payment_error` on the intent, which stays
///   `requires_payment_method` because the lifecycle has no `failed` status;
///   the merchant gets the `409` `charge_declined`;
/// * **anything else** — we do not know what the rail did, so *nothing*
///   moves. The `submitting` charge row and the status-less
///   `provider_requests` row stay behind on purpose: they are exactly the
///   state a crash between steps 4 and 6 would leave, and are what the
///   recovery pass will read (Step 4 — `docs/status.md`).
pub(crate) async fn confirm(
    State(pool): State<PgPool>,
    State(config): State<Arc<ResourceConfig>>,
    State(adapters): State<Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let params: ConfirmParams = post.form().await?;
    // Before the claim, where the body is decoded — so a refused confirm
    // stores nothing and leaves the key unspent, exactly as a body that fails
    // to decode already does. The corrected retry under the same key is a
    // fresh request, not a replay of the refusal.
    //
    // Running it first is safe because the check is config-independent — it
    // reads the body alone — so a genuine replay, whose body is byte for byte
    // the one that was accepted, can never be shadowed by it. What the
    // ordering is visible in is the other case: a confirm reusing a completed
    // key with a newly added refused field answers that field's `400` rather
    // than [`ApiError::idempotency_key_reused`] (`idempotency_key_in_use` on
    // the wire), because the field is refused before the key is looked at.
    params.unsupported.reject_unsupported()?;

    let claim_id = match post.claim_or_answer(&pool, &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = confirm_once(&pool, &config, &adapters, &scope, &id, params).await;
    post.finish(&pool, &scope, claim_id, outcome).await
}

/// The `payment_method_data[type]` key, and the `param` a caller sees when
/// something about the instrument is wrong.
const PMD_TYPE_PARAM: &str = "payment_method_data[type]";

async fn confirm_once(
    pool: &PgPool,
    config: &ResourceConfig,
    adapters: &BTreeMap<String, Box<dyn ProviderAdapter>>,
    scope: &MerchantScope,
    id: &str,
    params: ConfirmParams,
) -> Result<Response, ApiError> {
    // --- step 1: the intent, this merchant's, in a status that allows it
    let intent = payment_intents::get_for_merchant(pool, scope.merchant_id(), id)
        .await?
        .ok_or_else(|| not_found(id))?;
    let status = IntentStatus::from_wire(&intent.status).ok_or_else(|| {
        ApiError::Internal(format!(
            "payment_intents.status holds `{}`, which is not an IntentStatus",
            intent.status
        ))
    })?;
    // The lifecycle rule comes from `vpay_core::state`, never from a literal
    // here (`docs/flows/payment-lifecycle.md`). The rail is deliberately not
    // resolved yet, so this 409 cannot depend on which rail was asked for —
    // and it does not need to: whether a confirm is legal is a property of
    // the *status* alone, since `next_status` routes both flows through the
    // same arm. `confirm_legality_does_not_depend_on_the_rails_flow` below
    // is what keeps that true.
    if next_status(status, Transition::Confirm(ProviderFlow::Push)).is_none() {
        return Err(ApiError::Conflict {
            message: format!(
                "A payment intent can only be confirmed while its status is `{}`; \
                 this one is `{}`.",
                IntentStatus::RequiresPaymentMethod.as_wire_str(),
                status.as_wire_str(),
            ),
        });
    }
    // Checked before anything is inserted, so a second confirm cannot even
    // attempt a second charge. The unique index `one_charge_per_intent` is
    // what actually enforces it — this check is what turns the race's loser
    // into a 409 instead of a 500.
    if let Some(existing) = charges::get_for_intent(pool, &intent.id).await? {
        return Err(already_charged(&intent.id, Some(&existing)));
    }

    // --- step 2: the rail, resolved from the request and branched on by flow
    let data = params
        .payment_method_data
        .as_ref()
        .ok_or_else(missing_payment_method_data)?;
    let code = data
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(missing_payment_method_data)?;

    // The deployment first: a rail this deployment does not offer is a
    // caller's mistake (400), while a rail it offers with no adapter linked
    // is ours (500, through `ConfigError`) — two different answers that a
    // single lookup would collapse into one.
    let rail = config.enabled_rail(code).ok_or_else(|| {
        ApiError::invalid_param(
            PMD_TYPE_PARAM,
            "That payment method type is not available on this deployment.",
        )
    })?;
    let adapter = adapters.get(code).ok_or_else(|| {
        ApiError::Config(vpay_config::ConfigError::ProviderWithoutAdapter {
            code: code.to_owned(),
            linked: adapters.keys().cloned().collect::<Vec<_>>().join(", "),
        })
    })?;
    // The rails an intent was created for are the rails it may be confirmed
    // against. Without this, an intent created for one rail could be charged
    // on another the merchant never offered its payer.
    if !intent_allows(&intent.payment_method_types, code) {
        return Err(ApiError::invalid_param(
            PMD_TYPE_PARAM,
            "That payment method type is not one this payment intent was created with.",
        ));
    }

    // The rail's own settlement currency against the intent's. Checked
    // *before* anything is written, so a mismatch costs no charge row: see
    // [`refuse_a_currency_the_rail_does_not_settle`].
    currencies_agree(rail, &intent.currency_code)?;

    let flow = adapter.capabilities().flow;
    // The *only* branch on the rail in this file, and it is on the flow
    // shape, never on the code (ADR-0002).
    // The redirect rail's `return_url` is carried into the charge row and
    // committed *before* the rail is called (`charges.return_url`, migration
    // 0019), because `next_action.redirect_to_url.return_url` has to be
    // reproducible on every later read of the intent — see [`retrieve`].
    let (payer_ref, return_url) = match flow {
        ProviderFlow::Push => {
            let msisdn = data
                .get(code)
                .and_then(Value::as_object)
                .and_then(|instrument| instrument.get("msisdn"))
                .and_then(Value::as_str)
                .filter(|msisdn| !msisdn.trim().is_empty())
                .ok_or_else(|| {
                    ApiError::invalid_param(
                        "payment_method_data",
                        "This payment method needs the payer's number, sent as \
                         `payment_method_data[<type>][msisdn]`.",
                    )
                })?;
            (Some(msisdn.to_owned()), None)
        }
        ProviderFlow::Redirect => {
            let return_url = params
                .return_url
                .as_deref()
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .ok_or_else(|| {
                    ApiError::invalid_param(
                        "return_url",
                        "This payment method redirects the payer, so a `return_url` is \
                         required.",
                    )
                })?;
            checked_return_url(return_url)?;
            (None, Some(return_url.to_owned()))
        }
    };
    let currency = Currency::from_code(&intent.currency_code)?;
    let amount = Money::new(intent.amount, currency)?;

    // --- step 3: the reference, durable before anything is submitted
    let reference = Uuid::new_v4();
    let charge = insert_charge(
        pool,
        &NewCharge {
            id: ids::charge_id(),
            payment_intent_id: intent.id.clone(),
            provider_code: code.to_owned(),
            provider_reference_id: reference,
            provider_ref_extra: None,
            // Nothing a rail said, because nothing has been asked yet. The
            // merchant's own `return_url` *is* known and is written now.
            redirect_url: None,
            return_url: return_url.clone(),
            state: vpay_core::ChargeState::INITIAL.as_wire_str().to_owned(),
            amount: intent.amount,
            // The intent's currency, verbatim: no conversion (Step 2's D2).
            // `currencies_agree` above has already refused the case where the
            // rail settles in a different one.
            currency_code: intent.currency_code.clone(),
            payer_ref: payer_ref.clone(),
            payer_ref_masked: None,
        },
    )
    .await?;

    // --- step 4: the attempt, recorded before the call
    let attempt =
        provider_requests::insert_pending(pool, &charge.id, code, "submit", reference, 1).await?;

    // --- step 5: the rail. `submit` is `async` as of Step 3 (the port's
    // methods return boxed futures via `#[async_trait]`), so the `.await` is
    // load-bearing: without it this binds a future and step 6 below would be
    // matching on a `Pin<Box<..>>` that never ran — a "submit" that never
    // reached the rail while the charge row already said `submitting`.
    let charge_ref = ChargeRef {
        reference_id: reference,
        amount,
        payer_ref,
        ref_extra: BTreeMap::new(),
    };
    let submitted = adapter.submit(&charge_ref, &rail.provider_config()).await;

    // --- step 6: what came back
    match submitted {
        Ok(submitted) => {
            // The rail answered, so the attempt is answered — before any
            // state moves, and with a status that does not pretend to be an
            // HTTP one (`STATUS_CODE_NOT_CARRIED_BY_THE_PORT`).
            provider_requests::record_response(
                pool,
                attempt,
                Some(provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT),
                None,
            )
            .await?;
            let (intent, charge) =
                persist_submitted(pool, scope, &intent, &charge, flow, &submitted).await?;
            submitted_response(&intent, &charge)
        }
        Err(error) => {
            match &error {
                // A rail *decision*, not a rail failure (`docs/flows/errors.md`).
                // The charge is terminal, and the intent goes back to the
                // status it never left, now carrying why.
                ProviderError::Rejected { code, message } => {
                    provider_requests::record_response(
                        pool,
                        attempt,
                        Some(provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT),
                        Some(error_kind(&error)),
                    )
                    .await?;
                    persist_decline(
                        pool,
                        scope,
                        &intent.id,
                        &charge.id,
                        *code,
                        message,
                        &vpay_core::Classify::public_message(&error),
                    )
                    .await?;
                }
                // Everything else means we do not know what the rail did
                // with the request, so nothing moves: the charge stays
                // `submitting`, which is the recovery state, and the
                // attempt keeps `status_code IS NULL`, which is how a
                // recovery pass tells "no answer" from "answered"
                // (`docs/flows/crash-safety.md`'s table).
                //
                // `Malformed` is the one arm where "no answer" is not
                // literally true — bytes came back, they just did not parse
                // — and it is grouped here deliberately. What the table
                // decides is whether to go and *ask the rail*, and an
                // unparseable answer is exactly as unknown as a lost one:
                // "every ambiguity resolves toward 'find out', never 'give
                // up'".
                ProviderError::Transport(_)
                | ProviderError::Malformed(_)
                | ProviderError::Config(_)
                | ProviderError::Unsupported
                | ProviderError::NotImplemented(_) => {
                    provider_requests::record_response(
                        pool,
                        attempt,
                        None,
                        Some(error_kind(&error)),
                    )
                    .await?;
                }
            }
            Err(ApiError::from(error))
        }
    }
}

/// Commits everything a rail's acceptance implies, in **one** transaction,
/// and hands back the two rows as they now stand.
///
/// # Why one transaction, and why the response is built from its result
///
/// `docs/flows/crash-safety.md`: "the commit is the gate on the redirect".
/// A merchant may be handed a `next_action.redirect_to_url` only once the
/// database already holds the URL and the rail's key material — otherwise a
/// crash strands a payer on the rail's page against a charge vpay cannot
/// query. Two statements outside a transaction would also let a reader see
/// an intent in `requires_action` whose charge has no URL yet.
///
/// So both writes are here, the caller returns *this function's* rows rather
/// than the values it sent, and [`submitted_response`] reads the charge row
/// — which means a response carrying a URL is proof the committed row has
/// one. Deleting the charge update does not produce a response without a
/// `next_action`; it produces a `500`.
///
/// # Errors
///
/// [`vpay_db::DbError::WriteMatchedNoRow`] (a `500`) if either
/// compare-and-swap matches nothing: someone else advanced the charge out of
/// `submitting`, or moved the intent out of `requires_payment_method`. Both
/// are refusals of a guard, not of the merchant, and both roll the whole
/// transaction back — leaving the `submitting` charge and the answered
/// attempt for a recovery pass, which is the honest state.
async fn persist_submitted(
    pool: &PgPool,
    scope: &MerchantScope,
    intent: &vpay_db::PaymentIntentRow,
    charge: &ChargeRow,
    flow: ProviderFlow,
    submitted: &Submitted,
) -> Result<(vpay_db::PaymentIntentRow, ChargeRow), ApiError> {
    // The state machine answers, never a literal here
    // (`docs/flows/payment-lifecycle.md`): push → `processing`, redirect →
    // `requires_action`.
    let next = next_status(
        IntentStatus::RequiresPaymentMethod,
        Transition::Confirm(flow),
    )
    .ok_or_else(|| {
        ApiError::Internal(format!(
            "the lifecycle refuses a confirm from `{}`, which step 1 had already allowed",
            IntentStatus::RequiresPaymentMethod.as_wire_str(),
        ))
    })?;

    let mut tx = pool.begin().await.map_err(vpay_db::DbError::Query)?;

    // Written even when it is empty — a push rail returns no key material,
    // and `{}` says "the rail answered and there was none" where NULL would
    // be indistinguishable from a charge that was never submitted.
    let ref_extra = Value::Object(
        submitted
            .ref_extra
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect(),
    );
    let charge = charges::mark_submitted(
        &mut tx,
        &charge.id,
        vpay_core::ChargeState::Submitted.as_wire_str(),
        Some(&ref_extra),
        submitted.redirect_url.as_deref(),
    )
    .await?;

    let intent = payment_intents::transition_in_tx(
        &mut tx,
        scope.merchant_id(),
        &intent.id,
        IntentStatus::RequiresPaymentMethod.as_wire_str(),
        next.as_wire_str(),
    )
    .await?
    .ok_or_else(|| {
        // Unreachable while `cancel` refuses an intent with a live charge
        // (`vpay_db::payment_intents::cancel`'s `NOT EXISTS`), and loud
        // rather than quiet because if it ever fires, a rail is holding a
        // payment for an intent that says it is not being paid.
        ApiError::Internal(format!(
            "the rail accepted charge {} but payment intent {} was no longer `{}`; \
             the rail may hold a live payment",
            charge.id,
            intent.id,
            IntentStatus::RequiresPaymentMethod.as_wire_str(),
        ))
    })?;

    tx.commit().await.map_err(vpay_db::DbError::Query)?;
    Ok((intent, charge))
}

/// The `200` for a confirm the rail accepted, built from the **committed**
/// rows.
///
/// A redirect rail whose committed charge carries no URL is a `500`, not a
/// `200` with `next_action: null`: the intent is in `requires_action`, so
/// telling a merchant there is no action would be a fabricated success in
/// the most literal sense — the payer would never be sent anywhere, and the
/// rail would time the charge out hours later.
fn submitted_response(
    intent: &vpay_db::PaymentIntentRow,
    charge: &ChargeRow,
) -> Result<Response, ApiError> {
    let object = PaymentIntentObject::try_from(intent)?;
    if object.status != IntentStatus::RequiresAction {
        return json_response(StatusCode::OK, &object);
    }
    let next_action = next_action_of(charge).ok_or_else(|| {
        ApiError::Internal(format!(
            "the rail accepted charge {} on a redirect flow and no redirect_url was committed; \
             the payer cannot be sent anywhere",
            charge.id,
        ))
    })?;
    json_response(StatusCode::OK, &object.with_next_action(Some(next_action)))
}

/// Commits what a decline at submit implies, in one transaction: the charge
/// is terminal with its `failure_code`, and the intent carries
/// `last_payment_error` while keeping the status it never left.
///
/// # What each half stores, and why they are different strings
///
/// `charges.failure_raw` gets the **rail's own words** — kept because
/// `docs/flows/failures.md` requires an unmapped reason to survive for
/// whoever fixes the mapping table. `last_payment_error.message` is rendered
/// to the *merchant*, so it gets the error's `public_message()`, the same
/// sentence the `409` envelope carries: "the rail's raw reason string is not
/// [public] — it is logged via Display, never sent" (`docs/flows/errors.md`).
/// One rail message must not reach a merchant through a side door.
///
/// # Errors
///
/// As [`persist_submitted`]. `Ok(None)` from the intent write is *not* an
/// error here: the charge is failed either way, and an intent that moved on
/// simply does not get the error stamped onto it — see the body.
async fn persist_decline(
    pool: &PgPool,
    scope: &MerchantScope,
    intent_id: &str,
    charge_id: &str,
    code: vpay_core::FailureCode,
    rail_message: &str,
    public_message: &str,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(vpay_db::DbError::Query)?;

    charges::mark_failed(
        &mut tx,
        charge_id,
        code.as_str(),
        &bounded(rail_message, FAILURE_RAW_MAX_CHARS),
    )
    .await?;

    let updated = payment_intents::record_payment_error(
        &mut tx,
        scope.merchant_id(),
        intent_id,
        IntentStatus::RequiresPaymentMethod.as_wire_str(),
        code.as_str(),
        &bounded(public_message, LAST_PAYMENT_ERROR_MAX_CHARS),
    )
    .await?;
    if updated.is_none() {
        // The intent moved while the rail was deciding. The charge is still
        // failed — that write is about the rail's answer and is true
        // whatever the intent says — and the missing half is logged rather
        // than turned into a `500`, because the merchant's answer is the
        // decline, which is accurate.
        tracing::warn!(
            merchant_id = %scope.merchant_id(),
            payment_intent_id = %intent_id,
            "a rail declined a charge whose intent was no longer requires_payment_method; \
             last_payment_error was not recorded"
        );
    }

    tx.commit().await.map_err(vpay_db::DbError::Query)?;
    Ok(())
}

/// The `failure_raw_length` CHECK on `charges` (2000) and the
/// `lpe_message_length` CHECK on `payment_intents` (512), mirrored here so a
/// long rail message is truncated rather than answered as a constraint
/// violation rendered as a `500`.
const FAILURE_RAW_MAX_CHARS: usize = 2000;
const LAST_PAYMENT_ERROR_MAX_CHARS: usize = 512;

/// Truncates on a `char` boundary. Both database CHECKs count
/// `char_length`, i.e. characters, so this counts characters too — a byte
/// slice would both mis-measure and be a panic waiting for a rail that
/// answers in Arabic.
fn bounded(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// Refuses a confirm whose intent is denominated in a currency the chosen
/// rail does not settle in.
///
/// # Why this is the API's refusal and not the adapter's
///
/// `ProviderConfig.currency` is the rail's *profile* currency, from the
/// deployment's YAML — MTN's sandbox accepts EUR and rejects XAF
/// (`docs/flows/money.md`), which is a configuration fact and must never be
/// a code branch (ADR-0003). The charge, meanwhile, carries the intent's
/// currency verbatim (Step 2's D2). Nothing reconciled the two, so a confirm
/// used to submit an XAF amount under a EUR profile and let the rail decide
/// what that meant — which, for a rail that simply believes the number, is
/// a payer charged 5,000 of the wrong unit.
///
/// It is a `400` on `payment_method_data[type]` rather than on `currency`
/// because the currency is fixed at creation and cannot be changed: what the
/// caller can still choose, and what is actually wrong, is the rail. The
/// message names both so the merchant does not have to go and read the
/// deployment's configuration to find out which one it settles in.
fn currencies_agree(rail: &RailConfig, intent_currency: &str) -> Result<(), ApiError> {
    let rail_currency = rail.currency();
    if rail_currency.code() == intent_currency {
        return Ok(());
    }
    Err(ApiError::invalid_param(
        PMD_TYPE_PARAM,
        format!(
            "rail `{}` settles in {}; this PaymentIntent is {}. Confirm it on a rail \
             configured for {}, or create a new PaymentIntent in {}.",
            rail.code(),
            rail_currency.code(),
            intent_currency,
            intent_currency,
            rail_currency.code(),
        ),
    ))
}

/// The `jobs.kind` and `jobs.dedupe_key` this handler enqueues under.
///
/// Written out as literals rather than imported from `vpay_worker::jobs`,
/// which is where they are otherwise spelled. Not a preference: `vpay-worker`
/// depends on *this* crate (for `crate::model::PaymentIntentObject`, which it
/// renders `events.data` through), so importing it back would be a cycle.
///
/// Three things stop that duplication drifting silently, which is why it is
/// acceptable at all:
///
/// * migration 0021's `kind_is_known` CHECK refuses any `kind` outside the
///   four the worker knows, so a typo here fails the *confirm* rather than
///   producing a job nothing dispatches;
/// * `the_enqueued_job_matches_the_workers_own_spelling` below transcribes
///   both values from `vpay_worker::jobs` and fails if either moves;
/// * `worker_e2e.rs` drives a real confirm through a real worker, so a
///   dedupe key the worker cannot recognise means the payment never settles.
const POLL_CHARGE_KIND: &str = "poll_charge";

/// Commits the charge row in `submitting`, **and the job that will poll it**,
/// in one transaction, before any network call.
///
/// # Why the enqueue is here and not after the rail answers
///
/// `docs/flows/crash-safety.md` names three points at which this process can
/// die between a merchant's confirm and a settled charge. Enqueueing at step
/// 6 — beside `persist_submitted`, once the rail has answered — would leave
/// the first two of them, which are precisely the recovery cases, with a
/// committed charge and no job: a payer whose handset may be prompting, and
/// nothing anywhere that will ever ask the rail what happened. Enqueueing in
/// *this* transaction makes the job and the charge one atomic fact, so all
/// three kill points leave work behind and no scan is load-bearing for
/// recovery.
///
/// That is also why the transaction is not redundant for what looks like a
/// single insert: `vpay_db::charges::insert_for_intent` and
/// `vpay_db::jobs::enqueue_in_tx` both take a `PgConnection` precisely so the
/// commit point is the caller's, and this one has to be *before* the adapter
/// call rather than pooled into some later unit of work.
///
/// The enqueue's `ON CONFLICT (dedupe_key) DO NOTHING` cannot fire here — the
/// charge id was generated moments ago — but a `false` return is not treated
/// as an error anywhere, because the same key is enqueued by the worker's
/// backstop scan and by `resubmit_charge`.
async fn insert_charge(pool: &PgPool, new: &NewCharge) -> Result<ChargeRow, ApiError> {
    let mut tx = pool.begin().await.map_err(vpay_db::DbError::Query)?;
    let charge = match charges::insert_for_intent(&mut tx, new).await {
        Ok(charge) => charge,
        // The unique index is the enforcement; this arm is the race the
        // read in step 1 cannot close. Same 409 either way, so a merchant
        // cannot tell a race from a sequential second confirm — and does not
        // need to.
        Err(vpay_db::DbError::UniqueViolation { constraint, .. })
            if constraint == "one_charge_per_intent" =>
        {
            // Re-read so the 409 can say which of the two sentences applies
            // — and a read that fails must not turn the merchant's 409 into
            // a 503, so its own error is dropped in favour of `None`, which
            // `already_charged` treats as "assume the rail may hold it".
            // One extra query on a path that is already a lost race — taken
            // on a *second* pool connection, so the aborted transaction is
            // released first rather than held while this read waits for a
            // connection under a saturated pool (review finding, 2026-09-03).
            drop(tx);
            let existing = charges::get_for_intent(pool, &new.payment_intent_id)
                .await
                .ok()
                .flatten();
            return Err(already_charged(&new.payment_intent_id, existing.as_ref()));
        }
        Err(error) => return Err(error.into()),
    };

    // The payload is the minimal one `vpay_worker::jobs::PollChargePayload`
    // defaults the ladder fields from: the confirm path has no `NotFound`
    // streak to carry, and writing zeroes for one would be this handler
    // asserting something about a rail it has not yet called.
    vpay_db::jobs::enqueue_in_tx(
        &mut tx,
        POLL_CHARGE_KIND,
        &poll_dedupe_key(&charge.id),
        &serde_json::json!({ "charge_id": charge.id }),
        OffsetDateTime::now_utc(),
    )
    .await?;

    tx.commit().await.map_err(vpay_db::DbError::Query)?;
    Ok(charge)
}

/// The `jobs.dedupe_key` for polling one charge — `vpay_worker::jobs`'
/// `poll_dedupe_key`, duplicated for the reason [`POLL_CHARGE_KIND`] gives.
///
/// The unique index on `dedupe_key` is what makes "one live poll job per
/// charge, forever" a property of the schema, so this string is load-bearing
/// in a way the `kind` is not: a *different* key here would not fail any
/// constraint, it would quietly produce a second job for the same charge that
/// the worker's own enqueues would never deduplicate against.
fn poll_dedupe_key(charge_id: &str) -> String {
    format!("poll:{charge_id}")
}

/// The 409 both the read and the unique-index race produce — and which of
/// its two sentences the merchant is given.
///
/// # Why "create a new payment intent" is not always safe advice
///
/// It was the only sentence until the Step 3 security review, and on a
/// charge that is still **live** it is the double-charge instruction
/// `docs/flows/crash-safety.md` exists to prevent. The case is not exotic;
/// it is the ordinary one. A confirm whose `submit` timed out answers `502`,
/// the idempotency key is released, the merchant retries, and the retry
/// meets the `submitting` charge the first attempt left behind — a charge
/// the rail may well be holding, because "the response was lost" is not
/// "the request was not received" (that is the whole of crash-safety.md's
/// recovery table). Telling that merchant to open a second PaymentIntent is
/// telling them to prompt the payer's handset a second time for the same
/// money.
///
/// So the advice follows the charge's state, from
/// [`vpay_core::ChargeState::is_live`] — the same predicate the reconciler
/// polls on, rather than a second list here that could drift from it:
///
/// * live (`submitting`/`submitted`/`pending`/`unresolved`) → wait and
///   poll. The charge is going to resolve one way or the other, and the
///   `GET` is where the merchant will see it.
/// * terminal (`succeeded`/`failed`) → the original sentence. One charge
///   per intent, forever; a retry is a new intent, and there is nothing in
///   flight for it to duplicate.
///
/// `existing` is `None` when the caller could not read the row (the
/// unique-violation race, whose re-read is best-effort). That is treated as
/// live, because the unsafe direction of this decision is only one way:
/// telling a merchant to wait for a charge that has already settled costs
/// them one `GET`, and telling them to retry a charge the rail is holding
/// costs a payer twice.
fn already_charged(intent_id: &str, existing: Option<&ChargeRow>) -> ApiError {
    let live = existing.is_none_or(|charge| {
        vpay_core::ChargeState::from_wire(&charge.state).is_none_or(vpay_core::ChargeState::is_live)
    });

    if live {
        return ApiError::Conflict {
            message: format!(
                "A charge for this PaymentIntent is being resolved with the rail; poll \
                 GET /v1/payment_intents/{intent_id} — do not create a new PaymentIntent."
            ),
        };
    }
    ApiError::Conflict {
        message: "This payment intent already has a charge. One charge per intent, forever \
                  — create a new payment intent to try again."
            .to_owned(),
    }
}

/// The ceiling `charges.return_url` is constrained to (migration `0019`).
///
/// The same 2048 characters, deliberately: this check exists so that the
/// column's CHECK is a backstop rather than the guard. Trip the CHECK and a
/// merchant's over-long URL comes back as a `503` telling them vpay is
/// broken; trip this and it comes back as a `400` naming `return_url`,
/// which is the truth.
const RETURN_URL_MAX_CHARS: usize = 2_048;

/// The schemes a payer's browser may be sent back to.
///
/// A closed list, not a denylist of dangerous schemes: `javascript:` is the
/// obvious one, `data:` and `vbscript:` are the ones a denylist forgets, and
/// the set of things that legitimately belong here is exactly two. `http`
/// alongside `https` because a merchant's local development host is
/// plain-HTTP and refusing it would push people to a worse workaround; the
/// livemode https-only rule belongs to `vpay_config`'s `validate_host`,
/// which is deployment-wide.
const RETURN_URL_SCHEMES: [&str; 2] = ["http://", "https://"];

/// Refuses a `return_url` that would be a `503` from a CHECK, or a redirect
/// a browser would *execute* rather than navigate to.
///
/// Checked before any insert, so a bad value costs no charge row — the same
/// reason `currencies_agree` runs where it does. Both rules mirror
/// `charges`' constraints exactly (migration `0019`); "the API bounds it at
/// the boundary too" is a claim that file's comment makes, and this is what
/// makes it true.
///
/// # Errors
///
/// [`ApiError::invalid_param`] on `return_url` for a scheme that is not
/// `http`/`https` — compared case-insensitively, as URL schemes are — or a
/// value over [`RETURN_URL_MAX_CHARS`].
fn checked_return_url(url: &str) -> Result<(), ApiError> {
    let lowercase = url.to_lowercase();
    if !RETURN_URL_SCHEMES
        .iter()
        .any(|scheme| lowercase.starts_with(scheme))
    {
        return Err(ApiError::invalid_param(
            "return_url",
            "`return_url` must be an `http://` or `https://` URL — it is where the payer's \
             browser is sent after the rail's page.",
        ));
    }
    // Characters, not bytes: the column's CHECK is `char_length`, and
    // counting bytes here would refuse a legal URL whose query string is
    // not ASCII.
    if url.chars().count() > RETURN_URL_MAX_CHARS {
        return Err(ApiError::invalid_param(
            "return_url",
            format!("`return_url` must be at most {RETURN_URL_MAX_CHARS} characters."),
        ));
    }
    Ok(())
}

fn missing_payment_method_data() -> ApiError {
    ApiError::invalid_param(
        PMD_TYPE_PARAM,
        "A confirm needs the payment method to use, sent as `payment_method_data[type]`.",
    )
}

/// Whether the intent was created naming this rail.
fn intent_allows(payment_method_types: &Value, code: &str) -> bool {
    payment_method_types
        .as_array()
        .is_some_and(|types| types.iter().any(|value| value.as_str() == Some(code)))
}

/// The `provider_requests.error_kind` for a failed attempt.
///
/// [`vpay_core::Classify::code`] rather than a match on `ProviderError`'s
/// variants: the error's own classification already names it
/// (`not_implemented`, `provider_unavailable`, …), and a second vocabulary
/// here would drift from the one a merchant sees in the envelope.
fn error_kind(error: &ProviderError) -> &'static str {
    vpay_core::Classify::code(error)
}

// ------------------------------------------------------------- validation

fn parse_amount(raw: Option<&str>) -> Result<i64, ApiError> {
    let raw = raw.ok_or_else(|| {
        ApiError::invalid_param(
            "amount",
            "An `amount` in the currency's minor units is required.",
        )
    })?;
    let amount: i64 = raw.trim().parse().map_err(|_error| {
        ApiError::invalid_param(
            "amount",
            "`amount` must be a whole number of the currency's minor units.",
        )
    })?;
    if amount <= 0 {
        return Err(ApiError::invalid_param(
            "amount",
            "`amount` must be greater than zero.",
        ));
    }
    if amount > MAX_AMOUNT {
        return Err(ApiError::invalid_param(
            "amount",
            "`amount` is larger than this API accepts (2^53-1 minor units).",
        ));
    }
    Ok(amount)
}

/// Both currency gates: a code the system knows, *and* one this deployment
/// configured. See [`ResourceConfig::admits_currency`].
fn parse_currency(raw: Option<&str>, config: &ResourceConfig) -> Result<Currency, ApiError> {
    let raw = raw.ok_or_else(|| {
        ApiError::invalid_param("currency", "A three-letter `currency` code is required.")
    })?;
    let upper = raw.trim().to_ascii_uppercase();
    let currency = Currency::from_code(&upper).map_err(|_error| {
        ApiError::invalid_param("currency", "That is not a currency vpay supports.")
    })?;
    if !config.admits_currency(currency.code()) {
        return Err(ApiError::invalid_param(
            "currency",
            "That currency is not configured for this deployment.",
        ));
    }
    Ok(currency)
}

/// Refuses `confirm` on create, so a Stripe snippet that believes it
/// charged someone is told otherwise.
///
/// **`confirm=false` is accepted, and `confirm=true` is not.** They ask for
/// different things: `false` asks vpay to create the intent without
/// confirming it, which is exactly and only what this endpoint does, so
/// honouring it is not "ignoring" anything and refusing it would reject a
/// request vpay can satisfy as written. `true` asks for a charge this
/// endpoint will never make, and there is no answer to that but a refusal —
/// which is the whole point of the field's existence here.
///
/// Anything that is neither takes the same branch as `true`, deliberately:
/// vpay cannot know what a merchant meant by `confirm=yes`, and the safe
/// reading of an unparseable confirmation flag is the one that does not
/// leave them thinking a payment happened. The message is written to be true
/// in both cases.
///
/// `param: "confirm"` because that is the field a Stripe SDK will point its
/// user at, and the sentence names the endpoint that actually does the work
/// rather than merely saying "unsupported".
fn reject_confirm_on_create(confirm: Option<&str>) -> Result<(), ApiError> {
    match confirm {
        None | Some("false") => Ok(()),
        Some(_) => Err(ApiError::invalid_param(
            "confirm",
            "`confirm` is not supported when creating a PaymentIntent. Create the intent first, \
             then confirm it with POST /v1/payment_intents/{id}/confirm, which is where the \
             payment method is supplied.",
        )),
    }
}

/// Every named rail must be one this deployment offers *now*: a
/// `payment_method_types` naming a disabled or unknown rail would produce an
/// intent that can never be confirmed.
fn validated_rails(requested: &[String], config: &ResourceConfig) -> Result<Vec<String>, ApiError> {
    if requested.is_empty() {
        return Err(ApiError::invalid_param(
            "payment_method_types",
            "At least one `payment_method_types[]` entry is required; an intent with none \
             could never be confirmed.",
        ));
    }
    for code in requested {
        if config.enabled_rail(code).is_none() {
            return Err(ApiError::invalid_param(
                "payment_method_types",
                "One of the payment method types is not available on this deployment.",
            ));
        }
    }
    Ok(requested.to_vec())
}

fn validated_metadata(metadata: &BTreeMap<String, String>) -> Result<Map<String, Value>, ApiError> {
    if metadata.len() > METADATA_MAX_KEYS {
        return Err(ApiError::invalid_param(
            "metadata",
            "`metadata` accepts at most 50 keys.",
        ));
    }
    for (key, value) in metadata {
        if key.chars().count() > METADATA_MAX_KEY_CHARS {
            return Err(ApiError::invalid_param(
                "metadata",
                "A `metadata` key is longer than 40 characters.",
            ));
        }
        if value.chars().count() > METADATA_MAX_VALUE_CHARS {
            return Err(ApiError::invalid_param(
                "metadata",
                "A `metadata` value is longer than 500 characters.",
            ));
        }
    }
    Ok(crate::model::metadata_from_pairs(metadata))
}

fn validated_description(description: Option<String>) -> Result<Option<String>, ApiError> {
    match description {
        Some(description) if description.chars().count() > DESCRIPTION_MAX_CHARS => {
            Err(ApiError::invalid_param(
                "description",
                "`description` is longer than 1000 characters.",
            ))
        }
        other => Ok(other),
    }
}

// --------------------------------------------------------------- plumbing

fn not_found(id: &str) -> ApiError {
    ApiError::NotFound {
        resource: RESOURCE,
        id: id.to_owned(),
    }
}

/// Renders a stored row as its wire object, `200 OK`.
fn object_response(row: &vpay_db::PaymentIntentRow) -> Result<Response, ApiError> {
    json_response(StatusCode::OK, &PaymentIntentObject::try_from(row)?)
}

/// The one JSON renderer for a *successful* `/v1` response.
///
/// Goes through `serde_json::Value` rather than serialising the object
/// straight to bytes, and that is what makes an idempotent replay
/// byte-for-byte identical to the original answer.
///
/// The mechanism is `serde_json::Map`: without the `preserve_order` feature
/// (this workspace does not enable it) it is a `BTreeMap`, so *any* `Value`
/// serialises its object keys in one order — sorted, byte-wise — no matter
/// how it was built or parsed. Rendering the original response from a
/// `Value` and the replay from the `Value` that `response_body` deserialises
/// into therefore produces the same bytes.
///
/// It is **not** that JSONB normalises key order to match: `jsonb` does sort
/// object keys, but by length first and then by bytes, which is a different
/// order from `BTreeMap`'s. That ordering is invisible here precisely
/// because the stored document is parsed back into a `Value` before it is
/// rendered — see `replay`. Serialising a struct straight to bytes would
/// emit fields in declaration order and break the match immediately, which
/// is the mistake this function exists to prevent.
///
/// `pub(crate)` since Step 5: `crate::v1::events` renders through it too, so
/// both resources answer with the identical `Content-Type`, the identical
/// key ordering and the identical error for a body that will not serialise.
/// A second copy in the events module would be a second place for those
/// three to drift — and the key ordering in particular is what makes an
/// idempotent replay byte-for-byte identical, so a divergence would be
/// invisible until a replay failed to match.
///
/// # Errors
///
/// [`ApiError::Internal`] if `body` will not serialise, which for the types
/// this crate renders means a bug in a `Serialize` impl rather than anything
/// a caller sent.
pub(crate) fn json_response<T: serde::Serialize>(
    status: StatusCode,
    body: &T,
) -> Result<Response, ApiError> {
    let value = serde_json::to_value(body).map_err(ApiError::internal_serialization)?;
    Ok(value_response(status, &value))
}

/// Renders an already-built JSON value with the API's content type.
fn value_response(status: StatusCode, value: &Value) -> Response {
    let bytes = serde_json::to_vec(value).unwrap_or_else(|_error| {
        // `Value` cannot fail to serialise (no non-string map keys, no
        // non-finite floats — this workspace denies float arithmetic
        // outright). An empty body would be worse than an unreachable
        // fallback that is still valid JSON.
        b"{}".to_vec()
    });
    let mut response = (status, bytes).into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

/// One `POST` under `/v1`: its idempotency key, its raw body, and the
/// fingerprint the two are claimed under.
///
/// It exists because the idempotency fingerprint is taken over the **raw**
/// body (`crate::idempotency::request_hash`) while the handler needs the
/// *parsed* body, and no two extractors can both consume a body. Reading the
/// request whole here, then handing the same bytes to [`VpayForm`], keeps
/// one decoder for the wire format instead of a second one written for this
/// path.
struct PostRequest {
    key: IdempotencyKey,
    parts: Parts,
    body: Bytes,
    hash: [u8; 32],
}

impl PostRequest {
    async fn read(request: Request) -> Result<Self, ApiError> {
        let (mut parts, body) = request.into_parts();
        // Through the extractor rather than by reading the header here, so
        // the length and charset rules — and D7's exact sentence for a
        // missing key — live in one place.
        let key = IdempotencyKey::from_request_parts(&mut parts, &()).await?;
        let body = axum::body::to_bytes(body, crate::V1_BODY_LIMIT_BYTES)
            .await
            .map_err(|_error| {
                // The 64 KiB layer on the `/v1` nest already answered 413 for
                // an oversized body; what reaches here is a connection that
                // ended early.
                ApiError::invalid_param(
                    "body",
                    "The request body could not be read. Send it in one piece as \
                     `application/x-www-form-urlencoded`.",
                )
            })?;
        let hash = request_hash(parts.method.as_str(), parts.uri.path(), &body);
        Ok(Self {
            key,
            parts,
            body,
            hash,
        })
    }

    /// Decodes the body through the same [`VpayForm`] extractor every other
    /// handler uses, by handing it back the request it came from.
    async fn form<T: serde::de::DeserializeOwned>(&self) -> Result<T, ApiError> {
        let mut request = Request::new(axum::body::Body::from(self.body.clone()));
        *request.method_mut() = self.parts.method.clone();
        *request.uri_mut() = self.parts.uri.clone();
        *request.headers_mut() = self.parts.headers.clone();
        VpayForm::<T>::from_request(request, &())
            .await
            .map(|form| form.0)
    }

    async fn claim(
        &self,
        pool: &PgPool,
        scope: &MerchantScope,
    ) -> Result<IdempotencyClaim, ApiError> {
        idempotency::claim(
            pool,
            scope.merchant_id(),
            self.key.as_str(),
            self.parts.method.as_str(),
            self.parts.uri.path(),
            &self.hash,
        )
        .await
        .map_err(ApiError::from)
    }

    /// Claims the key and resolves what that means for this request: either
    /// this process owns it (and here is the `claim_id` it owns it under),
    /// or here is the answer the merchant is owed.
    ///
    /// One call rather than a `claim` followed by a match at three call
    /// sites, because the `claim_id` must not be droppable: a handler that
    /// destructured [`IdempotencyClaim::Fresh`] and threw the id away would
    /// compile, and would then have nothing to end the claim with.
    async fn claim_or_answer(
        &self,
        pool: &PgPool,
        scope: &MerchantScope,
    ) -> Result<ClaimOutcome, ApiError> {
        match self.claim(pool, scope).await? {
            IdempotencyClaim::Fresh { claim_id } => Ok(ClaimOutcome::Owned(claim_id)),
            IdempotencyClaim::Replay(record) => replay(&record).map(ClaimOutcome::Answered),
            // Its own variant rather than a `Conflict` with a hand-written
            // sentence: a merchant's client has to be able to tell "your
            // intent moved on" from "your own earlier call is still
            // running" by `code`, not by matching on prose. See
            // `ApiError::IdempotencyKeyInFlight`, which also explains why
            // the status is what the policy table says rather than
            // Stripe's 409.
            IdempotencyClaim::InFlight => {
                Err(ApiError::idempotency_key_in_flight(self.key.as_str()))
            }
            IdempotencyClaim::Mismatch => Err(ApiError::idempotency_key_reused(self.key.as_str())),
        }
    }

    /// Stores the outcome under the claimed key, when it is one a replay may
    /// answer with, and returns it.
    ///
    /// # What is stored, and what is handed back
    ///
    /// A `4xx` **is** stored, Stripe's own behaviour: the merchant caused it,
    /// re-running the request would produce it again, and answering the
    /// retry identically is both cheaper and less surprising than
    /// re-executing. ([`create`] carves out one exception — see its docs.)
    ///
    /// A `5xx` is **not** stored, and the key is [released][release]. Two
    /// wrong things were possible here and only the second is obvious:
    ///
    /// * freezing the failure for 24 hours, so a merchant retrying after the
    ///   deployment was fixed is answered with the old outage;
    /// * leaving the key `in_flight` — which is what this code did before,
    ///   and which was worse. Nothing else ever moves such a row, so every
    ///   retry under that key was answered "a request with this
    ///   Idempotency-Key is still in progress" for the *life of the
    ///   deployment*. Before the rails landed, when every `confirm` ended in
    ///   the adapter's `501`, that permanently burned a key on every confirm
    ///   a merchant made.
    ///
    /// Releasing means the retry re-executes. That is safe because it is not
    /// the idempotency key that stops a payment being taken twice — the
    /// unique index `one_charge_per_intent` is, and a re-executed confirm
    /// meets it and answers `409` rather than charging again.
    ///
    /// A failure to release is logged and swallowed: the key expires in 24
    /// hours either way (`vpay_db::idempotency::claim` reclaims an expired
    /// row), and turning a `501` into a `500` would replace an accurate
    /// answer about the rail with an inaccurate one about vpay.
    ///
    /// # Every failure after the claim releases first
    ///
    /// The three steps between "the work is done" and "the response is
    /// stored" can each fail — reading the response body back, parsing it as
    /// JSON, and the write itself — and each of them used to `?`-return
    /// while the key was still claimed. That is the *stuck `in_flight`* bug
    /// this method's own docs warn about, reached by a different door: the
    /// merchant is answered `500`, and every retry under that key is then
    /// answered "still in progress" until the 24-hour window closes. So each
    /// is a `match` that releases before returning, and the module comment's
    /// "a claim is always ended, on every path" is true rather than aspirational.
    ///
    /// Neither of the first two is reachable from a handler in this file
    /// today — every outcome is built by `json_response`/`value_response` or
    /// by `ApiError::into_response`, all of which emit JSON well under
    /// `V1_BODY_LIMIT_BYTES`. They are handled anyway because "unreachable
    /// today" is a property of the callers, not of this method, and the cost
    /// of being wrong about it is a merchant's key locked for a day.
    ///
    /// # A claim that is no longer this request's
    ///
    /// [`IdempotencyStoreOutcome::StaleClaim`] is not an error: this
    /// request's claim expired and a later one took the row over (see
    /// `vpay_db::idempotency`'s ABA note). The work *was* done, so the
    /// response is returned rather than turned into a `500` — it simply
    /// cannot be replayed, and there is no claim of ours left to release.
    ///
    /// [release]: vpay_db::idempotency::release
    /// [`IdempotencyStoreOutcome::StaleClaim`]: vpay_db::IdempotencyStoreOutcome::StaleClaim
    async fn finish(
        self,
        pool: &PgPool,
        scope: &MerchantScope,
        claim_id: Uuid,
        outcome: Result<Response, ApiError>,
    ) -> Result<Response, ApiError> {
        let response = match outcome {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        let status = response.status();
        if status.is_server_error() {
            self.release(pool, scope, claim_id).await;
            return Ok(response);
        }

        // The advisory as this response actually carries it, taken off the
        // rendered `HeaderMap` rather than worked out again from `status`.
        // That is what makes `idempotency_keys.response_retry` (migration
        // 0025) a record of what was sent instead of a second opinion about
        // it — see `error::STRIPE_SHOULD_RETRY_HEADER`. A value that is not
        // ASCII cannot have come from `into_response`'s two constants, and
        // storing it would fail the column's CHECK, so it is dropped.
        let retry = response
            .headers()
            .get(crate::error::STRIPE_SHOULD_RETRY_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let (parts, body) = response.into_parts();
        let bytes = match axum::body::to_bytes(body, crate::V1_BODY_LIMIT_BYTES).await {
            Ok(bytes) => bytes,
            Err(error) => {
                self.release(pool, scope, claim_id).await;
                return Err(ApiError::Internal(format!(
                    "reading a response body back to store it: {error}"
                )));
            }
        };
        let value: Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                self.release(pool, scope, claim_id).await;
                return Err(ApiError::Internal(format!(
                    "a /v1 response body was not JSON and could not be stored for replay: {error}"
                )));
            }
        };

        match idempotency::store(
            pool,
            scope.merchant_id(),
            self.key.as_str(),
            claim_id,
            status.as_u16(),
            &value,
            retry.as_deref(),
        )
        .await
        {
            Ok(IdempotencyStoreOutcome::Stored) => {}
            Ok(IdempotencyStoreOutcome::StaleClaim) => {
                tracing::warn!(
                    merchant_id = %scope.merchant_id(),
                    "this request's Idempotency-Key claim expired and was taken over before its \
                     response could be stored; the response is returned but is not replayable"
                );
            }
            Err(error) => {
                self.release(pool, scope, claim_id).await;
                return Err(ApiError::from(error));
            }
        }

        Ok(Response::from_parts(parts, axum::body::Body::from(bytes)))
    }

    /// Hands the claimed key back, so the merchant's retry re-executes
    /// rather than being told the first attempt is still running.
    ///
    /// Takes `&self` rather than consuming, because the two callers differ:
    /// [`finish`](Self::finish) has already produced the response it is
    /// about to return, while a validation failure releases and then returns
    /// the error it was given.
    ///
    /// Never fails the request. See [`finish`](Self::finish) for why a
    /// failed release is a log line and not an error.
    ///
    /// Deleting nothing is also not a failure: `claim_id` scopes the delete
    /// to *this* claim, so a request whose claim has already been reclaimed
    /// leaves the new owner's row alone — which is the point.
    async fn release(&self, pool: &PgPool, scope: &MerchantScope, claim_id: Uuid) {
        if let Err(error) =
            idempotency::release(pool, scope.merchant_id(), self.key.as_str(), claim_id).await
        {
            tracing::warn!(
                %error,
                merchant_id = %scope.merchant_id(),
                "could not release an Idempotency-Key after an outcome that is not stored; it \
                 stays claimed until it expires, and a retry under it is answered `in progress` \
                 until then"
            );
        }
    }
}

/// What claiming the key settled: this request owns it, or it already has
/// its answer.
///
/// An enum rather than `Option<Response>` because the owning case carries
/// the `claim_id` every later write needs. See
/// [`PostRequest::claim_or_answer`].
enum ClaimOutcome {
    /// This request owns the key under this `claim_id`, and must end the
    /// claim with `PostRequest::finish` or `PostRequest::release`.
    Owned(Uuid),
    /// The key already answers for itself — a stored replay.
    Answered(Response),
}

/// Replays a stored response verbatim — status, body **and** the
/// `stripe-should-retry` advisory the original carried.
///
/// The advisory is re-emitted from `idempotency_keys.response_retry`
/// (migration `0025`) and is never re-derived from `record.response_status`.
/// A replay that worked the header out again from the status would be a
/// second classification of an error this deployment may since have
/// reclassified — the drift ADR-0011 exists to prevent — and would be wrong
/// today for any status two categories share. `None` means the stored
/// response carried none, which is what a stored `2xx` looks like: only
/// `ApiError::into_response` sets the header.
fn replay(record: &IdempotencyRecord) -> Result<Response, ApiError> {
    let (Some(status), Some(body)) = (record.response_status, record.response_body.as_ref()) else {
        // The `complete_has_a_response` CHECK (migration 0015) makes this
        // unreachable from the database's side, so reaching it means the
        // schema is not what this code believes.
        return Err(ApiError::Internal(
            "a completed idempotency record carries no stored response".to_owned(),
        ));
    };
    let status = u16::try_from(status)
        .ok()
        .and_then(|status| StatusCode::from_u16(status).ok())
        .ok_or_else(|| {
            ApiError::Internal(format!(
                "a stored idempotent response carries {status}, which is not an HTTP status"
            ))
        })?;

    let mut response = value_response(status, body);
    // Parsed rather than trusted: the column's CHECK already confines it to
    // `true`/`false`, so an unparseable value means the row was written by
    // something other than `store`. Dropping it re-creates the pre-0025
    // behaviour for that row instead of failing a replay a merchant is
    // waiting on.
    if let Some(advisory) = record
        .response_retry
        .as_deref()
        .and_then(|advisory| HeaderValue::from_str(advisory).ok())
    {
        response
            .headers_mut()
            .insert(crate::error::STRIPE_SHOULD_RETRY_HEADER, advisory);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, ProviderHost};

    use super::*;

    fn resource_config() -> ResourceConfig {
        ResourceConfig::from_config(&Config {
            deployment: Deployment {
                name: "payment-intents-tests".to_owned(),
                livemode: false,
                public_base_url: "https://api.vpay.test".to_owned(),
            },
            providers: vec![
                ProviderHost {
                    code: "mtn_momo".to_owned(),
                    enabled: true,
                    host: HostEntry {
                        url: "https://mtn.example".to_owned(),
                        label: "mtn".to_owned(),
                    },
                    settings: BTreeMap::new(),
                    callback_url: None,
                    currency: "XAF".to_owned(),
                    credentials: BTreeMap::new(),
                },
                ProviderHost {
                    code: "orange_money".to_owned(),
                    enabled: false,
                    host: HostEntry {
                        url: "https://orange.example".to_owned(),
                        label: "orange".to_owned(),
                    },
                    settings: BTreeMap::new(),
                    callback_url: None,
                    currency: "XAF".to_owned(),
                    credentials: BTreeMap::new(),
                },
            ],
            currencies: vec![CurrencyEntry {
                code: "XAF".to_owned(),
                exponent: 0,
            }],
            merchant_clients: vec![crate::test_fixtures::merchant(
                "acme-cameroon",
                &["payments:write"],
            )],
            dashboard_client: None,
        })
        .expect("the fixture's rails project onto the port")
    }

    fn param_of(error: &ApiError) -> Option<&str> {
        match error {
            ApiError::InvalidParam { param, .. } => Some(param.as_str()),
            _ => None,
        }
    }

    /// The claim `confirm_once`'s step 1 depends on: whether a confirm is
    /// legal is a property of the intent's *status* alone, so the 409 can be
    /// decided before the rail is resolved.
    ///
    /// If `next_status` ever gains a flow-dependent arm, this fails — and
    /// `confirm_once` would then be answering 409 for a status one rail
    /// would have accepted, which is why it asks with `Push` hard-coded and
    /// this test is what licenses that.
    #[test]
    fn confirm_legality_does_not_depend_on_the_rails_flow() {
        for status in [
            IntentStatus::RequiresPaymentMethod,
            IntentStatus::RequiresAction,
            IntentStatus::Processing,
            IntentStatus::Succeeded,
            IntentStatus::Canceled,
        ] {
            assert_eq!(
                next_status(status, Transition::Confirm(ProviderFlow::Push)).is_some(),
                next_status(status, Transition::Confirm(ProviderFlow::Redirect)).is_some(),
                "a confirm from `{}` must be legal on both flows or on neither; \
                 confirm_once decides its 409 before it knows the rail",
                status.as_wire_str(),
            );
        }
    }

    /// Both currency gates, and which one a caller trips.
    #[test]
    fn a_currency_the_deployment_did_not_configure_is_refused_by_name() {
        let config = resource_config();
        assert_eq!(
            parse_currency(Some("xaf"), &config).expect("XAF is configured"),
            Currency::Xaf,
        );
        // Known to `vpay_core::Currency`, not configured here.
        let error = parse_currency(Some("EUR"), &config).expect_err("EUR is not configured");
        assert_eq!(param_of(&error), Some("currency"));
        // Not a currency at all.
        let error = parse_currency(Some("zzz"), &config).expect_err("ZZZ is not a currency");
        assert_eq!(param_of(&error), Some("currency"));
        let error = parse_currency(None, &config).expect_err("currency is required");
        assert_eq!(param_of(&error), Some("currency"));
    }

    /// A disabled rail is configured and not offerable: an intent created
    /// naming it could never be confirmed.
    #[test]
    fn a_disabled_rail_cannot_be_named_on_a_new_intent() {
        let config = resource_config();
        assert_eq!(
            validated_rails(&["mtn_momo".to_owned()], &config).expect("mtn is enabled"),
            vec!["mtn_momo".to_owned()],
        );
        for requested in [vec!["orange_money".to_owned()], vec!["nope".to_owned()]] {
            let error = validated_rails(&requested, &config)
                .expect_err("a disabled or unknown rail is refused");
            assert_eq!(param_of(&error), Some("payment_method_types"));
        }
        let error = validated_rails(&[], &config).expect_err("an empty list is refused");
        assert_eq!(param_of(&error), Some("payment_method_types"));
    }

    #[test]
    fn the_amount_bounds_are_the_ones_both_sdks_enforce() {
        assert_eq!(parse_amount(Some("5000")).expect("a plain amount"), 5000);
        assert_eq!(parse_amount(Some(" 1 ")).expect("surrounding space"), 1);
        assert_eq!(
            parse_amount(Some(&MAX_AMOUNT.to_string())).expect("the bound itself is accepted"),
            MAX_AMOUNT,
        );
        for raw in ["0", "-1", "9007199254740992", "5000.0", "", "five thousand"] {
            let error = parse_amount(Some(raw)).expect_err("refused: {raw}");
            assert_eq!(param_of(&error), Some("amount"), "for {raw:?}");
        }
        let error = parse_amount(None).expect_err("amount is required");
        assert_eq!(param_of(&error), Some("amount"));
    }

    #[test]
    fn metadata_and_description_bounds_name_their_own_parameter() {
        let ok = BTreeMap::from([("order_id".to_owned(), "1234".to_owned())]);
        assert_eq!(validated_metadata(&ok).expect("within bounds").len(), 1,);

        let too_many: BTreeMap<String, String> = (0..=METADATA_MAX_KEYS)
            .map(|index| (format!("k{index}"), "v".to_owned()))
            .collect();
        assert_eq!(
            param_of(&validated_metadata(&too_many).expect_err("51 keys")),
            Some("metadata")
        );

        let long_key = BTreeMap::from([("k".repeat(METADATA_MAX_KEY_CHARS + 1), "v".to_owned())]);
        assert_eq!(
            param_of(&validated_metadata(&long_key).expect_err("a 41-char key")),
            Some("metadata")
        );

        let long_value =
            BTreeMap::from([("k".to_owned(), "v".repeat(METADATA_MAX_VALUE_CHARS + 1))]);
        assert_eq!(
            param_of(&validated_metadata(&long_value).expect_err("a 501-char value")),
            Some("metadata")
        );

        assert_eq!(
            validated_description(Some("x".repeat(DESCRIPTION_MAX_CHARS)))
                .expect("the bound itself is accepted")
                .map(|description| description.chars().count()),
            Some(DESCRIPTION_MAX_CHARS),
        );
        assert_eq!(
            param_of(
                &validated_description(Some("x".repeat(DESCRIPTION_MAX_CHARS + 1)))
                    .expect_err("one over")
            ),
            Some("description")
        );
        assert_eq!(validated_description(None).expect("absent is fine"), None);
    }

    /// The rails an intent was created with are the rails it may be
    /// confirmed against, and a `payment_method_types` that is not an array
    /// of strings admits nothing rather than everything.
    #[test]
    fn an_intent_only_allows_the_rails_it_was_created_with() {
        assert!(intent_allows(
            &serde_json::json!(["mtn_momo", "orange_money"]),
            "mtn_momo"
        ));
        assert!(!intent_allows(
            &serde_json::json!(["orange_money"]),
            "mtn_momo"
        ));
        assert!(!intent_allows(&serde_json::json!([]), "mtn_momo"));
        assert!(!intent_allows(&Value::Null, "mtn_momo"));
        assert!(!intent_allows(&serde_json::json!("mtn_momo"), "mtn_momo"));
    }

    /// The `provider_requests.error_kind` a failed submit records is the
    /// error's own classification code, not a second vocabulary.
    ///
    /// The decline is the case worth pinning: the *taxonomy* code
    /// (`insufficient_funds`) is what `charges.failure_code` gets, and this
    /// column deliberately gets the classification instead, so an operator
    /// counting rail attempts and a merchant reading an envelope are never
    /// looking at the same word meaning two things
    /// (`docs/flows/errors.md`'s note on `charge_declined`).
    #[test]
    fn the_recorded_error_kind_is_the_errors_own_code() {
        assert_eq!(
            error_kind(&ProviderError::NotImplemented("mtn_momo::submit")),
            "not_implemented"
        );
        assert_eq!(
            error_kind(&ProviderError::Transport("connection refused".to_owned())),
            "provider_unavailable"
        );
        assert_eq!(
            error_kind(&ProviderError::Rejected {
                code: vpay_core::FailureCode::InsufficientFunds,
                message: "NOT_ENOUGH_FUNDS".to_owned(),
            }),
            "charge_declined"
        );
    }

    /// The rail's profile currency against the intent's, and what a
    /// mismatch costs: a `400` on the parameter the caller can still change.
    ///
    /// The fixture's `mtn_momo` settles in XAF, so the EUR case has to be
    /// built from the other rail — this is exactly the shape
    /// `config/application.yml` ships (MTN on EUR, Orange on XAF), and it
    /// is why the check exists at all.
    #[test]
    fn a_rail_that_settles_in_another_currency_is_refused_by_name() {
        let config = resource_config();
        let xaf_rail = config.rail("mtn_momo").expect("configured");
        currencies_agree(xaf_rail, "XAF").expect("the rail settles in the intent's currency");

        let error = currencies_agree(xaf_rail, "EUR").expect_err("XAF rail, EUR intent");
        assert_eq!(param_of(&error), Some(PMD_TYPE_PARAM));
        let message = vpay_core::Classify::public_message(&error);
        assert!(
            message.contains("XAF") && message.contains("EUR"),
            "the refusal has to name both currencies or a merchant cannot act on it: {message}"
        );
    }

    /// `next_action` is rendered from the charge row, never from a
    /// `Submitted` still in memory — which is what makes it reproducible on
    /// a later `GET` and what makes the commit the gate on the redirect.
    #[test]
    fn a_next_action_is_built_from_the_stored_row_and_needs_a_url() {
        let mut charge = charge_fixture();
        assert_eq!(
            next_action_of(&charge),
            None,
            "a charge with no redirect_url has no next_action, whatever else it holds"
        );

        charge.redirect_url = Some("https://pay.example/abc".to_owned());
        charge.return_url = Some("https://shop.example/return".to_owned());
        assert_eq!(
            next_action_of(&charge),
            Some(NextAction::RedirectToUrl {
                redirect_to_url: RedirectToUrl {
                    url: "https://pay.example/abc".to_owned(),
                    return_url: Some("https://shop.example/return".to_owned()),
                },
            })
        );

        // A rail that was given no return destination still gets a
        // next_action: the key is `null`, not missing.
        charge.return_url = None;
        assert!(matches!(
            next_action_of(&charge),
            Some(NextAction::RedirectToUrl {
                redirect_to_url: RedirectToUrl {
                    return_url: None,
                    ..
                }
            })
        ));
    }

    /// Both database CHECKs count characters, so the truncation does too —
    /// a rail answering in a non-ASCII script must not produce a panic or a
    /// constraint violation rendered as a 500.
    #[test]
    fn a_rails_words_are_truncated_on_a_character_boundary() {
        assert_eq!(bounded("short", FAILURE_RAW_MAX_CHARS), "short");
        assert_eq!(
            bounded(&"é".repeat(600), LAST_PAYMENT_ERROR_MAX_CHARS)
                .chars()
                .count(),
            LAST_PAYMENT_ERROR_MAX_CHARS,
        );
        assert_eq!(bounded("abc", 0), "");
    }

    /// The message a merchant is given when their intent already has a
    /// charge, and the reason it is not one message.
    ///
    /// The `submitting` row is the one that matters: it is what a confirm
    /// whose `submit` timed out leaves behind, the rail may be holding the
    /// payment, and "create a new payment intent to try again" would be an
    /// instruction to charge the payer twice
    /// (`docs/flows/crash-safety.md`).
    #[test]
    fn a_live_charge_is_never_answered_with_advice_to_open_a_second_intent() {
        for state in [
            vpay_core::ChargeState::Submitting,
            vpay_core::ChargeState::Submitted,
            vpay_core::ChargeState::Pending,
            vpay_core::ChargeState::Unresolved,
        ] {
            let mut charge = charge_fixture();
            charge.state = state.as_wire_str().to_owned();
            let message = conflict_message(already_charged("pi_live", Some(&charge)));

            assert!(
                message.contains("do not create a new PaymentIntent"),
                "{state:?}: {message}"
            );
            assert!(
                message.contains("pi_live"),
                "the merchant is told what to poll: {message}"
            );
            assert!(
                !message.contains("create a new payment intent to try again"),
                "{state:?} is live; that advice would double-charge the payer: {message}"
            );
        }
    }

    /// The terminal half, which keeps the original sentence: there is
    /// nothing in flight, so "one charge per intent, forever — a retry is a
    /// new intent" is both true and the only thing the merchant can do.
    #[test]
    fn a_terminal_charge_still_says_a_retry_is_a_new_intent() {
        for state in [
            vpay_core::ChargeState::Succeeded,
            vpay_core::ChargeState::Failed,
        ] {
            let mut charge = charge_fixture();
            charge.state = state.as_wire_str().to_owned();
            let message = conflict_message(already_charged("pi_done", Some(&charge)));

            assert!(
                message.contains("One charge per intent, forever"),
                "{state:?}: {message}"
            );
            assert!(
                !message.contains("do not create a new PaymentIntent"),
                "{state:?}: {message}"
            );
        }
    }

    /// The two cases where the state is not knowable — the unique-violation
    /// race whose re-read failed, and a label this build does not
    /// understand. Both must fall to the safe side: waiting for a settled
    /// charge costs a `GET`, retrying a live one costs a payer twice.
    #[test]
    fn an_unknowable_charge_state_is_treated_as_live() {
        let mut unparseable = charge_fixture();
        unparseable.state = "reticulating_splines".to_owned();

        for existing in [None, Some(&unparseable)] {
            let message = conflict_message(already_charged("pi_unknown", existing));
            assert!(
                message.contains("do not create a new PaymentIntent"),
                "{message}"
            );
        }
    }

    fn conflict_message(error: ApiError) -> String {
        match error {
            ApiError::Conflict { message } => message,
            other => panic!("expected a 409 Conflict, got {other:?}"),
        }
    }

    /// The two strings [`insert_charge`] enqueues under, transcribed from
    /// `vpay_worker::jobs` (`JobKind::PollCharge::as_wire_str` and
    /// `poll_dedupe_key`) and from migration 0021's `kind_is_known` CHECK.
    ///
    /// Transcribed rather than imported because `vpay-worker` depends on this
    /// crate; even as a dev-dependency the edge back would be a cycle. So
    /// this is the cheap half of the guard, and it only catches a change made
    /// *here*. The half that catches a change made in the worker is
    /// `worker_e2e.rs`, which asserts the row this handler writes against the
    /// worker's own constants and then drives the job to a settled payment —
    /// if these ever disagree, that suite's confirm never reaches
    /// `succeeded`.
    #[test]
    fn the_enqueued_job_matches_the_workers_own_spelling() {
        assert_eq!(POLL_CHARGE_KIND, "poll_charge");
        assert_eq!(poll_dedupe_key("ch_abc"), "poll:ch_abc");
        // A charge id is already unique, so the key is too — the property the
        // `jobs_dedupe_key` unique index turns into "one poll job per charge".
        assert_ne!(poll_dedupe_key("ch_a"), poll_dedupe_key("ch_b"));
        // Namespaced, so the worker's own `resubmit:<id>` cannot collide with
        // it and lose to its `ON CONFLICT DO NOTHING`.
        assert!(poll_dedupe_key("ch_abc").starts_with("poll:"));
    }

    /// `return_url` is persisted and then rendered back into a browser, so
    /// the two things a merchant must not be able to put there are a scheme
    /// a browser *executes* and a value the column would refuse.
    ///
    /// The second is the subtler one: without this check the CHECK in
    /// migration `0019` fires and the merchant is told, with a `503`, that
    /// vpay is broken — for a field they got wrong.
    #[test]
    fn a_return_url_must_be_a_bounded_web_url() {
        for accepted in [
            "https://shop.example/return",
            "http://localhost:3000/return?order=1",
            // Schemes are case-insensitive, and the column's CHECK
            // lowercases too — the two layers must agree.
            "HTTPS://shop.example/return",
            &format!("https://shop.example/{}", "x".repeat(2_000)),
        ] {
            assert!(
                checked_return_url(accepted).is_ok(),
                "must be accepted: {accepted}"
            );
        }

        for (refused, why) in [
            (
                "javascript:alert(1)",
                "a browser executes this rather than navigating",
            ),
            ("data:text/html;base64,PHNjcmlwdD4=", "same"),
            ("//shop.example/return", "scheme-relative is not a scheme"),
            ("shop.example/return", "no scheme at all"),
        ] {
            let error = checked_return_url(refused).expect_err(why);
            assert_eq!(
                invalid_param_of(error).as_deref(),
                Some("return_url"),
                "{refused}: the merchant must be told which field is wrong"
            );
        }

        let too_long = format!("https://shop.example/{}", "x".repeat(RETURN_URL_MAX_CHARS));
        assert_eq!(
            invalid_param_of(checked_return_url(&too_long).expect_err("over the column limit"))
                .as_deref(),
            Some("return_url"),
        );

        // Exactly at the limit is accepted, or the API and the column
        // disagree by one and the boundary case becomes a 503.
        let exact = format!(
            "https://shop.example/{}",
            "x".repeat(RETURN_URL_MAX_CHARS - "https://shop.example/".len())
        );
        assert_eq!(exact.chars().count(), RETURN_URL_MAX_CHARS);
        assert!(checked_return_url(&exact).is_ok());
    }

    /// `confirm=true` on create is refused, and `confirm=false` is not.
    ///
    /// The `false` half is the one that would be lost by "reject the field
    /// whenever it is present": a merchant asking vpay *not* to confirm is
    /// asking for exactly what this endpoint does, and a 400 there refuses a
    /// request that was already correct.
    ///
    /// The `true` half is what stops a copied Stripe snippet from returning
    /// a `200` and an unconfirmed intent to a merchant who believes they
    /// charged someone.
    #[test]
    fn confirm_on_create_is_refused_unless_it_asks_for_nothing() {
        assert!(reject_confirm_on_create(None).is_ok());
        assert!(reject_confirm_on_create(Some("false")).is_ok());

        for asked in ["true", "1", "yes", ""] {
            let error = reject_confirm_on_create(Some(asked))
                .expect_err("`confirm` that is not `false` must be refused");
            assert_eq!(
                param_of(&error),
                Some("confirm"),
                "a Stripe SDK points its user at `error.param`; got {error:?}"
            );
            let ApiError::InvalidParam { message, .. } = &error else {
                panic!("expected an invalid-parameter error, got {error:?}")
            };
            assert!(
                message.contains("/v1/payment_intents/{id}/confirm"),
                "the refusal must name the endpoint that does the work: {message}"
            );
        }
    }

    /// The fields that change where or when money moves are refused, on the
    /// **create** body and on the **confirm** body, decoded by the real form
    /// decoder rather than by constructing the struct.
    ///
    /// Going through `crate::form::parse_form` is the point: what a merchant
    /// sends is bytes, and `transfer_data[destination]=acct_x` only reaches
    /// `UnsupportedStripeParams::transfer_data` if the bracket nesting, the
    /// `#[serde(flatten)]` and the field name all agree. A test that built
    /// the struct by hand would pass with a field the wire can never fill.
    ///
    /// `capture_method=automatic` is in the accepted column deliberately: it
    /// asks for exactly what vpay does, and refusing it would refuse a
    /// correct request — the same rule `confirm=false` follows.
    #[test]
    fn the_fields_that_move_money_elsewhere_are_refused_through_the_real_decoder() {
        fn create_of(body: &str) -> CreateParams {
            let value = crate::form::parse_form(body.as_bytes()).expect("the body decodes");
            serde_json::from_value(value).expect("a create body deserializes")
        }
        fn confirm_of(body: &str) -> ConfirmParams {
            let value = crate::form::parse_form(body.as_bytes()).expect("the body decodes");
            serde_json::from_value(value).expect("a confirm body deserializes")
        }

        const VALID: &str = "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo";
        const PMD: &str = "payment_method_data[type]=mtn_momo\
                           &payment_method_data[mtn_momo][msisdn]=237670000000";

        // Refused, on both bodies, each naming its own field.
        for (suffix, param) in [
            ("&capture_method=manual", "capture_method"),
            ("&application_fee_amount=250", "application_fee_amount"),
            ("&transfer_data[destination]=acct_x", "transfer_data"),
            ("&on_behalf_of=acct_x", "on_behalf_of"),
        ] {
            for (label, refused) in [
                (
                    "create",
                    create_of(&format!("{VALID}{suffix}"))
                        .unsupported
                        .reject_unsupported(),
                ),
                (
                    "confirm",
                    confirm_of(&format!("{PMD}{suffix}"))
                        .unsupported
                        .reject_unsupported(),
                ),
            ] {
                let error = refused.expect_err(&format!("{label}: `{suffix}` must be refused"));
                assert_eq!(
                    param_of(&error),
                    Some(param),
                    "{label}: a Stripe SDK points its user at `error.param`; got {error:?}"
                );
                let ApiError::InvalidParam { message, .. } = &error else {
                    panic!("{label}: expected an invalid-parameter error, got {error:?}")
                };
                assert!(
                    message.contains("does not support"),
                    "{label}: the refusal must say vpay cannot do it: {message}"
                );
                // The envelope truncates a public message at
                // `MESSAGE_MAX_CHARS` (200) with no warning, so a sentence
                // edited past the cap silently loses its ending — and the
                // ending is where these two say what to do instead.
                assert!(
                    message.chars().count() <= 200,
                    "{label}: `{param}`'s message is {} chars and the envelope caps at 200: \
                     {message}",
                    message.chars().count()
                );
                // And it is one sentence rather than a line-continuation
                // that lost its backslash: `\` at the end of a Rust string
                // literal line eats the newline *and* the indentation, and
                // without it the merchant reads a paragraph of spaces.
                assert!(
                    !message.contains("  "),
                    "{label}: `{param}`'s message carries a run of spaces: {message:?}"
                );
            }
        }

        // Accepted: the value that asks for what vpay already does, and a
        // body that mentions none of them.
        for body in [
            VALID.to_owned(),
            format!("{VALID}&capture_method=automatic"),
            // The fields vpay ignores rather than refuses, all at once —
            // this is the half that fails if the refusal is widened to
            // "anything Stripe has that vpay lacks".
            format!(
                "{VALID}&setup_future_usage=off_session&confirmation_method=automatic\
                 &receipt_email=a@example.com&statement_descriptor=ACME&customer=cus_1\
                 &expand[0]=charge&metadata[order_id]=1234"
            ),
        ] {
            assert!(
                create_of(&body).unsupported.reject_unsupported().is_ok(),
                "must be accepted: {body}"
            );
        }
        assert!(
            confirm_of(&format!("{PMD}&capture_method=automatic"))
                .unsupported
                .reject_unsupported()
                .is_ok()
        );
    }

    /// A replayed response carries the `stripe-should-retry` it was stored
    /// with — the same value the fresh response carried, not a fresh opinion
    /// about the stored status.
    ///
    /// This is the test that used to pin the opposite. Before migration
    /// `0025` [`replay`] rebuilt a response from a status and a body alone,
    /// so a merchant whose stored answer was a `409` got it back bare and
    /// stripe-node applied its own "retry every 409" rule to a refusal that
    /// will never change. `error::STRIPE_SHOULD_RETRY_HEADER` records why the
    /// column stores the header's own text rather than re-deriving it.
    ///
    /// The equality with a *freshly rendered* `Conflict` is the assertion
    /// that matters: it is what fails if `finish` ever starts storing
    /// something other than what `into_response` emitted.
    ///
    /// The `None` row is the other half. A stored `2xx` never went through
    /// the error renderer, so its replay must emit no header at all — a
    /// `false` there would tell an SDK not to retry a successful create,
    /// which is meaningless, and a `true` would tell it to re-POST one.
    #[test]
    fn a_replayed_response_carries_the_advisory_it_was_stored_with() {
        let stored = |status: i16, retry: Option<&str>| IdempotencyRecord {
            request_hash: vec![0u8; 32],
            state: "complete".to_owned(),
            response_status: Some(status),
            response_body: Some(serde_json::json!({
                "error": {
                    "type": "invalid_request_error",
                    "code": "invalid_state",
                    "message": "This payment intent already has a charge.",
                }
            })),
            response_retry: retry.map(str::to_owned),
        };
        let advisory_of = |response: &Response| {
            response
                .headers()
                .get("stripe-should-retry")
                .map(|value| value.to_str().expect("the advisory is ascii").to_owned())
        };

        let replayed = replay(&stored(409, Some("false"))).expect("a complete record replays");
        assert_eq!(replayed.status(), StatusCode::CONFLICT);
        assert_eq!(advisory_of(&replayed).as_deref(), Some("false"));

        // The same error rendered *fresh* carries the same value: what a
        // replay hands back and what the classification says are one thing.
        let fresh = ApiError::Conflict {
            message: "This payment intent already has a charge.".to_owned(),
        }
        .into_response();
        assert_eq!(fresh.status(), StatusCode::CONFLICT);
        assert_eq!(
            advisory_of(&replayed),
            advisory_of(&fresh),
            "a replayed 409 and a fresh 409 must advise the same thing"
        );

        // `true` is not hard-coded anywhere in `replay` — it comes back
        // because it was stored.
        let in_flight = replay(&stored(400, Some("true"))).expect("a complete record replays");
        assert_eq!(advisory_of(&in_flight).as_deref(), Some("true"));

        // And a response that carried none replays with none.
        let none = replay(&stored(200, None)).expect("a complete record replays");
        assert!(
            advisory_of(&none).is_none(),
            "a stored 2xx never had an advisory, so its replay must not invent one"
        );
    }

    fn invalid_param_of(error: ApiError) -> Option<String> {
        match error {
            ApiError::InvalidParam { param, .. } => Some(param),
            other => panic!("expected an invalid-parameter error, got {other:?}"),
        }
    }

    /// A charge row as the confirm path commits one, for the render tests
    /// above. Not a database read — these assertions are about the
    /// rendering, and the repository's own suite covers the columns.
    fn charge_fixture() -> ChargeRow {
        ChargeRow {
            id: ids::charge_id(),
            payment_intent_id: ids::payment_intent_id(),
            provider_code: "orange_money".to_owned(),
            provider_reference_id: Uuid::nil(),
            provider_ref_extra: None,
            // Migration 0021's column: only the settlement transaction ever
            // writes it, and a charge the confirm path has just committed
            // has not settled.
            provider_txn_id: None,
            redirect_url: None,
            return_url: None,
            state: vpay_core::ChargeState::INITIAL.as_wire_str().to_owned(),
            amount: 5000,
            currency_code: "XAF".to_owned(),
            payer_ref: None,
            payer_ref_masked: None,
            failure_code: None,
            failure_raw: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }
}
