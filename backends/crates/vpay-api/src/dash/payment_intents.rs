//! `GET /dash/v1/payment_intents` and `GET /dash/v1/payment_intents/{id}` —
//! the whole of dashboard slice 1's read surface.
//!
//! Both are the merchant surface's reads with three differences, and each of
//! the three is a decision rather than an accident:
//!
//! * **the tenant is the bound one**, not one resolved from the caller's own
//!   credential — see [`super`]'s "The boundary, stated once";
//! * **no `client_secret` is ever rendered.** `GET /v1/payment_intents/{id}`
//!   includes it so a merchant who lost the create response can recover it;
//!   a *staff* reader has no such need, and the value authorises confirming
//!   the payment from any browser (`crate::browser`). The dashboard is an
//!   observer (ADR-0008), so it observes the object and not the credential
//!   that spends it;
//! * **the detail read carries the charge, the refunds and the event
//!   timeline**, which `/v1` does not assemble anywhere. That is what the
//!   surface exists for: a merchant asks about one object, an operator asks
//!   what happened.
//!
//! # What is not here, and why it is absent rather than empty
//!
//! * **No `search` by payer phone.** `charges.payer_ref` and
//!   `charges.payer_ref_masked` are **never written** — the confirm path
//!   stores `None` in both (`crate::v1::payment_intents`'s `open_attempt`),
//!   a gap `docs/status.md` names. A phone filter over a column that is
//!   always `NULL` would answer "no results" for every payer who ever paid,
//!   which is worse than no filter: it reads as an answer.
//! * **No `search` by id.** An id search over one tenant is
//!   [`retrieve`] with a `404`; a *prefix* search is a different query with
//!   an index that does not exist (`payment_intents` is indexed on
//!   `(merchant_id, seq DESC)`).
//! * **No writes.** [`super`]'s "Read-only" section.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use vpay_core::IntentStatus;
use vpay_db::{
    ChargeRow, EventRow, Events, IntentFilter, PaymentIntentRow, PaymentIntents, RefundRow, Refunds,
};

use crate::ApiError;
use crate::form::VpayQuery;
use crate::model::{ListObject, PaymentIntentObject, RefundObject};
use crate::v1::{MerchantScope, paging};

/// The object type this module reads, in the API's own vocabulary — the
/// `resource` a `404` names.
const RESOURCE: &str = "payment_intent";

/// `GET /dash/v1/payment_intents`'s query parameters.
///
/// Every field is `Option<String>` and parsed by hand for
/// `crate::v1::payment_intents::ListParams`' reason: a typed `Option<i64>`
/// makes serde answer "invalid type" for `limit=abc`, and the caller needs
/// to be told *which* parameter it got wrong (Stripe's `param` field), which
/// is a sentence this crate writes and serde does not.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ListParams {
    limit: Option<String>,
    starting_after: Option<String>,
    ending_before: Option<String>,
    /// One `IntentStatus` wire value. An unknown one is a `400` naming
    /// `status`, not an empty list: "no payments matched" and "you asked for
    /// a status that does not exist" are different answers, and the empty
    /// list is the one an operator would believe.
    status: Option<String>,
    /// RFC 3339, inclusive lower bound on `created`.
    created_gte: Option<String>,
    /// RFC 3339, inclusive upper bound on `created`.
    created_lte: Option<String>,
}

