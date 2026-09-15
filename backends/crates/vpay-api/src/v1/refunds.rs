//! The Refund resource — `POST /v1/refunds`, `GET /v1/refunds`,
//! `POST /v1/refunds/{id}`, `POST /v1/refunds/{id}/cancel` and
//! `GET /v1/refunds/{id}` (RFC-0003 § 2, issues #45 and #46).
//!
//! # What is real here, and what still is not
//!
//! Read this before any sentence elsewhere that says a refund "works".
//! Everything in this module is real: the row, the reservation, the
//! over-refund refusal, the tenancy, the two events, the four routes. What is
//! **not** real is a payer receiving money, and no amount of code in this file
//! can make it so:
//!
//! * `mtn_momo::refund` is written — MTN's Disbursements `transfer`, since
//!   2026-09-15 — and is WireMock-proven and **rail-unproven**. No deployment
//!   holds a Disbursements subscription key and the product has never been
//!   called from this repository, so a deployment reaching this handler on MTN
//!   gets [`ProviderError::Config`] naming the credential it lacks.
//! * `orange_money::refund` is a declared `ProviderError::NotImplemented`
//!   token. An Orange refund is an outbound transfer and this repository has
//!   no specification for one (RFC-0003 § 5), so nothing is guessed at.
//! * **Nothing settles a `pending` refund.** There is no refund poll ladder —
//!   the port has no refund status read and [`Refunded`] has no status field
//!   (RFC-0003 open question 8) — so a refund this route creates stays
//!   `pending` until an operator or the settlement path moves it. That is why
//!   an `Ok` from the rail does **not** become `succeeded` here; see
//!   [`finish_refund`].
//!
//! `docs/status.md` carries all three. A merchant reading this API's
//! documentation must not be able to conclude from the existence of these
//! routes that money has ever come back.
//!
//! # Why a read shipped before a create did
//!
//! `GET /v1/refunds/{id}` landed on 2026-09-05 (issue #45) while `POST
//! /v1/refunds` was still the nest's honest `404`, because a refund must have
//! an authoritative read the moment one can exist at all —
//! `docs/flows/provider-port.md` calls `query_status` "**the authoritative
//! read**", and `docs/flows/webhooks.md` says delivery is at-least-once and
//! unordered, so a webhook is not a substitute for a read. The create needed
//! RFC-0003's decisions about destinations, capabilities and the ledger, which
//! is Wave 3 — this change.
//!
//! # One renderer, and no eleventh key
//!
//! [`crate::model::RefundObject`] is what all five routes return and what both
//! event types carry in `data.object`, for the reason [`crate::v1::events`]
//! gives about its own: two renderers would let the documented fallback answer
//! a different question from the one the webhook asked. It is ten keys, held
//! by `the_refund_object_is_the_documented_ten_keys`, and **the destination is
//! not one of them**.
//!
//! # The destination never lands anywhere it can outlive this request
//!
//! The payee's MSISDN is a third party's personal data. RFC-0003 rejected
//! carrying it in `metadata` precisely because metadata is out of reach of
//! customer erasure and is inside every signed webhook vpay delivers. This
//! handler therefore:
//!
//! * hands it to the adapter and keeps no copy — there is **no `destination`
//!   column on `refunds`** and this change does not add one, because its
//!   retention is RFC-0003 open question 3 and is **undecided** (the customer
//!   object settled on twelve months; a refund destination has no policy).
//!   Storing it now would be answering a question reserved for the
//!   maintainer, and the erasure path would then owe it a statement;
//! * renders it in no response and in no event body;
//! * logs it **masked** — `+2376••••200`, [`crate::v1::account_holders`]'
//!   own shape and function — and never raw. [`RefundTarget`]'s redacting
//!   `Debug` is what makes that hold for any `{:?}` a future author writes;
//!   this module's masked line is the deliberate exception that doc names.
//!
//! What that costs, stated rather than hidden: vpay cannot tell an operator
//! which payee a refund was sent to. The rail's own records can, addressed by
//! the `provider_reference_id` this handler mints — which is the reconciliation
//! key `docs/flows/crash-safety.md` asks for and is on the row.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Map, Value};
use time::OffsetDateTime;
use uuid::Uuid;

use vpay_core::{Currency, Money, ids};
use vpay_db::{
    ChargeRow, NewRefund, PaymentIntents, RefundListPage, RefundRow, Refunds, Repositories,
    ResponseSubject, TxOutcome, UnitOfWork as _,
};
use vpay_provider::{
    ChargeRef, ProviderAdapter, ProviderConfig, ProviderError, RefundDestination, RefundTarget,
    Refunded,
};

use crate::error::ApiError;
use crate::form::VpayQuery;
use crate::model::{ListObject, RefundObject};
use crate::v1::payment_intents::{ClaimOutcome, PostRequest, json_response};
use crate::v1::{MerchantScope, RailConfig, ResourceConfig, paging};

/// The object type this module speaks about, in the API's own vocabulary.
/// One constant so a `404` for a refund can never be spelled two ways.
const RESOURCE: &str = "refund";

/// The collection's own URL, echoed as `url` on every list page.
const LIST_URL: &str = "/v1/refunds";

/// This resource's cursor vocabulary — `re_…`, per `vpay_core::ids`.
///
/// `pub(crate)` for [`crate::v1::payment_intents::CURSOR`]'s reason: it is
/// what lets a test prove this list refuses a `pi_…` cursor, which is the
/// point of the prefix being a parameter of `validated_cursor` rather than a
/// constant inside it.
pub(crate) const CURSOR: paging::CursorKind = paging::CursorKind {
    prefix: ids::REFUND_PREFIX,
    noun: "a refund id",
};

/// The parameter a destination problem names, in one place so the `400` a
/// merchant's typo earns cannot be spelled two ways.
const DESTINATION_PARAM: &str = "destination";

/// The `type` of the event a create emits.
///
/// Spelled as a constant for `vpay_db::settlement`'s reason: the type is a
/// property of *which transition this is*, and a caller free to choose it
/// could report a cancelled refund as a successful one.
///
/// **This is the first writer of either refund type.** Both have been in
/// `type_is_a_documented_event` since migration `0018` and in
/// `docs/flows/webhooks.md` since before that, with "— nothing" in the
/// "written by" column; migration `0023`'s lockstep rule (the CHECK moves
/// with the code that writes it) was not followed for these two, and this
/// change closes the gap from the other end rather than reopening it.
const EVENT_REFUNDED: &str = "charge.refunded";

/// See [`EVENT_REFUNDED`]. Emitted for every later move of a refund this
/// handler makes: a metadata update, a cancellation, and a rail that refused
/// the instruction.
///
/// Stripe's own split: `charge.refunded` says a charge has a refund against
/// it, `charge.refund.updated` says a refund that already existed changed.
/// A merchant's existing Stripe-shaped handler has a branch for both.
const EVENT_REFUND_UPDATED: &str = "charge.refund.updated";

/// `refunds.reason`'s `reason_length` CHECK (migration `0017`), mirrored here
/// so an over-long reason is a `400` naming the parameter rather than a
/// constraint violation rendered as a `500`.
const REASON_MAX_CHARS: usize = 512;

/// `refunds.failure_raw`'s ceiling (migration `0017`), mirrored for
/// `payment_intents`' reason: a rail whose text runs long must not abort the
/// transaction that records its refusal.
const FAILURE_RAW_MAX_CHARS: usize = 2000;

// ----------------------------------------------------------------- create

/// `POST /v1/refunds`'s fields, as the form decoder produces them.
///
/// `destination` is an untyped map for [`crate::v1::payment_intents`]'
/// `ConfirmParams::payment_method_data` reason: the payee is nested under the
/// *rail's own code* on the wire (`destination[mtn_momo][msisdn]`), so a typed
/// struct here would have to name `mtn_momo` as a field — and `if provider ==
/// "mtn_momo"` outside an adapter crate is exactly what ADR-0002 forbids.
///
/// `amount` is text although it is a number, for [`CreateParams`]' siblings'
/// reason: the wire is form-encoded, so typing it here would hand the "not a
/// number" case to serde, which answers `param: "body"`.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct CreateParams {
    payment_intent: Option<String>,
    amount: Option<String>,
    reason: Option<String>,
    destination: Option<Map<String, Value>>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

/// Redacts `destination`, and it is not belt and braces.
///
/// This struct holds a payee's phone number in a raw `serde_json::Map`, which
/// has no redacting `Debug` of its own — [`RefundTarget`]'s only exists from
/// the moment the adapter has parsed one, and this is the shape *before* that.
/// A `tracing::debug!(?params)` anywhere in this file, now or in five years,
/// would otherwise print a third party's number into an operator's log, which
/// is the exact leak RFC-0003 rejected `metadata.payee_number` to avoid.
/// `crate::v1::customers`' `ValidCreate` carries the same hand-written impl
/// for the same reason.
///
/// The key set is printed and the values are not: "the merchant nominated a
/// payee on `mtn_momo`" is what an operator needs, and the number is not.
impl std::fmt::Debug for CreateParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateParams")
            .field("payment_intent", &self.payment_intent)
            .field("amount", &self.amount)
            .field("reason", &self.reason)
            .field(
                "destination",
                &format_args!(
                    "{:?} [values redacted]",
                    self.destination
                        .as_ref()
                        .map(|map| map.keys().collect::<Vec<_>>())
                ),
            )
            .field("metadata", &self.metadata)
            .finish()
    }
}

