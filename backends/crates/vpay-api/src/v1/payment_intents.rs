//! `/v1/payment_intents` — create, retrieve, list, confirm, cancel.
//!
//! Everything a merchant can do to a payment intent through this API, and
//! nothing it cannot: `confirm` reaches the rail adapter and stops at the
//! adapter's own `ProviderError::NotImplemented`, which is a real `501` and
//! not a fabricated success (`docs/status.md`, `AGENTS.md`'s second rule).
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
    ChargeRow, IdempotencyClaim, IdempotencyRecord, IdempotencyStoreOutcome, ListPage, NewCharge,
    NewPaymentIntent, PgPool, charges, idempotency, payment_intents, provider_requests,
};
use vpay_provider::{ChargeRef, ProviderAdapter, ProviderError};

use crate::error::ApiError;
use crate::form::{VpayForm, VpayQuery};
use crate::idempotency::{IdempotencyKey, request_hash};
use crate::model::{ListObject, PaymentIntentObject};
use crate::v1::{MerchantScope, ResourceConfig};

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

/// Page size when a caller names none, and the ceiling it is capped to.
const DEFAULT_LIMIT: i64 = 10;
const MAX_LIMIT: i64 = 100;

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
/// `next_action` is `null` on every intent this deployment can currently
/// produce, and that is not an omission: it is derived from a charge's
/// `redirect_url`, which only a successful `submit` on a redirect rail can
/// write — and no adapter implements `submit` yet (`docs/status.md`).
/// Loading the charge here to render a field that cannot be populated would
/// be a query per request bought for nothing, and the missing half would
/// still be missing.
pub(crate) async fn retrieve(
    State(pool): State<PgPool>,
    scope: MerchantScope,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let row = payment_intents::get_for_merchant(&pool, scope.merchant_id(), &id)
        .await?
        .ok_or_else(|| not_found(&id))?;
    object_response(&row)
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
    let page = ListPage {
        limit: parse_limit(params.limit.as_deref())?,
        starting_after: validated_cursor(STARTING_AFTER, params.starting_after)?,
        ending_before: validated_cursor(ENDING_BEFORE, params.ending_before)?,
    };
    if page.starting_after.is_some() && page.ending_before.is_some() {
        return Err(ApiError::invalid_param(
            STARTING_AFTER,
            "Use either `starting_after` or `ending_before`, not both: they name opposite \
             directions through the list.",
        ));
    }

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
/// is in flight". Today that is not even a narrow window: a confirm that
/// ends in the rail's `501` leaves exactly that state behind on purpose.
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
/// 5. call the adapter — synchronously; `submit` is not `async`;
/// 6. record what came back and answer.
///
/// Today step 5 always returns `ProviderError::NotImplemented`, so step 6
/// records `error_kind = 'not_implemented'` and this endpoint answers `501`.
/// The `submitting` charge row and the status-less `provider_requests` row
/// stay behind **on purpose**: they are exactly the state a crash between
/// steps 4 and 6 would leave, and the recovery pass that will read them is
/// the next piece of work (`docs/status.md`). The intent itself stays
/// `requires_payment_method`, because nothing was submitted.
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
    if charges::get_for_intent(pool, &intent.id).await?.is_some() {
        return Err(already_charged());
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

    let flow = adapter.capabilities().flow;
    // The *only* branch on the rail in this file, and it is on the flow
    // shape, never on the code (ADR-0002).
    // The redirect rail's `return_url` is validated and then deliberately
    // dropped: `charges` has no column for it, and the only thing that would
    // read it is the `next_action.return_url` a successful redirect `submit`
    // would render — which nothing can produce yet (`docs/status.md`). The
    // validation still has to happen now, so that a redirect confirm without
    // one is refused rather than reaching a rail with nowhere to send the
    // payer back to.
    let (payer_ref, _return_url) = match flow {
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
            (None, Some(return_url.to_owned()))
        }
    };
    let currency = Currency::from_code(&intent.currency_code)?;
    let amount = Money::new(intent.amount, currency)?;

    // --- step 3: the reference, durable before anything is submitted
    let reference = Uuid::new_v4();
    let charge = insert_charge(
        pool,
        &intent.id,
        code,
        reference,
        intent.amount,
        &intent.currency_code,
        payer_ref.clone(),
    )
    .await?;

    // --- step 4: the attempt, recorded before the call
    let attempt =
        provider_requests::insert_pending(pool, &charge.id, code, "submit", reference, 1).await?;

    // --- step 5: the rail. `submit` is synchronous — do not `.await` it.
    let charge_ref = ChargeRef {
        reference_id: reference,
        amount,
        payer_ref,
        ref_extra: BTreeMap::new(),
    };
    let submitted = adapter.submit(&charge_ref, &rail.provider_config(currency));

    // --- step 6: what came back
    match submitted {
        Err(error) => {
            provider_requests::record_response(pool, attempt, None, Some(error_kind(&error)))
                .await?;
            Err(ApiError::from(error))
        }
        Ok(_) => {
            // Unreachable today: every linked adapter answers
            // `NotImplemented` (`docs/status.md`). It is written as a loud
            // internal failure rather than as a rendered success because
            // there is nothing here that *could* be honest: persisting a
            // submitted charge needs writes this layer has no repository
            // call for — the rail's `ref_extra`, a redirect rail's
            // `redirect_url`, the charge's move out of `submitting`, and the
            // intent's move to `processing`/`requires_action`. Returning the
            // intent as if nothing had happened would tell a merchant a
            // payer was never prompted when they may already have been.
            provider_requests::record_response(pool, attempt, None, Some("not_persisted")).await?;
            Err(ApiError::Internal(format!(
                "adapter `{code}` reported a successful submit for charge {} \
                 (reference {reference}), and vpay cannot persist it: recording a submitted \
                 charge is not built yet. The rail may hold a live payment.",
                charge.id,
            )))
        }
    }
}