/// `GET /dash/v1/payment_intents`.
///
/// Paged with the same cursors, the same default and the same ceiling as
/// `/v1` — [`crate::v1::paging`] is shared rather than re-derived, because a
/// dashboard whose `has_more` meant something different from the merchant
/// API's would be an operator and a merchant reading the same rows and
/// disagreeing about whether there are more.
pub(crate) async fn list(
    State(repositories): State<super::Repos>,
    scope: MerchantScope,
    VpayQuery(params): VpayQuery<ListParams>,
) -> Result<Response, ApiError> {
    let page = paging::list_page(
        params.limit.as_deref(),
        params.starting_after,
        params.ending_before,
        crate::v1::payment_intents::CURSOR,
    )?;
    let filter = IntentFilter {
        status: params.status.map(parse_status).transpose()?,
        created_gte: params
            .created_gte
            .map(|raw| parse_timestamp("created_gte", &raw))
            .transpose()?,
        created_lte: params
            .created_lte
            .map(|raw| parse_timestamp("created_lte", &raw))
            .transpose()?,
    };

    let (rows, has_more) = PaymentIntents::list_page_filtered(
        repositories.as_ref(),
        scope.merchant_id(),
        &page,
        &filter,
    )
    .await?;
    let data = rows
        .iter()
        .map(PaymentIntentObject::try_from)
        .collect::<Result<Vec<_>, _>>()?;

    crate::v1::payment_intents::json_response(
        StatusCode::OK,
        &ListObject::new(data, has_more, super::nested("/payment_intents")),
    )
}

/// `GET /dash/v1/payment_intents/{id}`.
///
/// One read of the intent — tenant-scoped, so another merchant's id is the
/// **same `404`** a nonexistent one gets, exactly as on `/v1` — then the
/// charge, the refunds and the events.
///
/// # Which of the four reads carries a tenant, and which does not
///
/// Three of the four take `scope.merchant_id()` and filter on it: the intent
/// ([`PaymentIntents::get_for_merchant`]), the refunds
/// ([`Refunds::list_for_intent`], through the join onto `payment_intents`)
/// and the events ([`Events::list_for_objects`]). The repetition is
/// deliberate, because "the first read already checked" is precisely the
/// reasoning that stops being true when someone reorders the function — and
/// for the events it is not merely belt and braces: `events.object_id` is a
/// plain `TEXT` column with **no foreign key** (migration `0018`), so its
/// `merchant_id` predicate is the only thing between two tenants' timelines
/// (`the_detail_read_renders_the_timeline_and_the_refunds_it_has`).
///
/// **The charge read is the exception, and it is not tenant-scoped.**
/// [`vpay_db::Charges::get_for_intent`] takes no `merchant_id` and cannot:
/// `charges` has no such column, and the trait's own doc says taking one
/// "would suggest this function performs that check, which would be a worse
/// lie than not offering it". It is safe here for exactly one reason — the
/// intent id it is handed came from the scoped read above — which means it
/// *is* an instance of the reasoning this section warns about, held together
/// by the ordering. A future edit that reads the charge before the intent, or
/// from an id a caller supplied, would leak it. Until 2026-09-06 this comment
/// claimed all four reads were "tenant-scoped in [their] own right", which
/// told a reader the opposite of that.
pub(crate) async fn retrieve(
    State(repositories): State<super::Repos>,
    scope: MerchantScope,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let repositories = repositories.as_ref();
    let intent = PaymentIntents::get_for_merchant(repositories, scope.merchant_id(), &id)
        .await?
        .ok_or_else(|| ApiError::NotFound {
            resource: RESOURCE,
            id: id.clone(),
        })?;

    let charge = repositories.get_for_intent(&intent.id).await?;
    let refunds = Refunds::list_for_intent(repositories, scope.merchant_id(), &intent.id).await?;

    // Both ids, because one payment's history is spread across two objects
    // — see `Events::list_for_objects`. The charge's id is only in the list
    // when there is a charge; an intent that was never confirmed has one
    // object and one id, not one id and a `NULL`.
    let mut object_ids = vec![intent.id.clone()];
    if let Some(charge) = charge.as_ref() {
        object_ids.push(charge.id.clone());
    }
    let events = Events::list_for_objects(repositories, scope.merchant_id(), &object_ids).await?;

    crate::v1::payment_intents::json_response(
        StatusCode::OK,
        &PaymentDetail::assemble(&intent, charge.as_ref(), &refunds, &events)?,
    )
}