/// `POST /v1/refunds`.
///
/// Reads and claims the `Idempotency-Key`, runs [`create_once`], and ends the
/// claim with whatever it answered. The claim is what stops a merchant's retry
/// of a request that already moved money from moving it a second time, and it
/// is the **only** thing that does: unlike a charge, which the
/// `one_charge_per_intent` index makes unrepeatable, nothing in the schema
/// forbids two refunds of one intent — they are a legitimate thing a merchant
/// does.
///
/// # Errors
///
/// [`ApiError::InvalidParam`] for a missing or unusable `Idempotency-Key`,
/// [`ApiError::IdempotencyKeyInFlight`] for a key still in flight, and
/// everything [`create_once`] can answer.
pub(crate) async fn create(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    State(adapters): State<Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>>,
    scope: MerchantScope,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let params: CreateParams = post.form().await?;

    // The claim runs **first, before any rule** — `crate::v1::customers::create`'s
    // ordering, and its reason: a replay must answer whatever the original
    // answered, whatever has changed since.
    //
    // This is deliberately **not** `crate::v1::payment_intents::confirm`'s
    // ordering, and the difference is the whole of this comment. That handler
    // refuses two Stripe parameters before claiming, and its own comment says
    // why that is safe: the check reads the **body alone**, so a genuine
    // replay — whose body is byte for byte the one that was accepted — can
    // never be shadowed by it. Every refusal below reads **mutable rows**,
    // and the intent's own counters among them. Resolving first would mean a
    // merchant whose `201` was lost to a timeout, retrying under the same key
    // against an intent that now has nothing left to refund *because their
    // own first request took it*, is answered `409` instead of the refund
    // they already have — and with no `re_…` in the envelope, no way to find
    // it. That is the exact case an idempotency key exists for.
    //
    // Measured 2026-09-16 rather than reasoned about: with the two swapped,
    // `a_replayed_key_answers_the_stored_refund_even_when_the_intent_has_moved_on`
    // fails and every other case in this repository passes — including the
    // other replay case, which replays against an intent nothing changed.
    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    // Which puts every refusal here, and every one of them **releases the
    // key**: nothing is written before [`create_once`] opens its transaction,
    // so re-executing a corrected retry under the same key is exactly
    // equivalent to this request never having been made.
    // `crate::v1::customers::create`'s carve-out, applied for its reason —
    // and it is why the release is here rather than left to
    // [`PostRequest::finish`], which would store the refusal and replay it at
    // a merchant who has since corrected their request.
    let target =
        match resolve_target(repositories.as_ref(), &config, &adapters, &scope, &params).await {
            Ok(target) => target,
            Err(error) => {
                post.release(repositories.as_ref(), &scope, claim_id).await;
                return Err(error);
            }
        };

    let outcome = create_once(repositories.as_ref(), &scope, target).await;

    // `Verbatim`: a refund object carries no payer identifier at all — ten
    // keys, none of them a person — so there is nothing here for an erasure
    // to overtake, unlike `/v1/customers`' stored responses (issue #111).
    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Verbatim,
    )
    .await
}

/// Every step of a create, in the order RFC-0003 § 3 puts them.
///
/// The order is the safety property, and each step is where it is for a
/// stated reason:
///
/// 1. the intent, **merchant-scoped**, so every later refusal may speak
///    plainly about an object this caller is already entitled to see;
/// 2. the charge, which is what names the rail — a refund goes back the way
///    the money came, and the merchant does not get to choose;
/// 3. the rail's capabilities: does it refund at all, does it do partials,
///    and does it need a payee. All three read [`vpay_provider::Capabilities`]
///    and none reads a provider code (ADR-0002);
/// 4. the destination, parsed **by the adapter** from the rail-scoped inner
///    map, and name-matched through the rail's account-holder lookup where
///    there is one;
/// 5. the row, its reservation and `charge.refunded`, in **one** transaction,
///    with a `provider_reference_id` minted before any of it;
/// 6. the rail, addressed by *that* reference;
/// 7. the answer.
///
/// Everything that can refuse the request is in steps 1 to 4, before anything
/// is written — so a refused refund costs no row, no reservation and no event.
///
/// # Errors
///
/// `404` for an intent this scope cannot see; `400` for a parameter this
/// deployment or this rail will not take; `409` for an intent with nothing
/// left to refund (the `no_over_refund` CHECK's own refusal, never a
/// read-then-compare); the rail's own classification for anything the adapter
/// answers that proves it never took the instruction.
async fn create_once(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    target: RefundTargetResolved<'_>,
) -> Result<Response, ApiError> {
    // Minted before the row, and the row before the rail. Both halves of
    // `docs/flows/crash-safety.md`'s rule: a process that dies mid-call
    // leaves a reference to reconcile by, and the reference is the refund's
    // own — never the charge's. RFC-0003 open question 7, and the whole of
    // what `the_transfer_is_addressed_by_the_reference_the_core_supplied`
    // pins on the adapter's side.
    let reference = Uuid::new_v4();
    let new = NewRefund {
        id: ids::refund_id(),
        payment_intent_id: target.intent.id.clone(),
        amount: target.amount,
        reason: target.reason.clone(),
        metadata: Value::Object(target.metadata.clone()),
        provider_reference_id: reference,
    };

    let row = write_pending_refund(repositories, &target.intent, &new).await?;

    let attempt = repositories
        .insert_pending(
            &target.charge.id,
            &target.charge.provider_code,
            "refund",
            reference,
            1,
        )
        .await?;

    let refunded = refund_at_rail(&target, &row, reference).await;

    finish_refund(
        repositories,
        scope,
        &target,
        row,
        reference,
        attempt,
        refunded,
    )
    .await
}

/// Everything steps 1 to 4 resolve, owned because it outlives the borrows it
/// was read from.
struct RefundTargetResolved<'a> {
    intent: vpay_db::PaymentIntentRow,
    charge: ChargeRow,
    rail: &'a RailConfig,
    adapter: &'a dyn ProviderAdapter,
    amount: i64,
    reason: Option<String>,
    metadata: Map<String, Value>,
    /// `Some` exactly when the rail declares [`RefundDestination::Required`]
    /// — the invariant [`ProviderAdapter::refund`] is entitled to, and which
    /// [`resolve_destination`] is the only producer of.
    destination: Option<RefundTarget>,
}

/// Steps 1 to 4: everything that can refuse the request, before anything is
/// written.
async fn resolve_target<'a>(
    repositories: &dyn Repositories,
    config: &'a ResourceConfig,
    adapters: &'a BTreeMap<String, Box<dyn ProviderAdapter>>,
    scope: &MerchantScope,
    params: &CreateParams,
) -> Result<RefundTargetResolved<'a>, ApiError> {
    let intent_id = params
        .payment_intent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::invalid_param(
                "payment_intent",
                "`payment_intent` is required: a refund is requested against the payment \
                 intent whose money is coming back.",
            )
        })?;

    // Merchant-scoped, and the `404` folds "not yours" into "no such intent"
    // exactly as `GET /v1/payment_intents/{id}` does. Everything below this
    // line may speak plainly, because the caller has been shown to be
    // entitled to this object.
    let intent = PaymentIntents::get_for_merchant(repositories, scope.merchant_id(), intent_id)
        .await?
        .ok_or_else(|| ApiError::NotFound {
            resource: "payment_intent",
            id: intent_id.to_owned(),
        })?;

    // The rail is the charge's, never the request's. A merchant does not get
    // to choose where money goes back through: it goes back the way it came,
    // and an intent with no charge has captured nothing to return.
    let charge = repositories
        .get_for_intent(&intent.id)
        .await?
        .ok_or_else(|| ApiError::Conflict {
            message: format!(
                "Payment intent {} has no charge to refund: nothing was ever captured on it.",
                intent.id
            ),
        })?;

    let rail = config.rail(&charge.provider_code).ok_or_else(|| {
        // Ours, not the caller's: the charge names a rail this deployment's
        // YAML no longer carries, so there is no credential to call it with.
        ApiError::Config(vpay_config::ConfigError::ProviderWithoutAdapter {
            code: charge.provider_code.clone(),
            linked: adapters.keys().cloned().collect::<Vec<_>>().join(", "),
        })
    })?;
    let adapter = adapters.get(&charge.provider_code).ok_or_else(|| {
        ApiError::Config(vpay_config::ConfigError::ProviderWithoutAdapter {
            code: charge.provider_code.clone(),
            linked: adapters.keys().cloned().collect::<Vec<_>>().join(", "),
        })
    })?;

    // `rail`, not `enabled_rail`. `enabled` governs whether a rail may take
    // *new* charges; a deployment that has switched a rail off still owes
    // every payer it already charged their money back, and refusing here
    // would make an operator's routing decision into a merchant's refund
    // outage.
    let capabilities = adapter.capabilities();
    if !capabilities.supports_refunds {
        return Err(ApiError::invalid_param(
            "payment_intent",
            "The payment method this intent was charged on does not support refunds.",
        ));
    }

    let amount = resolve_amount(params.amount.as_deref(), &intent, capabilities)?;
    let destination = resolve_destination(
        adapter.as_ref(),
        &charge.provider_code,
        params.destination.as_ref(),
        capabilities.refund_destination,
    )?;

    if let Some(destination) = destination.as_ref() {
        verify_registered_holder(adapter.as_ref(), destination, &rail.provider_config()).await?;
    }

    let reason = validated_reason(params.reason.as_deref())?;
    let metadata = crate::v1::invoices::validated_metadata(&params.metadata)?;

    Ok(RefundTargetResolved {
        intent,
        charge,
        rail,
        adapter: adapter.as_ref(),
        amount,
        reason,
        metadata,
        destination,
    })
}