/// Commits the charge row in `submitting`, in its own transaction, before
/// any network call.
///
/// A transaction for a single insert looks redundant and is not: the
/// signature `vpay_db::charges::insert_for_intent` offers takes a
/// `PgConnection` precisely so the commit point is the caller's decision,
/// and this one has to be *before* the adapter call rather than pooled into
/// some later unit of work.
async fn insert_charge(
    pool: &PgPool,
    payment_intent_id: &str,
    provider_code: &str,
    reference: Uuid,
    amount: i64,
    currency_code: &str,
    payer_ref: Option<String>,
) -> Result<ChargeRow, ApiError> {
    let new = NewCharge {
        id: ids::charge_id(),
        payment_intent_id: payment_intent_id.to_owned(),
        provider_code: provider_code.to_owned(),
        provider_reference_id: reference,
        provider_ref_extra: None,
        redirect_url: None,
        state: vpay_core::ChargeState::INITIAL.as_wire_str().to_owned(),
        amount,
        // The intent's currency, verbatim: no conversion, and no per-rail
        // currency check (this step's D2).
        currency_code: currency_code.to_owned(),
        payer_ref,
        payer_ref_masked: None,
    };

    let mut tx = pool.begin().await.map_err(vpay_db::DbError::Query)?;
    let charge = match charges::insert_for_intent(&mut tx, &new).await {
        Ok(charge) => charge,
        // The unique index is the enforcement; this arm is the race the
        // read in step 1 cannot close. Same 409 either way, so a merchant
        // cannot tell a race from a sequential second confirm — and does not
        // need to.
        Err(vpay_db::DbError::UniqueViolation { constraint, .. })
            if constraint == "one_charge_per_intent" =>
        {
            return Err(already_charged());
        }
        Err(error) => return Err(error.into()),
    };
    tx.commit().await.map_err(vpay_db::DbError::Query)?;
    Ok(charge)
}

/// The 409 both the read and the unique-index race produce.
fn already_charged() -> ApiError {
    ApiError::Conflict {
        message: "This payment intent already has a charge. One charge per intent, forever \
                  — create a new payment intent to try again."
            .to_owned(),
    }
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

/// The two cursor parameters, named once so the `param` a caller is told to
/// fix cannot drift from the field they sent.
const STARTING_AFTER: &str = "starting_after";
const ENDING_BEFORE: &str = "ending_before";

/// Checks a cursor's *shape* — that it could be one of this merchant's
/// `pi_…` ids — and nothing else.
///
/// The repository resolves a cursor id to a `seq` with a merchant-scoped
/// subquery, so an id that matches nothing (a typo, another merchant's, a
/// deleted one) yields `NULL`, every comparison against `NULL` is false, and
/// the page comes back **empty**. That silence is the right answer for a
/// *foreign* id — telling the caller apart from an empty page would make the
/// list an existence oracle across tenants — and the wrong one for a
/// mistyped id, where the merchant is left staring at an empty list with
/// nothing to fix.
///
/// A shape check separates the two without leaking anything: it depends only
/// on the bytes the caller sent, never on what is in the database. A
/// well-formed id that names no row of theirs still returns the empty page,
/// deliberately.
///
/// An empty value is treated as absent rather than as a malformed cursor:
/// `?starting_after=` is what a client templating an optional field emits
/// when it has none, and refusing it would break paging for a caller that is
/// not paging.
fn validated_cursor(param: &'static str, raw: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(cursor) = raw.map(|cursor| cursor.trim().to_owned()) else {
        return Ok(None);
    };
    if cursor.is_empty() {
        return Ok(None);
    }
    if !ids::is_well_formed(ids::PAYMENT_INTENT_PREFIX, &cursor) {
        return Err(ApiError::invalid_param(
            param,
            "A cursor must be a payment intent id, as returned in a previous page's `data`.",
        ));
    }
    Ok(Some(cursor))
}