/// What `GET /dash/v1/payment_intents/{id}` answers.
///
/// Not a Stripe shape and not pretending to be one: Stripe has no
/// "everything about this payment" object, and inventing one under the
/// merchant API's vocabulary would make an SDK author reasonably expect it
/// there. `object: "dashboard.payment_detail"` says whose shape it is.
///
/// The `payment_intent` inside **is** the merchant surface's object, byte
/// for byte minus the credential, so an operator and a merchant looking at
/// the same payment cannot be told different things about its status or its
/// amounts.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct PaymentDetail {
    /// Always `"dashboard.payment_detail"`.
    object: &'static str,
    /// The intent exactly as `/v1` renders it, without `client_secret`.
    payment_intent: PaymentIntentObject,
    /// The one charge, or `null` for an intent nobody confirmed. One, not a
    /// list: one charge per intent, forever (AGENTS.md), enforced by a
    /// unique index rather than by this field's shape.
    charge: Option<ChargeSummary>,
    /// Every refund of this intent, oldest first.
    refunds: Vec<RefundObject>,
    /// Every event about this intent or its charge, oldest first — the
    /// timeline an operator reads.
    events: Vec<TimelineEvent>,
}

impl PaymentDetail {
    /// Builds the envelope from the four reads [`retrieve`] made.
    ///
    /// A constructor rather than a struct literal in the handler so that
    /// `object` cannot be spelled twice, and so a fifth section added later
    /// is a compile error at one call site rather than a field silently
    /// missing from the response.
    ///
    /// # Errors
    ///
    /// Whatever [`PaymentIntentObject::try_from`] returns for a row this
    /// deployment stored — a status or currency outside the vocabulary,
    /// which means the database holds something this build cannot name.
    fn assemble(
        intent: &PaymentIntentRow,
        charge: Option<&ChargeRow>,
        refunds: &[RefundRow],
        events: &[EventRow],
    ) -> Result<Self, ApiError> {
        Ok(Self {
            object: "dashboard.payment_detail",
            payment_intent: PaymentIntentObject::try_from(intent)?,
            charge: charge.map(ChargeSummary::from),
            refunds: refunds
                .iter()
                .map(RefundObject::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            events: events.iter().map(TimelineEvent::from).collect(),
        })
    }
}

/// The charge, as an operator needs to see it.
///
/// **Deliberately not every column.** `provider_ref_extra` is a rail's own
/// blob, `return_url` and `redirect_url` are a payer's browsing path, and
/// `payer_ref` is an unmasked phone number. What is here is what answers
/// "which rail, what did it say, and when": the rail's code, its own
/// reference, the state, the amounts, the failure pair, and the *masked*
/// payer reference.
///
/// # `payer_ref_masked` is `null` on every row this deployment has ever
/// written
///
/// Nothing writes that column — `open_attempt` stores `None`
/// (`docs/status.md`). It is rendered anyway, and rendered from the column
/// rather than derived here, for two reasons: deriving a mask in the
/// renderer would mean reading `payer_ref`, the unmasked value, into a staff
/// surface; and a field that appears the day the column is written is a
/// smaller change than a field added then. What must not happen is this
/// field quietly becoming the *unmasked* value because the masked one was
/// empty.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
struct ChargeSummary {
    /// Always `"charge"`.
    object: &'static str,
    /// The `ch_…` id.
    id: String,
    /// `providers.code` — which rail took it.
    provider_code: String,
    /// vpay's own idempotency reference for the rail call, which is what a
    /// rail's support desk is asked about.
    provider_reference_id: String,
    /// The rail's own transaction id, once it gave one.
    provider_txn_id: Option<String>,
    /// The charge's state as stored.
    state: String,
    /// Minor units (`docs/flows/money.md`).
    amount: i64,
    /// Lower-case, matching the merchant surface's `currency`.
    currency: String,
    /// The payer's instrument, masked. See this struct's docs: `null` on
    /// every row written so far.
    payer_ref_masked: Option<String>,
    /// The closed failure vocabulary, once it failed.
    failure_code: Option<String>,
    /// The rail's own words for that failure — kept because "the rail said
    /// no" is not an answer an operator can act on. Staff-only: it is not
    /// rendered on `/v1` anywhere.
    failure_raw: Option<String>,
    /// Unix seconds, like every other timestamp this API renders.
    created: i64,
    /// Unix seconds.
    updated: i64,
}

impl From<&ChargeRow> for ChargeSummary {
    fn from(row: &ChargeRow) -> Self {
        Self {
            object: "charge",
            id: row.id.clone(),
            provider_code: row.provider_code.clone(),
            provider_reference_id: row.provider_reference_id.to_string(),
            provider_txn_id: row.provider_txn_id.clone(),
            state: row.state.clone(),
            amount: row.amount,
            currency: row.currency_code.to_ascii_lowercase(),
            payer_ref_masked: row.payer_ref_masked.clone(),
            failure_code: row.failure_code.clone(),
            failure_raw: row.failure_raw.clone(),
            created: row.created_at.unix_timestamp(),
            updated: row.updated_at.unix_timestamp(),
        }
    }
}

/// One row of the timeline.
///
/// The event's **type and time**, and its id — not its `data` snapshot. The
/// snapshot is the whole object as it was at emit time, so rendering it
/// would put a second, older copy of the payment intent inside a response
/// whose first field is the current one, and an operator reading the page
/// would have no way to tell which they were looking at. `GET
/// /v1/events/{id}` is where the snapshot lives, for whoever wants it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
struct TimelineEvent {
    /// Always `"event"`.
    object: &'static str,
    /// The `evt_…` id, so an operator can go and read the snapshot.
    id: String,
    /// One of the seven types migration `0018`'s CHECK admits.
    r#type: String,
    /// Which object this event is about — the intent's id or the charge's.
    object_id: String,
    /// Unix seconds.
    created: i64,
}

impl From<&EventRow> for TimelineEvent {
    fn from(row: &EventRow) -> Self {
        Self {
            object: "event",
            id: row.id.clone(),
            r#type: row.event_type.clone(),
            object_id: row.object_id.clone(),
            created: row.created_at.unix_timestamp(),
        }
    }
}

/// Parses a `status` filter value, refusing anything outside
/// [`IntentStatus`]'s vocabulary.
///
/// The refusal is the point. `IntentFilter::status` is compared as text, so
/// an unknown value would be a perfectly well-formed query returning zero
/// rows — and "no payments are `succeded`" is a sentence an operator would
/// read as an answer about their payments rather than about their typo.
fn parse_status(raw: String) -> Result<String, ApiError> {
    if IntentStatus::ALL
        .iter()
        .any(|status| status.as_wire_str() == raw)
    {
        return Ok(raw);
    }
    Err(ApiError::invalid_param(
        "status",
        format!(
            "Unknown payment intent status. Known statuses are: {}.",
            IntentStatus::ALL
                .iter()
                .map(|status| status.as_wire_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ))
}

/// Parses an RFC 3339 timestamp bound, naming the parameter that was wrong.
///
/// RFC 3339 rather than Stripe's unix-seconds `created[gte]`: this is not an
/// SDK surface ([`super`]'s "Not an SDK surface"), the caller is the
/// dashboard's own server-side fetch, and a timestamp a human can read in a
/// URL is worth more here than compatibility with a client that will never
/// call it.
fn parse_timestamp(param: &'static str, raw: &str) -> Result<time::OffsetDateTime, ApiError> {
    time::OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339).map_err(|_| {
        ApiError::invalid_param(
            param,
            "Must be an RFC 3339 timestamp, for example `2026-09-06T00:00:00Z`.",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every status the API can render is accepted by the filter, and
    /// nothing else is.
    ///
    /// Walks `IntentStatus::ALL` rather than listing the strings, so a
    /// status added to the enum is covered here the day it is added — the
    /// failure this guards against is a filter that silently cannot select
    /// the newest status, which looks like "there are none of those".
    #[test]
    fn the_status_filter_accepts_exactly_the_known_statuses() {
        for status in IntentStatus::ALL {
            assert_eq!(
                parse_status(status.as_wire_str().to_owned())
                    .as_deref()
                    .ok(),
                Some(status.as_wire_str()),
            );
        }

        for unknown in ["succeded", "SUCCEEDED", "", "succeeded ", "'; DROP TABLE"] {
            let error = parse_status(unknown.to_owned())
                .expect_err("an unknown status must be refused, not answered with an empty list");
            assert!(
                matches!(&error, ApiError::InvalidParam { param, .. } if param == "status"),
                "{error:?}"
            );
        }
    }

    /// A bad timestamp names its own parameter, not the other one.
    ///
    /// Both bounds go through one function, so the parameter name is an
    /// argument — and an argument is exactly the kind of thing that gets
    /// copied from the line above and left pointing at the wrong field.
    #[test]
    fn a_malformed_timestamp_names_the_parameter_it_came_from() {
        for param in ["created_gte", "created_lte"] {
            let error =
                parse_timestamp(param, "yesterday").expect_err("`yesterday` is not RFC 3339");
            assert!(
                matches!(&error, ApiError::InvalidParam { param: named, .. } if named == param),
                "{error:?}"
            );
        }

        let parsed = parse_timestamp("created_gte", "2026-09-06T00:00:00Z")
            .expect("an RFC 3339 timestamp parses");
        assert_eq!(parsed.unix_timestamp(), 1_788_652_800);
    }

    /// The detail envelope names its own shape, and never carries a
    /// `client_secret`.
    ///
    /// The second half is the load-bearing one and it is asserted over the
    /// **serialised** value rather than over the type: `PaymentIntentObject`
    /// has no such field, but the day someone renders
    /// `PaymentIntentWithSecret` here instead — one word — the type still
    /// compiles and this test is what notices.
    #[test]
    fn the_detail_envelope_carries_no_client_secret() {
        let detail = PaymentDetail::assemble(&intent_row(), None, &[], &[])
            .expect("the fixture row renders");
        let value = serde_json::to_value(&detail).expect("a wire DTO always serialises");

        assert_eq!(
            value.get("object"),
            Some(&serde_json::json!("dashboard.payment_detail"))
        );
        assert_eq!(value.get("charge"), Some(&serde_json::Value::Null));
        assert_eq!(value.get("refunds"), Some(&serde_json::json!([])));
        assert_eq!(value.get("events"), Some(&serde_json::json!([])));
        assert!(
            !serde_json::to_string(&value)
                .expect("a JSON value always serialises")
                .contains("client_secret"),
            "{value}"
        );
    }

    /// One `payment_intents` row, in the shape the repository returns.
    ///
    /// Written here rather than shared, because the *point* of this fixture
    /// is `client_secret_suffix`: it carries a value, so a renderer that
    /// leaked it would have something to leak. A shared fixture with an
    /// empty suffix would make the test above pass for the wrong reason.
    fn intent_row() -> PaymentIntentRow {
        PaymentIntentRow {
            id: "pi_dashboard_fixture".to_owned(),
            seq: 1,
            merchant_id: "acme-tenant".to_owned(),
            livemode: false,
            amount: 5_000,
            amount_received: 0,
            amount_refunded: 0,
            amount_refund_pending: 0,
            currency_code: "XAF".to_owned(),
            status: IntentStatus::INITIAL.as_wire_str().to_owned(),
            last_payment_error_code: None,
            last_payment_error_message: None,
            payment_method_types: serde_json::json!(["mtn_momo"]),
            metadata: serde_json::json!({}),
            description: None,
            customer_id: None,
            client_secret_suffix: "thisisthesecretsuffix".to_owned(),
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
        }
    }
}