/// `amount`, or the whole of what is left when it is omitted.
///
/// # This read does **not** decide the over-refund, and must not
///
/// `no_over_refund` (migration `0003`) is what refuses an over-refund, in the
/// `UPDATE` that takes the reservation, under concurrency, against the
/// committed total — RFC-0003 § 3 step 3, and `vpay_db::Refunds::create`'s own
/// doc says there is deliberately no read-then-compare anywhere on that path.
/// What this function reads the counters for is a different question with no
/// other answer: "the merchant sent no `amount`, so how much is a *full*
/// refund?". Two concurrent full refunds of one intent still both resolve to
/// the same number here and the second is still refused by the CHECK — which
/// is the case that would be silently wrong if this were a guard.
///
/// The one refusal it does own is a resolved amount of zero or less, because
/// `amount_positive` would otherwise answer it as a `500`-shaped constraint
/// violation for a request that is simply asking for nothing.
fn resolve_amount(
    raw: Option<&str>,
    intent: &vpay_db::PaymentIntentRow,
    capabilities: vpay_provider::Capabilities,
) -> Result<i64, ApiError> {
    let remaining = intent
        .amount
        .saturating_sub(intent.amount_refunded)
        .saturating_sub(intent.amount_refund_pending);

    let amount = match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => remaining,
        Some(raw) => raw.parse::<i64>().map_err(|_error| {
            ApiError::invalid_param(
                "amount",
                "`amount` must be a whole number of minor units — no decimal point, no \
                 currency symbol.",
            )
        })?,
    };

    if amount <= 0 {
        return Err(ApiError::Conflict {
            message: format!("Payment intent {} has nothing left to refund.", intent.id),
        });
    }

    // The capability, never a provider code. A rail that refunds only in full
    // is told so by `supports_partial_refunds`, and "in full" is measured
    // against the intent's own amount rather than against what is left —
    // otherwise a rail with no partial support would accept a second refund
    // for the remainder, which is a partial refund by another name.
    if !capabilities.supports_partial_refunds && amount != intent.amount {
        return Err(ApiError::invalid_param(
            "amount",
            "The payment method this intent was charged on refunds in full only; omit \
             `amount` or send the intent's whole amount.",
        ));
    }

    Ok(amount)
}

/// The destination, checked against the rail's capability and parsed by the
/// rail's own adapter.
///
/// # The capability is the check, and a provider code is never read
///
/// ADR-0002: the core reads [`RefundDestination`] and branches on *that*. A
/// `Required` rail with no destination and an [`Origin`](RefundDestination::Origin)
/// rail sent one are both the merchant's mistake, both `400`, and both named
/// on `destination`. Accepting a destination on an `Origin` rail and dropping
/// it would tell a merchant their nominated payee had been honoured when the
/// rail was never told about it — which is the mutation
/// `an_origin_rail_refuses_a_destination` exists to catch.
///
/// # The rail code is stripped before the adapter sees the map
///
/// [`ProviderAdapter::parse_destination`] is documented to take the
/// **rail-scoped inner map** — the value of `destination[<code>]`, with the
/// code already gone — and nothing but this function enforces that. The outer
/// key is vpay's own envelope, exactly as `payment_method_data[<code>]` is on
/// the confirm path, so a `destination` naming no rail or naming one as
/// something that is not an object is refused here and never reaches an
/// adapter. `the_outer_destination_map_is_refused_rather_than_misread` pins
/// the direction the mistake fails in on both rails; the test below is the
/// half that proves this side strips it.
///
/// # Why a `Malformed` is translated rather than forwarded
///
/// Because this is the one call site where **no rail answered anything**.
/// `ProviderError::Malformed` is the only honest variant the port offers for
/// "this map is not a payee" and its *classification* is written for a rail
/// that answered gibberish: forwarded unchanged it becomes a `502`, a
/// `Retry::AfterBackoff` an SDK will honour, the sentence "The payment rail is
/// temporarily unavailable" and a charge against the rail's error budget — all
/// for a typo in the merchant's own request, with the parameter name the
/// adapter carefully put in `context` reaching the operator's log and never
/// the integrator it was written for. `a_malformed_destination_is_classified_as_a_rail_fault`
/// measures that classification, and the decision to translate here is
/// recorded on [`ProviderAdapter::parse_destination`] § "The caller must
/// translate this error, not forward it".
///
/// The adapter's `context` is **not** echoed to the merchant: an adapter is
/// forbidden from putting the number in it, but a `400` body is not the place
/// to start trusting that. The sentence names the parameter and the shape,
/// which is what an integrator can act on.
fn resolve_destination(
    adapter: &dyn ProviderAdapter,
    code: &str,
    destination: Option<&Map<String, Value>>,
    capability: RefundDestination,
) -> Result<Option<RefundTarget>, ApiError> {
    let sent = destination.filter(|map| !map.is_empty());

    match (capability, sent) {
        (RefundDestination::Origin, None) => Ok(None),
        (RefundDestination::Origin, Some(_)) => Err(ApiError::invalid_param(
            DESTINATION_PARAM,
            "This payment method returns a refund to the instrument that paid, so it takes \
             no `destination`.",
        )),
        (RefundDestination::Required, None) => Err(ApiError::invalid_param(
            DESTINATION_PARAM,
            "This payment method returns money by transfer, so a refund needs a payee: send \
             it as `destination[<payment_method_type>][…]`.",
        )),
        (RefundDestination::Required, Some(map)) => {
            let inner = map.get(code).and_then(Value::as_object).ok_or_else(|| {
                ApiError::invalid_param(
                    DESTINATION_PARAM,
                    "`destination` must name the payment method this intent was charged on, \
                     as `destination[<payment_method_type>][…]`.",
                )
            })?;

            adapter.parse_destination(inner).map(Some).map_err(|error| {
                // Logged whole for an operator — the adapter's `context`
                // names the key that was wrong and is forbidden from
                // carrying the value — and answered as the caller error it
                // is. See this function's docs.
                tracing::debug!(
                    rail = %code,
                    %error,
                    "a refund destination was refused by the rail's own parser"
                );
                ApiError::invalid_param(
                    DESTINATION_PARAM,
                    "`destination` is not a payee this payment method can be given. A \
                     mobile-money payee is sent as \
                     `destination[<payment_method_type>][msisdn]` in international form, \
                     starting with `+`.",
                )
            })
        }
    }
}

/// Refuses a payee the rail has no record of, where the rail can be asked.
///
/// This is the caller [issue #47](https://github.com/vaam-apps/vpay/issues/47)
/// built `account_holder_name` for, and the three-way answer is the whole of
/// why it exists (`docs/flows/account-holder-lookup.md`):
///
/// * `Ok(Some(_))` — the rail named a holder, so the number is a real
///   registered account and the transfer may be attempted. **The name is not
///   compared to anything, not stored and not logged**: vpay holds no
///   verified name for the buyer to compare it against, and inventing a
///   comparison would be a rule with no input. Matching the holder's name
///   against a buyer's own is the *merchant's* job, which is what the route
///   `GET /v1/account_holders` is for;
/// * `Ok(None)` — the rail answered and has no such account. That is a fact
///   about the **number**, it is the merchant's to fix, and it is a `400`;
/// * `Err(..)` — nobody asked, or the rail could not answer. That is a fact
///   about the **lookup**, it is ours, and it keeps the error's own
///   classification (`502`/`500`). Collapsing it into the row above would
///   tell an integrator that a real person's real account does not exist.
///
/// A rail with no lookup skips this entirely, on the capability and never on
/// a code (ADR-0002) — `orange_money` is exactly that case, and RFC-0003 § 1
/// says so: on Orange the destination is accepted unverified.
async fn verify_registered_holder(
    adapter: &dyn ProviderAdapter,
    destination: &RefundTarget,
    config: &ProviderConfig,
) -> Result<(), ApiError> {
    if !adapter.capabilities().supports_account_holder_lookup {
        return Ok(());
    }

    match adapter
        .account_holder_name(destination.msisdn(), config)
        .await?
    {
        Some(_holder) => Ok(()),
        None => Err(ApiError::invalid_param(
            DESTINATION_PARAM,
            "The nominated payee is not a registered account on this payment method. Check \
             the number with the payer before sending the refund again.",
        )),
    }
}