/// The page size: absent means [`DEFAULT_LIMIT`], and anything above
/// [`MAX_LIMIT`] is capped to it rather than refused — a ceiling, not a
/// validation rule. A caller who asks for more gets a full page and
/// `has_more: true`, which is a correct answer to their question; refusing
/// would only make them ask again.
fn parse_limit(raw: Option<&str>) -> Result<i64, ApiError> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_LIMIT);
    };
    let limit: i64 = raw
        .trim()
        .parse()
        .map_err(|_error| ApiError::invalid_param("limit", "`limit` must be a whole number."))?;
    if limit < 1 {
        return Err(ApiError::invalid_param(
            "limit",
            "`limit` must be at least 1.",
        ));
    }
    Ok(limit.min(MAX_LIMIT))
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
fn json_response<T: serde::Serialize>(status: StatusCode, body: &T) -> Result<Response, ApiError> {
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
    ///   deployment*. With today's rails, where every `confirm` ends in the
    ///   adapter's `501`, that permanently burned a key on every confirm a
    ///   merchant made.
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

/// Replays a stored response verbatim.
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
    Ok(value_response(status, body))
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

    /// A `limit` above the ceiling is *capped*, not refused — see
    /// [`parse_limit`]'s doc for why that is the right answer to the
    /// caller's question.
    #[test]
    fn the_page_limit_is_a_ceiling_and_not_a_validation_rule() {
        assert_eq!(parse_limit(None).expect("the default"), DEFAULT_LIMIT);
        assert_eq!(parse_limit(Some("7")).expect("a plain limit"), 7);
        assert_eq!(
            parse_limit(Some("1000")).expect("capped, not refused"),
            MAX_LIMIT
        );
        for raw in ["0", "-3", "many"] {
            let error = parse_limit(Some(raw)).expect_err("refused: {raw}");
            assert_eq!(param_of(&error), Some("limit"), "for {raw:?}");
        }
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

    /// A cursor is checked for *shape* only, and the shapes it refuses are
    /// the ones that would otherwise be answered with an empty page.
    ///
    /// The last assertion is the one that keeps this from becoming an
    /// oracle: a well-formed id this merchant does not own is accepted here
    /// and answered with an empty page by the query, exactly as a foreign
    /// `pi_…` in a `GET /v1/payment_intents/{id}` is answered `404`.
    #[test]
    fn a_cursor_is_checked_for_shape_and_not_for_existence() {
        let real = ids::payment_intent_id();
        assert_eq!(
            validated_cursor(STARTING_AFTER, Some(real.clone())).expect("a real id is accepted"),
            Some(real.clone())
        );
        // Absent, and the empty string a client templating an optional
        // field sends when it has no cursor.
        assert_eq!(
            validated_cursor(STARTING_AFTER, None).expect("absent"),
            None
        );
        for blank in ["", "   "] {
            assert_eq!(
                validated_cursor(ENDING_BEFORE, Some(blank.to_owned()))
                    .expect("blank is absent, not malformed"),
                None
            );
        }
        // Surrounding whitespace survives a copy/paste out of a terminal.
        assert_eq!(
            validated_cursor(STARTING_AFTER, Some(format!("  {real} ")))
                .expect("trimmed, then accepted"),
            Some(real)
        );

        for malformed in [
            "pi_",
            "pi_tooshort",
            "ch_00000000000000000000000x",
            "00000000000000000000000x",
            "PI_00000000000000000000000X",
            "pi_0000000000000000000000 x",
            "1",
        ] {
            let error = validated_cursor(ENDING_BEFORE, Some(malformed.to_owned()))
                .expect_err("a malformed cursor must be named, not answered with an empty page");
            assert_eq!(param_of(&error), Some(ENDING_BEFORE), "for {malformed:?}");
        }

        // Well-formed, and no merchant has ever had it: accepted here on
        // purpose — the query answers an empty page, which is what stops
        // this endpoint from telling one merchant which ids another has.
        assert!(
            validated_cursor(
                STARTING_AFTER,
                Some("pi_00000000000000000000000x".to_owned())
            )
            .expect("shape is all that is checked")
            .is_some()
        );
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
    #[test]
    fn the_recorded_error_kind_is_the_errors_own_code() {
        assert_eq!(
            error_kind(&ProviderError::NotImplemented("mtn_momo::submit")),
            "not_implemented"
        );
    }
}