/// The merchant's `reason`, bounded by the column's own CHECK.
fn validated_reason(reason: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(reason) = reason.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if reason.chars().count() > REASON_MAX_CHARS {
        return Err(ApiError::invalid_param(
            "reason",
            format!("`reason` is longer than {REASON_MAX_CHARS} characters."),
        ));
    }
    Ok(Some(reason.to_owned()))
}

/// Step 5: the row, its reservation and `charge.refunded`, in one transaction.
///
/// The event is written beside the write it reports, which is the only shape
/// this repository has (`docs/flows/webhooks.md`'s "TX 1"): an event committed
/// apart from its transition is either a webhook for something that did not
/// happen or a transition no merchant hears about.
///
/// # Why `charge.refunded` is emitted for a refund that is only `pending`
///
/// Because the transition it reports is "this charge now has a refund against
/// it", which has happened, and because there is no second moment vpay can
/// promise to reach: nothing settles a refund (RFC-0003 open question 8), so
/// an event held back until `succeeded` would be an event that never arrives.
/// The body carries `status: "pending"` and the merchant reads it — the object
/// is the whole answer, which is why both event types carry the same renderer.
///
/// # Errors
///
/// `404` if the intent is not this merchant's `succeeded` one — the fold
/// `vpay_db::Refunds::create` documents, re-answered here as the same `404`
/// [`resolve_target`] would have given, because between that read and this
/// write the intent may legitimately have moved; `409` (`over_refund`) from
/// the CHECK; [`ApiError::Db`] otherwise.
async fn write_pending_refund(
    repositories: &dyn Repositories,
    intent: &vpay_db::PaymentIntentRow,
    new: &NewRefund,
) -> Result<RefundRow, ApiError> {
    let outcome: TxOutcome<Option<RefundRow>> = repositories
        .transaction(|tx| {
            Box::pin(async move {
                let Some(row) = tx.create_refund_in_tx(&intent.merchant_id, new).await? else {
                    return Ok::<_, ApiError>(TxOutcome::Abandon(None));
                };

                tx.insert_in_tx(&refund_event(EVENT_REFUNDED, intent, &row)?)
                    .await?;

                Ok::<_, ApiError>(TxOutcome::Commit(Some(row)))
            })
        })
        .await?;

    outcome.into_inner().ok_or_else(|| ApiError::NotFound {
        resource: "payment_intent",
        id: new.payment_intent_id.clone(),
    })
}

/// Step 6: the rail, addressed by the **refund's** reference.
///
/// # The reference is the whole of this function's reason to exist
///
/// [`ProviderAdapter::refund`] takes one [`ChargeRef`], which carries one
/// reference, and the adapter addresses the rail with whatever it is given —
/// MTN's `X-Reference-Id` and the body's `externalId`, both. If this handler
/// passed the **charge's** reference, the second partial refund of one charge
/// would reuse a reference the rail has already seen, be answered `409
/// RESOURCE_ALREADY_EXIST`, and — correctly, under the crash-retry invariant
/// that mapping exists for — be reported **accepted with no money moved**. A
/// merchant told a refund happened, a payer who received nothing, and no error
/// anywhere. RFC-0003 open question 7 is that bug written down before it was
/// written in; `the_transfer_is_addressed_by_the_reference_the_core_supplied`
/// pins the adapter's half and
/// `two_partial_refunds_of_one_charge_carry_two_references` pins this one.
///
/// `payer_ref` is the charge's — it is what the rail knows this movement is
/// unwinding — and `ref_extra` is empty because a refund carries no key
/// material from a submit. `return_url` is `None`: no payer's browser is
/// involved in giving money back.
async fn refund_at_rail(
    target: &RefundTargetResolved<'_>,
    row: &RefundRow,
    reference: Uuid,
) -> Result<Refunded, ProviderError> {
    let currency = Currency::from_code(&row.currency_code)
        .map_err(|error| ProviderError::Config(error.to_string()))?;
    let amount = Money::new(row.amount, currency)
        .map_err(|error| ProviderError::Config(error.to_string()))?;

    let charge_ref = ChargeRef {
        // The REFUND's reference. See this function's docs.
        reference_id: reference,
        amount,
        payer_ref: target.charge.payer_ref.clone(),
        ref_extra: BTreeMap::new(),
        return_url: None,
    };

    // The one place this handler says anything about the payee, and it says
    // it masked. `RefundTarget`'s redacting `Debug` makes every other
    // rendering safe by default; this line is the deliberate exception its
    // doc comment names, and it is the same mask `charges.payer_ref_masked`
    // is documented to hold.
    tracing::info!(
        refund_id = %row.id,
        payment_intent_id = %row.payment_intent_id,
        rail = %target.charge.provider_code,
        reference_id = %reference,
        destination = target
            .destination
            .as_ref()
            .map(|destination| crate::v1::account_holders::masked(destination.msisdn()))
            .unwrap_or_else(|| "none (this rail refunds to origin)".to_owned()),
        "instructing a rail to return money"
    );

    target
        .adapter
        .refund(
            &charge_ref,
            amount,
            target.destination.as_ref(),
            &target.rail.provider_config(),
        )
        .await
}

/// Step 7: what came back.
///
/// # An `Ok` is an acceptance and is **not** a settlement
///
/// The refund stays `pending`. [`Refunded`] has no status field and the port
/// has no refund status read, so the strongest thing an adapter can mean by
/// `Ok` is "the rail took the instruction" — on the one rail that implements
/// this it is literally so, because MTN's `transfer` answers `202 ACCEPTED`
/// with an empty body and its outcome is read back from a call vpay does not
/// make. Writing `succeeded` here would tell a merchant money moved on the
/// strength of a response that did not say so
/// ([`ProviderAdapter::refund`] § "An `Ok` is an *acceptance*", RFC-0003 open
/// question 8, which stays open because closing it needs a refund poll ladder
/// that does not exist).
///
/// # Which errors release the reservation, and which deliberately do not
///
/// The question every arm below answers is one thing: **did the rail take the
/// instruction?**
///
/// * It provably did not — the adapter refused before or instead of a call
///   ([`ProviderError::NotImplemented`], [`ProviderError::Unsupported`],
///   [`ProviderError::Config`]) or the rail itself refused
///   ([`ProviderError::Rejected`]). The refund is `failed`, the reservation
///   goes back, `charge.refund.updated` says so, and the merchant gets the
///   error's own classification. Retrying is then safe, because nothing is in
///   flight.
/// * It may have — a transport failure or an answer that could not be parsed
///   ([`ProviderError::Transport`], [`ProviderError::Malformed`], and any
///   variant a future port adds). **Nothing is released and nothing is
///   failed.** The refund stays `pending` with its reservation held, which is
///   the state that stops a merchant's retry from paying the payee twice, and
///   the response is the `201` carrying that `pending` object — the same
///   answer an `Ok` gets, because the two states are genuinely the same one:
///   an instruction given, an outcome unknown. It is logged at `error` level
///   with the reference an operator reconciles by.
///
/// The alternative — failing the refund and releasing the reservation
/// whenever the rail did not clearly say yes — is the money bug: a merchant
/// retries, a second transfer goes out, and the payee is paid twice for one
/// refund. This asymmetry is deliberate and is why the fallthrough arm is the
/// cautious one.
///
/// # Errors
///
/// Whatever the adapter answered, for the arms that fail the refund;
/// [`ApiError::Db`] if the bookkeeping write fails.
async fn finish_refund(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    target: &RefundTargetResolved<'_>,
    row: RefundRow,
    reference: Uuid,
    attempt: i64,
    refunded: Result<Refunded, ProviderError>,
) -> Result<Response, ApiError> {
    let error = match refunded {
        Ok(refunded) => {
            repositories
                .record_response(
                    attempt,
                    Some(vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT),
                    None,
                )
                .await?;
            if refunded.fee.is_some() {
                // `refunds.fee` has no writer (issue #46, `docs/status.md`),
                // and this handler is not it: writing the column is a
                // settlement-path change, and a fee reported by a rail that
                // has never been called is not a number to start persisting
                // on the strength of a stub. Logged so the day it is ever
                // populated is visible rather than silent.
                tracing::warn!(
                    refund_id = %row.id,
                    "a rail reported a refund fee and nothing in this repository stores one; \
                     `refund.fee` stays null"
                );
            }
            return json_response(StatusCode::CREATED, &RefundObject::try_from(&row)?);
        }
        Err(error) => error,
    };

    match &error {
        // The rail was never asked, or answered a decision. Either way the
        // instruction is not in flight, so the reservation must go back —
        // an intent holding a reservation for a refund that will never
        // happen cannot be refunded again up to its own amount.
        ProviderError::Rejected { code, message } => {
            repositories
                .record_response(
                    attempt,
                    Some(vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT),
                    Some("rejected"),
                )
                .await?;
            fail_with_event(repositories, scope, target, &row, *code, message).await?;
        }
        ProviderError::NotImplemented(_)
        | ProviderError::Unsupported
        | ProviderError::Config(_) => {
            repositories
                .record_response(attempt, None, Some(error_kind(&error)))
                .await?;
            fail_with_event(
                repositories,
                scope,
                target,
                &row,
                vpay_core::FailureCode::ProviderError,
                &error.to_string(),
            )
            .await?;
        }
        // Neither of these means the rail refused; both mean vpay does not
        // know what it did with the instruction. Matched by name rather than
        // caught by a `_`, so a seventh `ProviderError` variant is a compile
        // error here and somebody has to decide which side of this line it
        // falls on. See this function's docs for why nothing is released,
        // and why the merchant is answered with the pending refund.
        ProviderError::Transport { .. } | ProviderError::Malformed { .. } => {
            repositories
                .record_response(attempt, None, Some(error_kind(&error)))
                .await?;
            tracing::error!(
                refund_id = %row.id,
                payment_intent_id = %row.payment_intent_id,
                rail = %target.charge.provider_code,
                reference_id = %reference,
                %error,
                "a refund instruction was sent and its outcome is unknown; the refund stays \
                 pending with its reservation held, and nothing in this repository will move \
                 it — reconcile it against the rail by its provider_reference_id"
            );
            return json_response(StatusCode::CREATED, &RefundObject::try_from(&row)?);
        }
    }

    Err(ApiError::from(error))
}

/// Moves a refund to `failed` and emits `charge.refund.updated`, in one
/// transaction.
///
/// The row is re-read under the same transaction rather than projected from
/// what was written: `vpay_db::settlement`'s fail statement returns the
/// narrow settlement projection (id, intent, amount) by design, and the event
/// body must be the wire object as the database now holds it — a projection
/// of what this code believes it wrote is a second implementation of the
/// write, which is the mistake `vpay_api::v1::invoices` records at length.
async fn fail_with_event(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    target: &RefundTargetResolved<'_>,
    row: &RefundRow,
    code: vpay_core::FailureCode,
    raw: &str,
) -> Result<(), ApiError> {
    let raw = bounded(raw, FAILURE_RAW_MAX_CHARS);
    let intent = &target.intent;
    let refund_id = row.id.clone();

    repositories
        .transaction(move |tx| {
            let raw = raw.clone();
            let refund_id = refund_id.clone();
            Box::pin(async move {
                let failed = tx
                    .fail_refund_in_tx(&refund_id, code.as_str(), &raw, OffsetDateTime::now_utc())
                    .await?;
                if failed.is_none() {
                    // Not `pending` any more: something else settled it
                    // between the rail's answer and this write. Nothing is
                    // written and no event is emitted — an event for a
                    // transition that did not happen is worse than none.
                    return Ok::<_, ApiError>(TxOutcome::Abandon(()));
                }

                let stored = tx
                    .lock_refund_for_update(scope.merchant_id(), &refund_id)
                    .await?
                    .ok_or_else(|| {
                        ApiError::Internal(format!(
                            "refund {refund_id} was failed in this transaction and then could \
                             not be read back in it"
                        ))
                    })?;

                tx.insert_in_tx(&refund_event(EVENT_REFUND_UPDATED, intent, &stored)?)
                    .await?;

                Ok::<_, ApiError>(TxOutcome::Commit(()))
            })
        })
        .await?;

    Ok(())
}

// ----------------------------------------------------------------- update

/// `POST /v1/refunds/{id}`'s fields.
///
/// `metadata` is the only one vpay acts on, as Stripe. The other four are
/// here **so they can be refused**: a merchant who sends `amount=100` to
/// "correct" a refund must not be answered `200` with the original amount and
/// left to discover the difference in their settlement statement. Stripe
/// answers `400` for an unknown parameter on this endpoint; vpay names the
/// parameter it will not take, which is the same answer with a better
/// `param`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct UpdateParams {
    metadata: Option<BTreeMap<String, String>>,
    amount: Option<String>,
    reason: Option<String>,
    payment_intent: Option<String>,
    destination: Option<Map<String, Value>>,
}

impl UpdateParams {
    /// The four refusals, in the order a caller would most likely trip them.
    fn reject_unsupported(&self) -> Result<(), ApiError> {
        for (sent, param) in [
            (self.amount.is_some(), "amount"),
            (self.reason.is_some(), "reason"),
            (self.payment_intent.is_some(), "payment_intent"),
            (self.destination.is_some(), DESTINATION_PARAM),
        ] {
            if sent {
                return Err(ApiError::invalid_param(
                    param,
                    format!(
                        "`{param}` cannot be changed after a refund is created. Only \
                         `metadata` may be updated; cancel the refund while it is pending \
                         and create a new one instead."
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// `POST /v1/refunds/{id}` — the metadata update.
///
/// # Errors
///
/// `404` for a refund this merchant has none of, `400` for a parameter that
/// is not `metadata`, and everything the claim can answer.
pub(crate) async fn update(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let params: UpdateParams = post.form().await?;
    // Before the claim, where the body is decoded — `crate::v1::payment_intents::confirm`'s
    // ordering and its reason: a refused update stores nothing and leaves the
    // key unspent, so the corrected retry under it is a fresh request rather
    // than a replay of the refusal.
    params.reject_unsupported()?;

    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = update_once(repositories.as_ref(), &scope, &id, params.metadata).await;

    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Verbatim,
    )
    .await
}

/// The update itself: the merge under the row's lock, the write and the event
/// in one transaction.
///
/// # Why the row is locked
///
/// `metadata` is merged key-wise (Stripe's contract), so the value written is
/// a function of the stored one. A pooled read left `POST /v1/customers/{id}`
/// with a window in which two concurrent updates each adding one key lost one
/// of them, and the event the merchant then received described a state the
/// database did not hold (issue #66). The same merge here takes the same lock.
///
/// # A bodiless update writes nothing and emits nothing
///
/// Stripe answers such a request with the object unchanged; so does vpay, and
/// an event about a change that did not happen is a webhook a merchant has to
/// work out how to ignore.
async fn update_once(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    id: &str,
    metadata: Option<BTreeMap<String, String>>,
) -> Result<Response, ApiError> {
    let row = lookup(repositories, scope.merchant_id(), id)
        .await?
        .ok_or_else(|| not_found(id))?;
    let intent = owning_intent(repositories, scope, &row).await?;

    let Some(sent) = metadata else {
        return json_response(StatusCode::OK, &RefundObject::try_from(&row)?);
    };

    let updated: TxOutcome<Option<RefundRow>> = repositories
        .transaction(|tx| {
            let sent = sent.clone();
            Box::pin(async move {
                let Some(locked) = tx.lock_refund_for_update(scope.merchant_id(), id).await? else {
                    // Erased or otherwise gone between the read above and
                    // this lock. The `404` is the caller's answer and nothing
                    // is written.
                    return Ok::<_, ApiError>(TxOutcome::Abandon(None));
                };

                let merged = crate::v1::invoices::merged_metadata(&locked.metadata, &sent)?;
                let written = tx
                    .update_refund_metadata_in_tx(
                        scope.merchant_id(),
                        id,
                        &Value::Object(merged),
                        OffsetDateTime::now_utc(),
                    )
                    .await?
                    .ok_or_else(|| {
                        ApiError::Internal(format!(
                            "refund {id} was locked in this transaction and then matched no \
                             row in the same one"
                        ))
                    })?;

                tx.insert_in_tx(&refund_event(EVENT_REFUND_UPDATED, &intent, &written)?)
                    .await?;

                Ok::<_, ApiError>(TxOutcome::Commit(Some(written)))
            })
        })
        .await?;

    let written = updated.into_inner().ok_or_else(|| not_found(id))?;
    json_response(StatusCode::OK, &RefundObject::try_from(&written)?)
}

// ----------------------------------------------------------------- cancel

/// `POST /v1/refunds/{id}/cancel`.
///
/// # The state machine is the `WHERE` clause
///
/// `vpay_db`'s cancel carries `AND status = 'pending'` in the statement, so a
/// refund that settles between the read below and the write is refused by the
/// database rather than by a check beside it. The read that precedes it is a
/// *diagnosis* — it is what tells a `404` from a `409` — and never what
/// decides the write.
///
/// # Errors
///
/// `404` for a refund this merchant has none of; `409` for one that is no
/// longer `pending`; everything the claim can answer.
pub(crate) async fn cancel(
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

    let outcome = cancel_once(repositories.as_ref(), &scope, &id).await;

    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Verbatim,
    )
    .await
}

/// The cancel itself: the compare-and-swap, the released reservation and
/// `charge.refund.updated`, in one transaction.
async fn cancel_once(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    id: &str,
) -> Result<Response, ApiError> {
    let row = lookup(repositories, scope.merchant_id(), id)
        .await?
        .ok_or_else(|| not_found(id))?;
    let intent = owning_intent(repositories, scope, &row).await?;

    let canceled: TxOutcome<Option<RefundRow>> = repositories
        .transaction(|tx| {
            Box::pin(async move {
                let Some(canceled) = tx
                    .cancel_refund_in_tx(scope.merchant_id(), id, OffsetDateTime::now_utc())
                    .await?
                else {
                    return Ok::<_, ApiError>(TxOutcome::Abandon(None));
                };

                tx.insert_in_tx(&refund_event(EVENT_REFUND_UPDATED, &intent, &canceled)?)
                    .await?;

                Ok::<_, ApiError>(TxOutcome::Commit(Some(canceled)))
            })
        })
        .await?;

    let Some(canceled) = canceled.into_inner() else {
        // The read above found the refund and found it this merchant's, so
        // the statement's only other guard is the one that refused: it is no
        // longer `pending`. The status named is the one the read saw, which
        // is the most useful thing that can honestly be said — it may have
        // moved again since, and the merchant re-reads either way.
        return Err(ApiError::Conflict {
            message: format!(
                "A refund can only be canceled while its status is `pending`; this one is \
                 `{}`.",
                row.status
            ),
        });
    };

    json_response(StatusCode::OK, &RefundObject::try_from(&canceled)?)
}

// ------------------------------------------------------------------- list

/// `GET /v1/refunds`'s query parameters — text for [`CreateParams`]' reason.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ListParams {
    limit: Option<String>,
    starting_after: Option<String>,
    ending_before: Option<String>,
    payment_intent: Option<String>,
}

/// `GET /v1/refunds` — this merchant's refunds, newest first.
///
/// The `payment_intent` filter's *shape* is checked for `validated_cursor`'s
/// reason: a `re_…` sent as `payment_intent` (an easy mistake, since both ids
/// are on the object) would otherwise return an empty page with nothing to
/// fix. An id of the right shape that names no intent, or another merchant's,
/// is an empty page and not a `404` — answering otherwise would make the
/// filter an existence oracle for ids this caller cannot see.
pub(crate) async fn list(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    VpayQuery(params): VpayQuery<ListParams>,
) -> Result<Response, ApiError> {
    let page = paging::list_page(
        params.limit.as_deref(),
        params.starting_after,
        params.ending_before,
        CURSOR,
    )?;

    let payment_intent = params
        .payment_intent
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty());
    if let Some(intent) = payment_intent.as_deref()
        && !ids::is_well_formed(ids::PAYMENT_INTENT_PREFIX, intent)
    {
        return Err(ApiError::invalid_param(
            "payment_intent",
            "`payment_intent` must be a PaymentIntent id — `pi_` followed by 24 characters.",
        ));
    }

    let page = RefundListPage {
        limit: page.limit,
        starting_after: page.starting_after,
        ending_before: page.ending_before,
        payment_intent,
    };

    let (rows, has_more) =
        Refunds::list_page(repositories.as_ref(), scope.merchant_id(), &page).await?;
    let data = rows
        .iter()
        .map(RefundObject::try_from)
        .collect::<Result<Vec<_>, _>>()?;

    json_response(StatusCode::OK, &ListObject::new(data, has_more, LIST_URL))
}

// --------------------------------------------------------------- retrieve

/// `GET /v1/refunds/{id}`.
///
/// A foreign merchant's refund id is a `404`, byte for byte the same `404` a
/// nonexistent one gets — `vpay_db::Refunds::get_for_merchant` folds both
/// into `None` on purpose, and the scope is a join onto the owning intent
/// because `refunds` carries no `merchant_id` of its own. Telling the two
/// apart would let anyone holding one credential enumerate which `re_…` ids
/// exist across the whole deployment, which is the same reason
/// `GET /v1/payment_intents/{id}` answers the way it does.
///
/// # Why the prefix is checked, and why it is checked *into the same 404*
///
/// An id that is not `re_…` cannot name a row in `refunds`
/// (`vpay_core::ids::refund_id` is the only minter, and the `id_length`
/// CHECK bounds the rest), so the check saves a database round trip for a
/// caller who pasted a `pi_…`. It answers the identical `404` rather than a
/// `400`, and that is the load-bearing half: `crate::v1::events`'s own
/// `retrieve` explains why a malformed id here is a `404` and not a shape
/// error, and a distinguishable answer would be one more thing this route
/// tells a caller than `/v1/events/{id}` does. The check changes what
/// Postgres is asked, never what the merchant is told.
///
/// # Errors
///
/// [`ApiError::NotFound`] for an id this merchant has no refund under;
/// [`ApiError::Db`] if the read fails; [`ApiError::Internal`] for a row that
/// will not render, which migration `0017`'s `metadata_is_object` CHECK makes
/// unreachable.
pub(crate) async fn retrieve(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let row = lookup(repositories.as_ref(), scope.merchant_id(), &id)
        .await?
        .ok_or_else(|| not_found(&id))?;

    json_response(StatusCode::OK, &RefundObject::try_from(&row)?)
}

// --------------------------------------------------------------- plumbing

/// The read, with the prefix short-circuit in front of it.
///
/// Split out of [`retrieve`] so the short-circuit is a value a unit test can
/// reason about rather than a branch only an HTTP round trip reaches, and so
/// the handler reads as one sentence. The three writes above go through it
/// too, which is what makes "this refund is yours" one answer rather than
/// four.
async fn lookup(
    repositories: &dyn Repositories,
    merchant_id: &str,
    id: &str,
) -> Result<Option<RefundRow>, ApiError> {
    if !id.starts_with(vpay_core::ids::REFUND_PREFIX) {
        return Ok(None);
    }
    Ok(Refunds::get_for_merchant(repositories, merchant_id, id).await?)
}

/// The intent a refund hangs off, read for the one field the refund row does
/// not carry: `livemode`.
///
/// An event's `livemode` comes **from the object and never from configuration**
/// (`vpay_db::NewEvent::livemode`): a deployment's flag can change between
/// emit and delivery, and the event describes what was true when it happened.
/// A refund has no `livemode` column of its own — it is the intent's, through
/// the same foreign key the tenancy join uses — so this read is what makes the
/// rule keepable rather than a `config.livemode()` that looks identical today
/// and is wrong the day a deployment flips.
///
/// `None` is impossible: `payment_intent_id` is a `NOT NULL` foreign key and
/// the caller reached this row through a merchant-scoped join onto that very
/// intent. It is [`ApiError::Internal`] rather than a `404`, because a refund
/// whose intent cannot be read is a broken database and not a missing object.
async fn owning_intent(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    row: &RefundRow,
) -> Result<vpay_db::PaymentIntentRow, ApiError> {
    PaymentIntents::get_for_merchant(repositories, scope.merchant_id(), &row.payment_intent_id)
        .await?
        .ok_or_else(|| {
            ApiError::Internal(format!(
                "refund {} names payment intent {}, which this merchant cannot read",
                row.id, row.payment_intent_id
            ))
        })
}

/// One refund event, rendered from the row as the database holds it.
///
/// Both types go through here so they cannot disagree about what a refund
/// event carries: the same [`RefundObject`] `GET /v1/refunds/{id}` returns,
/// `object_id` the `re_…`, and `livemode` the intent's.
fn refund_event(
    event_type: &str,
    intent: &vpay_db::PaymentIntentRow,
    row: &RefundRow,
) -> Result<vpay_db::NewEvent, ApiError> {
    let object = RefundObject::try_from(row)?;
    let data = serde_json::to_value(&object).map_err(ApiError::internal_serialization)?;

    Ok(vpay_db::NewEvent {
        id: ids::event_id(),
        merchant_id: intent.merchant_id.clone(),
        livemode: intent.livemode,
        event_type: event_type.to_owned(),
        object_id: row.id.clone(),
        data,
    })
}

/// The one `404` this module produces, so the envelope cannot be built two
/// ways with two different `resource` strings.
fn not_found(id: &str) -> ApiError {
    ApiError::NotFound {
        resource: RESOURCE,
        id: id.to_owned(),
    }
}

/// The `error_kind` an attempt row records for a rail error — the operator's
/// free-text field, never a merchant-facing code.
fn error_kind(error: &ProviderError) -> &'static str {
    match error {
        ProviderError::Transport { .. } => "transport",
        ProviderError::Malformed { .. } => "malformed",
        ProviderError::Rejected { .. } => "rejected",
        ProviderError::Config(_) => "config",
        ProviderError::Unsupported => "unsupported",
        ProviderError::NotImplemented(_) => "not_implemented",
    }
}

/// Truncates on a `char` boundary, for `payment_intents`' reason: the CHECK
/// counts characters, and slicing bytes would both mis-measure and panic on a
/// rail that answers in Arabic.
fn bounded(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpay_core::{Classify as _, ProviderFlow};
    use vpay_provider::{AccountHolder, CallbackRef, Capabilities, ChargeStatus, ProviderConfig};

    /// The documentation payee every case here nominates — `basicuserinfo`'s
    /// registered-holder number, reused so the one number in these tests is
    /// one a reader can look up in the WireMock mappings.
    const PAYEE: &str = "+237600000200";

    /// A rail that needs a payee and parses its own destination, which is
    /// what both rails this workspace carries declare.
    #[derive(Debug)]
    struct RequiredRail {
        lookup: Option<Result<Option<AccountHolder>, ProviderError>>,
    }

    /// A rail that returns money to the instrument that paid — a card, a
    /// wallet. vpay carries none today, which is exactly why the capability
    /// check needs one here: without it the `Origin` branch is unreachable
    /// from any test and the check can be deleted with everything still
    /// green.
    #[derive(Debug)]
    struct OriginRail;

    #[async_trait::async_trait]
    impl ProviderAdapter for RequiredRail {
        fn code(&self) -> &'static str {
            "required_rail"
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                flow: ProviderFlow::Push,
                supports_refunds: true,
                supports_partial_refunds: true,
                delivers_callbacks: true,
                requires_ip_allowlist: false,
                supports_account_holder_lookup: self.lookup.is_some(),
                refund_destination: RefundDestination::Required,
            }
        }

        async fn submit(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<vpay_provider::Submitted, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        async fn query_status(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<ChargeStatus, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        fn parse_callback(&self, _body: &[u8]) -> Result<CallbackRef, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        /// The rail's own wire shape, and the whole point of the fake: it
        /// reads `msisdn` out of the **inner** map, so a handler that handed
        /// it the outer one gets `Malformed` rather than a payee.
        fn parse_destination(
            &self,
            raw: &Map<String, Value>,
        ) -> Result<RefundTarget, ProviderError> {
            let msisdn = raw
                .get("msisdn")
                .and_then(Value::as_str)
                .ok_or_else(|| ProviderError::malformed("destination[msisdn] is missing"))?;
            RefundTarget::mobile_money(msisdn)
                .map_err(|error| ProviderError::malformed(error.to_string()))
        }

        async fn account_holder_name(
            &self,
            _msisdn: &str,
            _config: &ProviderConfig,
        ) -> Result<Option<AccountHolder>, ProviderError> {
            match &self.lookup {
                Some(Ok(holder)) => Ok(holder.clone()),
                Some(Err(_)) => Err(ProviderError::transport("the rail could not be reached")),
                None => Err(ProviderError::Unsupported),
            }
        }
    }

    #[async_trait::async_trait]
    impl ProviderAdapter for OriginRail {
        fn code(&self) -> &'static str {
            "origin_rail"
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                flow: ProviderFlow::Push,
                supports_refunds: true,
                supports_partial_refunds: true,
                delivers_callbacks: true,
                requires_ip_allowlist: false,
                supports_account_holder_lookup: false,
                refund_destination: RefundDestination::Origin,
            }
        }

        async fn submit(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<vpay_provider::Submitted, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        async fn query_status(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<ChargeStatus, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        fn parse_callback(&self, _body: &[u8]) -> Result<CallbackRef, ProviderError> {
            Err(ProviderError::Unsupported)
        }
    }

    fn required() -> RequiredRail {
        RequiredRail { lookup: None }
    }

    /// The deployment material an adapter is handed. Nothing here reaches a
    /// network: the fakes above answer without looking at it, and it exists
    /// so [`verify_registered_holder`] can be called with the same shape a
    /// real rail gets.
    fn provider_config() -> ProviderConfig {
        ProviderConfig {
            base_url: "http://127.0.0.1:1".to_owned(),
            callback_url: "http://127.0.0.1:1/callback".to_owned(),
            currency: vpay_core::Currency::from_code("XAF").expect("XAF is modelled"),
            settings: BTreeMap::new(),
            credentials: BTreeMap::new(),
            connect_timeout: vpay_provider::DEFAULT_CONNECT_TIMEOUT,
            request_timeout: vpay_provider::DEFAULT_REQUEST_TIMEOUT,
        }
    }

    fn destination_map(rail: &str, msisdn: &str) -> Map<String, Value> {
        let mut inner = Map::new();
        inner.insert("msisdn".to_owned(), Value::String(msisdn.to_owned()));
        let mut outer = Map::new();
        outer.insert(rail.to_owned(), Value::Object(inner));
        outer
    }

    fn param_of(error: &ApiError) -> String {
        match error {
            ApiError::InvalidParam { param, .. } => param.clone(),
            other => panic!("expected InvalidParam, got {other:?}"),
        }
    }

    fn intent(amount: i64, refunded: i64, pending: i64) -> vpay_db::PaymentIntentRow {
        vpay_db::PaymentIntentRow {
            id: "pi_000000000000000000000001".to_owned(),
            seq: 1,
            merchant_id: "acme".to_owned(),
            livemode: false,
            amount,
            amount_received: amount,
            amount_refunded: refunded,
            amount_refund_pending: pending,
            currency_code: "XAF".to_owned(),
            status: "succeeded".to_owned(),
            last_payment_error_code: None,
            last_payment_error_message: None,
            payment_method_types: serde_json::json!(["mtn_momo"]),
            metadata: serde_json::json!({}),
            description: None,
            customer_id: None,
            client_secret_suffix: "x".repeat(32),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    /// The wire vocabulary this module is written against, pinned as a
    /// literal: `resource` is what a `404` envelope's message names
    /// (`No such refund: …`), and a rename that only touched the constant
    /// would compile and change the message with nothing noticing.
    #[test]
    fn the_resource_name_is_the_one_the_documented_envelope_uses() {
        assert_eq!(RESOURCE, "refund");
        match not_found("re_x") {
            ApiError::NotFound { resource, id } => {
                assert_eq!(resource, "refund");
                assert_eq!(id, "re_x");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    /// The prefix this route reads is the one `vpay_core` mints, not a second
    /// spelling of it. A literal `"re_"` here and a changed `REFUND_PREFIX`
    /// would make every real id a `404`.
    #[test]
    fn the_prefix_short_circuit_uses_the_minters_own_vocabulary() {
        assert_eq!(vpay_core::ids::REFUND_PREFIX, "re_");
        assert!(vpay_core::ids::refund_id().starts_with(vpay_core::ids::REFUND_PREFIX));
    }

    /// **The decisive case for the capability check.** An `Origin` rail's
    /// refund goes back to the instrument that paid; a destination sent to
    /// one is a payee nobody will honour, and accepting it silently would
    /// tell a merchant their nomination had been acted on.
    ///
    /// Delete the `(Origin, Some(_))` arm from [`resolve_destination`] and
    /// this is the case that fails — measured 2026-09-16, and nothing else
    /// in the workspace fails with it, because vpay carries no `Origin` rail
    /// for an integration suite to drive.
    #[test]
    fn an_origin_rail_refuses_a_destination() {
        let error = resolve_destination(
            &OriginRail,
            "origin_rail",
            Some(&destination_map("origin_rail", PAYEE)),
            RefundDestination::Origin,
        )
        .expect_err("an Origin rail takes no destination");
        assert_eq!(param_of(&error), DESTINATION_PARAM);
    }

    /// The other half of the capability, and the half an adapter is entitled
    /// to rely on: a `Required` rail is never called with `None`.
    #[test]
    fn a_required_rail_refuses_a_refund_with_no_destination() {
        let error = resolve_destination(
            &required(),
            "required_rail",
            None,
            RefundDestination::Required,
        )
        .expect_err("a Required rail needs a payee");
        assert_eq!(param_of(&error), DESTINATION_PARAM);
    }

    /// An empty `destination` map is *no destination*, not a present one —
    /// `destination[mtn_momo]=` decodes to an empty map, and reading it as
    /// "the merchant nominated a payee" would hand the adapter a map with
    /// nothing in it.
    #[test]
    fn an_empty_destination_map_is_no_destination_at_all() {
        let error = resolve_destination(
            &required(),
            "required_rail",
            Some(&Map::new()),
            RefundDestination::Required,
        )
        .expect_err("an empty map nominates nobody");
        assert_eq!(param_of(&error), DESTINATION_PARAM);
    }

    /// **The rail code is stripped before the adapter is called.** The fake
    /// reads `msisdn` from the map it is handed; if this handler passed the
    /// outer map, the adapter would answer `Malformed` and this case would
    /// see a `400` instead of a payee.
    ///
    /// The other half of the boundary — that passing the outer map really
    /// does fail rather than silently reading some other key — is
    /// `the_outer_destination_map_is_refused_rather_than_misread` in both
    /// adapters' own suites.
    #[test]
    fn the_adapter_is_handed_the_rail_scoped_inner_map() {
        let destination = resolve_destination(
            &required(),
            "required_rail",
            Some(&destination_map("required_rail", PAYEE)),
            RefundDestination::Required,
        )
        .expect("a well-formed destination parses")
        .expect("a Required rail yields Some");
        assert_eq!(destination.msisdn(), "237600000200");
    }

    /// A destination naming a rail that is not the one the charge was taken
    /// on never reaches the adapter: the outer key is vpay's envelope, and a
    /// map that names no rail this charge used is the caller's mistake.
    #[test]
    fn a_destination_naming_another_rail_is_refused_by_the_core() {
        let error = resolve_destination(
            &required(),
            "required_rail",
            Some(&destination_map("some_other_rail", PAYEE)),
            RefundDestination::Required,
        )
        .expect_err("the envelope must name this rail");
        assert_eq!(param_of(&error), DESTINATION_PARAM);
    }

    /// **A malformed payee is a `400` naming `destination`, never a `502`.**
    ///
    /// `parse_destination` answers `ProviderError::Malformed`, whose
    /// classification is written for a rail that answered gibberish — 502,
    /// `Retry::AfterBackoff`, and the envelope sentence "The payment rail is
    /// temporarily unavailable. The charge will be retried." Forwarding it
    /// unchanged would give a merchant that answer for a typo in their own
    /// request. This case is what fails if a future author `?`s the error
    /// instead of translating it.
    #[test]
    fn a_malformed_payee_is_the_callers_error_and_not_the_rails() {
        let error = resolve_destination(
            &required(),
            "required_rail",
            // No `+`: `RefundTarget::mobile_money` refuses it, because a
            // market-agnostic crate has no country to attach it to.
            Some(&destination_map("required_rail", "600000200")),
            RefundDestination::Required,
        )
        .expect_err("a bare national number is not an international one");

        assert_eq!(param_of(&error), DESTINATION_PARAM);
        assert_eq!(
            error.category().http_status(),
            400,
            "a merchant's typo must not be answered as a rail outage"
        );
        assert_eq!(error.retry(), vpay_core::Retry::Never);
    }

    /// No refusal about a payee ever carries the payee. `InvalidMsisdn`'s
    /// variants are unit variants precisely so the adapter cannot leak one,
    /// and this is the check that the `/v1` layer does not undo it by
    /// echoing its own input back.
    #[test]
    fn a_refused_destination_never_echoes_the_number() {
        let error = resolve_destination(
            &required(),
            "required_rail",
            Some(&destination_map("required_rail", "+23760000020x")),
            RefundDestination::Required,
        )
        .expect_err("a non-digit string is not a payee");
        let rendered = format!("{error} {}", error.public_message());
        assert!(
            !rendered.contains("23760000020"),
            "a refusal echoed the number it refused: {rendered}"
        );
    }

    /// The masked form an operator sees, and the only rendering of a payee
    /// this module produces.
    #[test]
    fn the_only_logged_form_of_a_payee_is_masked() {
        let destination = RefundTarget::mobile_money(PAYEE).expect("a documentation MSISDN");
        let masked = crate::v1::account_holders::masked(destination.msisdn());
        assert_eq!(masked, "+2376••••200");
        assert!(!masked.contains("237600000200"));
    }

    /// `amount` omitted is a full refund of what is **left**, not of what was
    /// captured: an intent that has already had 2 000 back has 3 000 to give.
    #[test]
    fn an_omitted_amount_refunds_what_is_left() {
        assert_eq!(
            resolve_amount(None, &intent(5000, 2000, 0), required().capabilities())
                .expect("3 000 is left"),
            3000
        );
        assert_eq!(
            resolve_amount(None, &intent(5000, 1000, 1000), required().capabilities())
                .expect("a reservation is not available to refund twice"),
            3000
        );
    }

    /// Nothing left is a `409`, not a `500` from `amount_positive` and not a
    /// zero-amount row.
    #[test]
    fn an_intent_with_nothing_left_is_a_conflict() {
        let error = resolve_amount(None, &intent(5000, 5000, 0), required().capabilities())
            .expect_err("a fully refunded intent has nothing left");
        assert!(matches!(error, ApiError::Conflict { .. }), "{error:?}");
        assert_eq!(error.category().http_status(), 409);
    }

    /// A rail that refunds in full only refuses a partial, on the capability
    /// and never on a code — and "in full" is the intent's own amount, so a
    /// second refund "for the remainder" is refused too.
    #[test]
    fn a_rail_without_partial_refunds_refuses_a_partial_amount() {
        let capabilities = Capabilities {
            supports_partial_refunds: false,
            ..required().capabilities()
        };
        let error = resolve_amount(Some("2500"), &intent(5000, 0, 0), capabilities)
            .expect_err("this rail refunds in full only");
        assert_eq!(param_of(&error), "amount");

        assert_eq!(
            resolve_amount(Some("5000"), &intent(5000, 0, 0), capabilities)
                .expect("the whole amount is not a partial refund"),
            5000
        );
    }

    /// An `amount` that is not a whole number names `amount`, not `body`.
    #[test]
    fn a_non_numeric_amount_names_its_own_parameter() {
        let error = resolve_amount(
            Some("25.00"),
            &intent(5000, 0, 0),
            required().capabilities(),
        )
        .expect_err("minor units are integers");
        assert_eq!(param_of(&error), "amount");
    }

    /// The lookup's middle answer is the payer's problem and the bottom one
    /// is ours. Collapsing them would tell an integrator that a real
    /// person's real account does not exist because a socket timed out.
    #[tokio::test]
    async fn an_unregistered_payee_is_a_400_and_an_unreachable_rail_is_not() {
        let rail = provider_config();

        let unregistered = RequiredRail {
            lookup: Some(Ok(None)),
        };
        let destination = RefundTarget::mobile_money(PAYEE).expect("a documentation MSISDN");
        let error = verify_registered_holder(&unregistered, &destination, &rail)
            .await
            .expect_err("the rail has no record of this number");
        assert_eq!(param_of(&error), DESTINATION_PARAM);

        let unreachable = RequiredRail {
            lookup: Some(Err(ProviderError::transport("unreachable"))),
        };
        let error = verify_registered_holder(&unreachable, &destination, &rail)
            .await
            .expect_err("nobody asked");
        assert!(matches!(error, ApiError::Provider(_)), "{error:?}");
        assert_eq!(error.category().http_status(), 502);
    }

    /// A rail with no lookup accepts the destination unverified, and does so
    /// without calling a method it would answer `Unsupported` to. Orange is
    /// exactly this case (RFC-0003 § 1).
    #[tokio::test]
    async fn a_rail_without_a_lookup_accepts_the_destination_unverified() {
        let rail = provider_config();
        let destination = RefundTarget::mobile_money(PAYEE).expect("a documentation MSISDN");
        verify_registered_holder(&required(), &destination, &rail)
            .await
            .expect("a rail with no lookup has nothing to ask");
    }

    /// An update may only change `metadata`. A merchant who sends `amount`
    /// must be told, not answered `200` with the original.
    #[test]
    fn an_update_refuses_every_field_but_metadata() {
        for (params, expected) in [
            (
                UpdateParams {
                    metadata: None,
                    amount: Some("100".to_owned()),
                    reason: None,
                    payment_intent: None,
                    destination: None,
                },
                "amount",
            ),
            (
                UpdateParams {
                    metadata: None,
                    amount: None,
                    reason: Some("duplicate".to_owned()),
                    payment_intent: None,
                    destination: None,
                },
                "reason",
            ),
            (
                UpdateParams {
                    metadata: None,
                    amount: None,
                    reason: None,
                    payment_intent: Some("pi_x".to_owned()),
                    destination: None,
                },
                "payment_intent",
            ),
            (
                UpdateParams {
                    metadata: None,
                    amount: None,
                    reason: None,
                    payment_intent: None,
                    destination: Some(destination_map("required_rail", PAYEE)),
                },
                DESTINATION_PARAM,
            ),
        ] {
            let error = params
                .reject_unsupported()
                .expect_err("only metadata may be updated");
            assert_eq!(param_of(&error), expected);
        }

        UpdateParams {
            metadata: Some(BTreeMap::new()),
            amount: None,
            reason: None,
            payment_intent: None,
            destination: None,
        }
        .reject_unsupported()
        .expect("metadata alone is what this route takes");
    }

    /// The `Debug` a `tracing::debug!(?params)` would print carries the key
    /// set and no value. This is the impl that stops a payee's number
    /// reaching an operator's log before an adapter has ever seen it.
    #[test]
    fn the_create_params_debug_prints_no_payee() {
        let params = CreateParams {
            payment_intent: Some("pi_1".to_owned()),
            amount: None,
            reason: None,
            destination: Some(destination_map("required_rail", PAYEE)),
            metadata: BTreeMap::new(),
        };
        let rendered = format!("{params:?}");
        assert!(
            !rendered.contains("237600000200") && !rendered.contains(PAYEE),
            "the destination leaked into Debug: {rendered}"
        );
        assert!(
            rendered.contains("required_rail"),
            "an operator still needs to know which rail was nominated: {rendered}"
        );
    }

    /// A reason longer than the column's CHECK is a `400` naming `reason`,
    /// not a constraint violation rendered as a `500`.
    #[test]
    fn an_over_long_reason_names_its_own_parameter() {
        let error = validated_reason(Some(&"x".repeat(REASON_MAX_CHARS + 1)))
            .expect_err("the column bounds this");
        assert_eq!(param_of(&error), "reason");
        assert_eq!(validated_reason(Some("  ")).expect("blank is absent"), None);
    }

    /// The two event types are the documented Stripe spellings, and both are
    /// in the vocabulary migration `0018` closed. A typo here would be
    /// refused by `type_is_a_documented_event` at the database — which is a
    /// `500` on a money path, discovered in production.
    #[test]
    fn the_event_types_are_the_documented_spellings() {
        assert_eq!(EVENT_REFUNDED, "charge.refunded");
        assert_eq!(EVENT_REFUND_UPDATED, "charge.refund.updated");
    }
}
