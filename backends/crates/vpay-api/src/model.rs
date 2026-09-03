//! The `/v1` wire objects: what a merchant's SDK actually receives.
//!
//! These types exist separately from `vpay-core`'s domain types and from
//! `vpay-db`'s row structs on purpose (Step 2's D5). A row carries things no
//! merchant may see — `seq`, `merchant_id`, the running `amount_*` totals —
//! and a domain type carries no rendering decisions at all. Putting the wire
//! shape in its own module means "what goes over the wire" is one file that
//! can be read against `docs/api/README.md`'s object table, and a new column
//! cannot leak into a response by being added to a struct.
//!
//! # Every key, every time
//!
//! There is no `#[serde(skip_serializing_if)]` anywhere below, and that is the
//! whole design of these types. Stripe's objects always carry every documented
//! key, `null` where there is no value, and both SDKs model them accordingly:
//! `sdks/rust/src/model.rs`'s `PaymentIntent` has
//! `next_action: Option<NextAction>` as a *required* field, so an object that
//! omitted the key would fail to decode in a merchant's own client. The
//! fixture in `sdks/rust/tests/support/mod.rs` is the pinned shape, and
//! `the_wire_object_is_byte_for_byte_the_sdk_fixture` below compares against
//! it directly.
//!
//! # Conversions are fallible, and that is deliberate
//!
//! A row's `status` is a `String` (D4: Postgres enums are text in `vpay-db`
//! and `vpay-core` parses them), so rendering a row means parsing a value this
//! process wrote earlier. There is no honest total conversion: a label outside
//! `IntentStatus`'s five means the schema and the code disagree, which is
//! `ApiError::Internal` (500, pages) and never a status invented to make a
//! `From` impl compile. Hence [`TryFrom`] rather than [`From`] — a handler
//! writes `row.try_into()?` and the `?` does the same work `From` would have.

use std::collections::BTreeMap;

use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use vpay_core::IntentStatus;

use crate::ApiError;

/// Declares a type that serialises as one fixed string and holds nothing.
///
/// Stripe's `object` discriminator is part of the contract — an SDK switches
/// on it — so it must not be a `String` field a call site can fill in wrongly.
/// A zero-sized tag makes `object: "payment_intent"` unwritable by anything
/// except this module, while still being an ordinary struct field that serde
/// renders in place.
macro_rules! object_tag {
    ($(#[$meta:meta])* $name:ident, $wire:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
        pub struct $name;

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str($wire)
            }
        }
    };
}

object_tag!(
    /// The `"payment_intent"` discriminator.
    PaymentIntentTag,
    "payment_intent"
);
object_tag!(
    /// The `"list"` discriminator.
    ListTag,
    "list"
);
object_tag!(
    /// The `"event"` discriminator.
    EventTag,
    "event"
);

/// Where to send a payer on a redirect rail.
///
/// Stripe's own `next_action.redirect_to_url` shape, so a merchant's existing
/// redirect handling works unchanged (`docs/flows/payment-lifecycle.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedirectToUrl {
    /// The rail's hosted page. Opaque to us — never parsed or rewritten.
    pub url: String,
    /// Where the rail returns the payer afterwards; `null` if it was not given
    /// one.
    pub return_url: Option<String>,
}

/// What a payer must do next.
///
/// Only ever `redirect_to_url`: a push rail has nothing for a browser to do
/// while a payer types a PIN into their own handset, so `next_action` stays
/// `null` there. The externally-tagged-by-`type` shape matches Stripe's and
/// `sdks/rust/src/model.rs`'s `NextAction` exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NextAction {
    /// Send the payer to [`RedirectToUrl::url`].
    RedirectToUrl {
        /// The destination and the return URL the rail was given.
        redirect_to_url: RedirectToUrl,
    },
}

/// The `last_payment_error` sub-object: why the last charge on this intent was
/// refused.
///
/// Named `…Object` rather than `LastPaymentError` because
/// `cargo xtask verify-errors` (ADR-0011) treats every public type whose name
/// ends in `Error` as an error type owing an `impl Classify`. This is a wire
/// DTO, not an error: it is *rendered*, never returned, and classifying it
/// would put a meaningless entry into the check that keeps real error types
/// honest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastPaymentErrorObject {
    /// A code from `docs/flows/failures.md`'s closed vocabulary.
    ///
    /// A `String` rather than `vpay_core::FailureCode`, mirroring the SDKs'
    /// own deliberate choice: the column is text (D4), the vocabulary is owned
    /// by the core and may grow, and a value that failed to parse would then
    /// make a merchant's `GET` answer 500 instead of showing them why their
    /// payment failed. The vocabulary is enforced where it is *written* — the
    /// `failure_code` Postgres enum — not on the read path.
    pub code: String,
    /// The rail's failure, in words, as `docs/flows/failures.md` maps it.
    pub message: String,
}

/// A `payment_intent`, exactly as `docs/api/README.md`'s object table and
/// `sdks/rust/tests/support/mod.rs`'s fixture describe it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PaymentIntentObject {
    /// `pi_…` — `vpay_core::ids::payment_intent_id`.
    pub id: String,
    /// Always `"payment_intent"`.
    pub object: PaymentIntentTag,
    /// Integer minor units (`docs/flows/money.md`): `5000` on a `xaf` intent
    /// is 5,000 FCFA, because XAF is zero-decimal.
    pub amount: i64,
    /// **Lowercase** on the wire (`xaf`), Stripe-style, although the column
    /// holds the uppercase ISO-4217 code. Lowercased once, here at the
    /// boundary, so no handler has to remember to.
    pub currency: String,
    /// One of the five `vpay_core::IntentStatus` values. Typed rather than a
    /// `String` so a handler cannot render a status that is not in the enum.
    pub status: IntentStatus,
    /// The rail codes this intent may be confirmed against.
    pub payment_method_types: Vec<String>,
    /// Redirect rails only; `null` on a push rail.
    pub next_action: Option<NextAction>,
    /// The last rail refusal, if any. Present *with*
    /// `requires_payment_method` — there is no `failed` status.
    pub last_payment_error: Option<LastPaymentErrorObject>,
    /// The merchant's own key/value pairs, echoed back.
    ///
    /// `Map<String, Value>` rather than `BTreeMap<String, String>` although
    /// the contract says string→string: the API only ever writes strings (the
    /// form decoder produces nothing else), so this is lossless today, and
    /// keeping the row's own JSON means a value written by something other
    /// than a `/v1` handler renders rather than making a merchant's `GET`
    /// answer 500.
    pub metadata: Map<String, Value>,
    /// The merchant's own description, or `null`.
    pub description: Option<String>,
    /// Unix **seconds** — not milliseconds, and not RFC 3339. Stripe's
    /// `created` is seconds and both SDKs model it as `i64`.
    pub created: i64,
    /// `false` for a sandbox deployment's objects. Taken from the row, not
    /// from configuration read at render time: an object created under one
    /// setting must not start describing itself differently if the deployment
    /// is reconfigured.
    pub livemode: bool,
}

impl PaymentIntentObject {
    /// Attaches (or clears) `next_action`.
    ///
    /// Separate from the row conversion because `next_action` is not on the
    /// intent row at all: it is derived from the *charge* — the rail's
    /// redirect URL, plus the `return_url` the merchant supplied. A handler
    /// that has loaded the charge adds it; one that has not renders `null`,
    /// which is the correct answer for a push rail and for an intent nobody
    /// has confirmed.
    #[must_use]
    pub fn with_next_action(mut self, next_action: Option<NextAction>) -> Self {
        self.next_action = next_action;
        self
    }
}

/// A `payment_intent` **plus the payer credential that addresses it** —
/// what `POST /v1/payment_intents`, `GET /v1/payment_intents/{id}` and the
/// two `/v1/browser` routes answer with (Step 5c's D2).
///
/// # Why a wrapper, and not a field on [`PaymentIntentObject`]
///
/// Because the field must reach exactly four responses and no others, and a
/// field on `PaymentIntentObject` would reach *every* response that type can
/// render — which includes two a payer credential must never appear in:
///
/// * `GET /v1/payment_intents`, the list. One page would hand a merchant's
///   integration the live browser credential for every intent on it, and a
///   list page is the response most likely to be logged wholesale.
/// * `events.data.object` — `vpay_worker::handlers`' `intent_snapshot`
///   renders through the same type, so the credential would be in every
///   webhook body, signed, delivered at-least-once, and stored in `events`
///   forever.
///
/// Neither of those is a hypothetical: both are the *current* callers of
/// `PaymentIntentObject::try_from`. `every_documented_key_is_present_including_the_null_ones`
/// below asserts that object has exactly twelve keys, and that assertion is
/// the tripwire this type exists to leave standing.
///
/// # `#[serde(flatten)]`, so the wire shape is the twelve keys plus one
///
/// A payer's client decodes one object, not a nested one:
/// `sdks/stripe-js/src/types.ts`'s `PaymentIntent` is
/// `PaymentIntentObject`'s twelve fields with `client_secret` beside them.
/// Flattening is what makes that true *by construction* rather than by two
/// structs agreeing — and it is why `client_secret` is declared after the
/// flattened field: `serde_json::Map` sorts keys on serialisation in this
/// workspace (no `preserve_order`), so declaration order does not affect the
/// bytes, but reading order should still match the shape.
///
/// No `Deserialize`: nothing decodes this. A merchant's SDK models the wire,
/// and `vpay_api` only ever writes it.
///
/// `Debug` is **hand-written** below rather than derived, because
/// [`Self::client_secret`] is a live credential — see that impl. Mirrors
/// `vpay_db::PaymentIntentRow`'s impl in shape and in what it redacts: this
/// is the one place in `vpay-api` a full, joined `client_secret` exists in
/// memory (every other type here carries either no secret or, on
/// [`PaymentIntentRow`](vpay_db::PaymentIntentRow), only the stored suffix).
#[derive(Clone, Serialize)]
pub struct PaymentIntentWithSecret {
    /// The twelve documented keys, unchanged and rendered by the same code
    /// every other surface uses.
    #[serde(flatten)]
    pub intent: PaymentIntentObject,
    /// `pi_…_secret_…` — `vpay_core::ids::client_secret` of the row's `id`
    /// and its `client_secret_suffix`.
    ///
    /// Not `Option<String>` although Stripe's is nullable: every route that
    /// renders this type has the row in hand, and the row's suffix is `NOT
    /// NULL` (migration `0026`). A nullable field would model a state the
    /// schema forbids and would let a future caller ship `null` to a browser
    /// that has nothing to do with it.
    pub client_secret: String,
}

/// Redacts [`PaymentIntentWithSecret::client_secret`], leaving the rendered
/// intent (already the public wire shape) visible.
///
/// This struct is `{:?}`-ed wherever a handler logs "what would have been
/// sent" or a test failure prints an assertion's left-hand side, and unlike
/// `vpay_db::PaymentIntentRow::client_secret_suffix` — half a credential,
/// useless without the row's `id` — `client_secret` here is the **whole**
/// bearer token `/v1/browser` accepts. A derived `Debug` would put it in a
/// log line as readily as `println!("{secret}")` would.
///
/// The length stays, matching `PaymentIntentRow`'s impl, because "is this
/// the right shape?" is a legitimate debugging question that needs no
/// secret to answer.
impl std::fmt::Debug for PaymentIntentWithSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentIntentWithSecret")
            .field("intent", &self.intent)
            .field(
                "client_secret",
                &format_args!("[{} chars redacted]", self.client_secret.len()),
            )
            .finish()
    }
}

impl PaymentIntentWithSecret {
    /// Pairs a rendered object with the secret derived from the row it came
    /// from.
    ///
    /// Takes the already-rendered object rather than the row, because two
    /// of the four callers have adjusted `next_action` first
    /// ([`PaymentIntentObject::with_next_action`]) and a constructor that
    /// re-derived from the row would silently drop that.
    #[must_use]
    pub fn new(intent: PaymentIntentObject, client_secret: String) -> Self {
        Self {
            intent,
            client_secret,
        }
    }
}

/// Stripe's `list` envelope.
///
/// Generic over the element type although only `payment_intent` is listed
/// today: `refund` and `event` are the same envelope
/// (`docs/api/README.md`), and a second hand-written copy of these four keys
/// is how `has_more` ends up meaning different things on different endpoints.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListObject<T> {
    /// Always `"list"`.
    pub object: ListTag,
    /// This page's objects, **newest first** — see D8: `ending_before` pages
    /// backwards by querying ascending and reversing, so `data` is ordered the
    /// same way whichever cursor was used.
    pub data: Vec<T>,
    /// Whether another page exists after this one. Answered by asking the
    /// database for `limit + 1` rows, never by comparing `data.len()` to the
    /// limit — the latter says `true` on a final page that happens to be full.
    pub has_more: bool,
    /// The path this list was read from, without the query string.
    pub url: String,
}

impl<T> ListObject<T> {
    /// Builds the envelope. The only constructor, so `object` is always
    /// `"list"` and `url` is always supplied by the route that served it.
    #[must_use]
    pub fn new(data: Vec<T>, has_more: bool, url: impl Into<String>) -> Self {
        Self {
            object: ListTag,
            data,
            has_more,
            url: url.into(),
        }
    }
}

/// Renders a row's `metadata`, which the `metadata_is_object` CHECK
/// (migration 0014) guarantees is a JSON object.
///
/// A non-object would mean that CHECK is gone or was bypassed, which is this
/// layer's invariant failing rather than anything a merchant did — hence
/// `Internal`, which pages, rather than an empty object that would quietly
/// tell the merchant their metadata was lost.
fn metadata_of(value: &Value) -> Result<Map<String, Value>, ApiError> {
    value.as_object().cloned().ok_or_else(|| {
        ApiError::Internal(format!(
            "payment_intents.metadata is {} rather than an object",
            kind_of(value)
        ))
    })
}

/// Renders a row's `payment_method_types`, which the `pmt_is_array` CHECK
/// guarantees is a JSON array — of rail codes, which are strings.
fn payment_method_types_of(value: &Value) -> Result<Vec<String>, ApiError> {
    let items = value.as_array().ok_or_else(|| {
        ApiError::Internal(format!(
            "payment_intents.payment_method_types is {} rather than an array",
            kind_of(value)
        ))
    })?;
    items
        .iter()
        .map(|item| {
            item.as_str().map(str::to_owned).ok_or_else(|| {
                ApiError::Internal(format!(
                    "payment_intents.payment_method_types holds {} rather than a rail code",
                    kind_of(item)
                ))
            })
        })
        .collect()
}

/// The JSON type name, for an operator-facing message. `serde_json::Value` has
/// no `Display` for its discriminant and printing the *value* here would put a
/// merchant's metadata into a log line.
fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Pairs a row's `last_payment_error_code`/`_message` columns, which the
/// `lpe_paired` CHECK keeps either both null or both present.
fn last_payment_error_of(
    code: Option<&String>,
    message: Option<&String>,
) -> Result<Option<LastPaymentErrorObject>, ApiError> {
    match (code, message) {
        (None, None) => Ok(None),
        (Some(code), Some(message)) => Ok(Some(LastPaymentErrorObject {
            code: code.clone(),
            message: message.clone(),
        })),
        // Half a failure is not something to render as a whole one, and not
        // something to drop either: the pairing is a database CHECK, so seeing
        // it broken means the schema is not what this code believes.
        _ => Err(ApiError::Internal(
            "payment_intents.last_payment_error_{code,message} are not both set or both null"
                .to_owned(),
        )),
    }
}

/// The envelope around an event's payload: `data.object`, and nothing else.
///
/// A struct rather than an inline `Value` so the nesting is impossible to get
/// wrong: `sdks/rust/src/model.rs`'s `EventData` and
/// `sdks/nodejs/src/types.ts` both require the extra `object` level, and an
/// event that put the payload directly under `data` would fail to decode in
/// every merchant's client while still looking plausible in a log.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventDataObject {
    /// The object the event is about, verbatim from `events.data` — the
    /// snapshot taken when the transition happened, never a re-read of
    /// whatever is true now.
    ///
    /// A `Value` rather than a typed union because the payload is a
    /// `payment_intent` today and a `refund` tomorrow, and because both SDKs
    /// deliberately keep it as raw JSON with typed accessors
    /// (`docs/flows/merchant-auth.md`): an event naming an object type a
    /// merchant's SDK version predates must still be *deliverable*, not a
    /// decode failure in their handler.
    pub object: Value,
}

/// An `event`, as `GET /v1/events` serves it **and** as the webhook
/// deliverer signs and sends it.
///
/// # One renderer, two surfaces, on purpose
///
/// `vpay_worker::webhooks::event_bytes` serialises this same type to get the
/// bytes it signs. That is the whole reason it lives here rather than in the
/// worker: a merchant who misses a webhook is told to re-read the event from
/// `GET /v1/events` (`docs/api/README.md`), and if the two surfaces rendered
/// an event differently, the fallback would answer a different question from
/// the one the webhook asked. A second hand-written copy of these six keys is
/// how `created` ends up in milliseconds on one of them.
///
/// # Why serialisation is deterministic, and why that matters here
///
/// The delivered bytes are signed, and `payload_sha256` on the delivery row
/// asserts that attempt two renders exactly what attempt one signed. That
/// holds because every key below is a fixed struct field in a fixed order and
/// `serde_json::Map` is a `BTreeMap` in this workspace (no `preserve_order`
/// feature anywhere in the graph), so `data.object`'s keys serialise in sorted
/// order however the `Value` was built. Turning that feature on would make
/// the digest depend on JSON parse order — see
/// `the_rendered_bytes_are_stable_whatever_order_the_data_was_built_in`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventObject {
    /// `evt_…`. Delivery is at-least-once, so this is the id merchants
    /// dedupe on (`docs/flows/webhooks.md`).
    pub id: String,
    /// Always `"event"`.
    pub object: EventTag,
    /// One of the real Stripe event types `docs/flows/webhooks.md` commits
    /// to. The wire field is `type`, which is a Rust keyword.
    ///
    /// A `String` rather than a closed enum for the reason both SDKs give:
    /// the vocabulary is closed by the database (`type_is_a_documented_event`,
    /// migration 0018) where it is *written*, and a value that failed to parse
    /// on the read path would turn a merchant's `GET /v1/events` into a 500
    /// instead of showing them the event.
    #[serde(rename = "type")]
    pub kind: String,
    /// Unix **seconds**, like every other `created` on this API.
    pub created: i64,
    /// Taken from the row, not from configuration read at render time: an
    /// event describes what was true when it happened, and a redeployment
    /// must not change what a delivered webhook says about itself.
    pub livemode: bool,
    /// The payload.
    pub data: EventDataObject,
}

impl TryFrom<&vpay_db::EventRow> for EventObject {
    type Error = ApiError;

    /// Renders a stored event as the object a merchant reads — over
    /// `GET /v1/events` or in a webhook body.
    ///
    /// Fallible only for a state migration 0018's `data_is_object` CHECK
    /// makes impossible: a `data` that is not a JSON object cannot be nested
    /// under `data.object` without changing what the event *means*, so it is
    /// `Internal` (500, pages) and never a `null` payload quietly delivered to
    /// a merchant's handler. Nothing a caller can send reaches an `Err` here.
    fn try_from(row: &vpay_db::EventRow) -> Result<Self, Self::Error> {
        if !row.data.is_object() {
            return Err(ApiError::Internal(format!(
                "events.data is {} rather than an object",
                kind_of(&row.data)
            )));
        }

        Ok(Self {
            id: row.id.clone(),
            object: EventTag,
            kind: row.event_type.clone(),
            created: row.created_at.unix_timestamp(),
            livemode: row.livemode,
            data: EventDataObject {
                // Verbatim. Not re-validated against a payment-intent shape:
                // the snapshot was rendered by this same module when the
                // transition happened, and re-deriving it now would deliver
                // whatever is true today under a timestamp that says
                // otherwise.
                object: row.data.clone(),
            },
        })
    }
}

impl TryFrom<&vpay_db::PaymentIntentRow> for PaymentIntentObject {
    type Error = ApiError;

    /// Renders a stored intent as the object a merchant reads.
    ///
    /// Fallible only for states the database should make impossible — see this
    /// module's header. Nothing a *caller* can send reaches an `Err` here.
    fn try_from(row: &vpay_db::PaymentIntentRow) -> Result<Self, Self::Error> {
        let status = IntentStatus::from_wire(&row.status).ok_or_else(|| {
            // Not `row.status` verbatim in a public message — `Internal`'s
            // payload is logged and never rendered, which is exactly where a
            // schema/code mismatch belongs.
            ApiError::Internal(format!(
                "payment_intents.status holds `{}`, which is not an IntentStatus",
                row.status
            ))
        })?;

        Ok(Self {
            id: row.id.clone(),
            object: PaymentIntentTag,
            amount: row.amount,
            currency: row.currency_code.to_lowercase(),
            status,
            payment_method_types: payment_method_types_of(&row.payment_method_types)?,
            // Always `null` here: `next_action` lives on the charge, not the
            // intent. See `with_next_action`.
            next_action: None,
            last_payment_error: last_payment_error_of(
                row.last_payment_error_code.as_ref(),
                row.last_payment_error_message.as_ref(),
            )?,
            metadata: metadata_of(&row.metadata)?,
            description: row.description.clone(),
            created: row.created_at.unix_timestamp(),
            livemode: row.livemode,
        })
    }
}

/// The `metadata` shape a *handler* builds an object from when it has plain
/// string pairs rather than a row — kept here so the one conversion from
/// `BTreeMap<String, String>` to the wire shape is written once.
#[must_use]
pub fn metadata_from_pairs(pairs: &BTreeMap<String, String>) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The fixture `sdks/rust/tests/support/mod.rs`'s `payment_intent_json`
    /// serves to the SDK's own tests, pasted verbatim. It is the shape a
    /// shipping merchant client is written against, so it — not this module —
    /// is the specification.
    fn sdk_fixture(id: &str) -> Value {
        json!({
            "id": id,
            "object": "payment_intent",
            "amount": 5000,
            "currency": "xaf",
            "status": "requires_payment_method",
            "payment_method_types": ["mtn_momo"],
            "next_action": null,
            "last_payment_error": null,
            "metadata": { "order_id": "1234" },
            "description": null,
            "created": 1_753_401_600,
            "livemode": false,
        })
    }

    fn sample(id: &str) -> PaymentIntentObject {
        PaymentIntentObject {
            id: id.to_owned(),
            object: PaymentIntentTag,
            amount: 5000,
            currency: "xaf".to_owned(),
            status: IntentStatus::RequiresPaymentMethod,
            payment_method_types: vec!["mtn_momo".to_owned()],
            next_action: None,
            last_payment_error: None,
            metadata: metadata_from_pairs(&BTreeMap::from([(
                "order_id".to_owned(),
                "1234".to_owned(),
            )])),
            description: None,
            created: 1_753_401_600,
            livemode: false,
        }
    }

    #[test]
    fn the_wire_object_is_the_sdk_fixture() {
        let rendered =
            serde_json::to_value(sample("pi_3MtwBwLkdIwHu7ix28a3tqPa")).expect("serialises");
        assert_eq!(rendered, sdk_fixture("pi_3MtwBwLkdIwHu7ix28a3tqPa"));
    }

    /// `serde_json::Value` comparison above normalises key order, so on its
    /// own it would also pass for an object that *omitted* `next_action`,
    /// `last_payment_error` and `description` — the SDK's `PaymentIntent` has
    /// those as required fields, so omitting them breaks a merchant's client.
    /// This asserts the keys are present and null.
    #[test]
    fn every_documented_key_is_present_including_the_null_ones() {
        let rendered = serde_json::to_value(sample("pi_1")).expect("serialises");
        let object = rendered.as_object().expect("an object");

        for key in [
            "id",
            "object",
            "amount",
            "currency",
            "status",
            "payment_method_types",
            "next_action",
            "last_payment_error",
            "metadata",
            "description",
            "created",
            "livemode",
        ] {
            assert!(object.contains_key(key), "`{key}` is missing");
        }
        assert_eq!(
            object.len(),
            12,
            "an undocumented key was added: {object:?}"
        );
        for null_key in ["next_action", "last_payment_error", "description"] {
            assert_eq!(
                object.get(null_key),
                Some(&Value::Null),
                "`{null_key}` must be present and null, not absent"
            );
        }
    }

    /// The browser wrapper is the twelve keys **plus one**, at the top level
    /// — not a nested object.
    ///
    /// `sdks/stripe-js/src/types.ts`'s `PaymentIntent` is written against
    /// exactly this shape, and its `testing/browser-stub.ts` builds it key by
    /// key. A missing `#[serde(flatten)]` would nest the intent under
    /// `intent` and every browser call would decode to something with no
    /// `status` on it.
    #[test]
    fn the_browser_wrapper_is_the_twelve_keys_plus_the_client_secret() {
        let rendered = serde_json::to_value(PaymentIntentWithSecret::new(
            sample("pi_1"),
            "pi_1_secret_abc".to_owned(),
        ))
        .expect("serialises");
        let object = rendered.as_object().expect("an object");

        assert_eq!(
            object.len(),
            13,
            "the browser object must be exactly the twelve documented keys plus \
             `client_secret` — got a different key count"
        );
        assert_eq!(
            object.get("client_secret").and_then(Value::as_str),
            Some("pi_1_secret_abc")
        );
        // Every one of the twelve is still there, at the top level, byte for
        // byte what the merchant surface renders.
        let plain = serde_json::to_value(sample("pi_1")).expect("serialises");
        let plain = plain.as_object().expect("an object");
        for (key, value) in plain {
            assert_eq!(object.get(key), Some(value), "`{key}` changed shape");
        }
    }

    /// `client_secret` here is the whole bearer token `/v1/browser` accepts
    /// — not the stored suffix `vpay_db::PaymentIntentRow` redacts, the
    /// already-joined value. A derived `Debug` would put it in a `tracing`
    /// field or a test failure message as readily as `println!` would.
    ///
    /// Decisive: replacing the hand-written impl with `#[derive(Debug)]`
    /// fails this test on its first assertion.
    #[test]
    fn a_payment_intent_with_secrets_debug_output_never_contains_the_client_secret() {
        let secret = "pi_1_secret_neverlogthispayercredential00000".to_owned();
        let with_secret = PaymentIntentWithSecret::new(sample("pi_1"), secret.clone());

        let formatted = format!("{with_secret:?}");

        assert!(
            !formatted.contains(&secret),
            "Debug output must not contain the client_secret"
        );
        // Not even a prefix of it: a redaction that truncated rather than
        // replaced would still hand a guesser most of the credential.
        assert!(
            !formatted.contains("neverlog"),
            "Debug output must not contain even a prefix of the client_secret"
        );
        assert!(
            formatted.contains(&format!("[{} chars redacted]", secret.len())),
            "Debug output must contain the redaction marker"
        );
        // The rendered intent itself carries no secret, so it stays visible
        // — an operator's `{:?}` still shows the status and amount.
        assert!(
            formatted.contains("pi_1"),
            "Debug output must still show the non-secret intent fields"
        );
    }

    /// The tripwire, restated from the other side: adding the credential to
    /// the *wrapper* must not add it to the type the list and every webhook
    /// body render through.
    ///
    /// `every_documented_key_is_present_including_the_null_ones` already
    /// pins `len() == 12`; this says out loud what that number is protecting,
    /// so a future reader tempted to "just add the field" finds the reason
    /// rather than only the assertion.
    #[test]
    fn the_plain_payment_intent_object_still_carries_no_client_secret() {
        let rendered = serde_json::to_value(sample("pi_1")).expect("serialises");
        let object = rendered.as_object().expect("an object");
        assert!(
            !object.contains_key("client_secret"),
            "a payer credential must not reach the /v1 list or events.data: {object:?}"
        );
        assert_eq!(object.len(), 12);
    }

    /// The contract's whole point: a merchant's own client decodes what this
    /// renders. `vpay_sdk::PaymentIntent` is the shipping Rust SDK's type, not
    /// a copy of it written for this test.
    #[test]
    fn the_merchant_sdk_deserialises_what_this_renders() {
        let rendered = serde_json::to_string(&sample("pi_abc")).expect("serialises");
        let decoded: vpay_sdk::PaymentIntent =
            serde_json::from_str(&rendered).expect("the SDK decodes the object vpay renders");

        assert_eq!(decoded.id, "pi_abc");
        assert_eq!(decoded.object, "payment_intent");
        assert_eq!(decoded.amount, 5000);
        assert_eq!(decoded.currency, "xaf");
        assert_eq!(
            decoded.status,
            vpay_sdk::IntentStatus::RequiresPaymentMethod
        );
        assert_eq!(decoded.payment_method_types, vec!["mtn_momo"]);
        assert_eq!(decoded.next_action, None);
        assert_eq!(decoded.last_payment_error, None);
        assert_eq!(decoded.description, None);
        assert_eq!(decoded.created, 1_753_401_600);
        assert!(!decoded.livemode);
        assert_eq!(
            decoded.metadata.get("order_id").map(String::as_str),
            Some("1234")
        );
    }

    #[test]
    fn the_object_discriminator_cannot_be_anything_else() {
        assert_eq!(
            serde_json::to_value(PaymentIntentTag).expect("serialises"),
            json!("payment_intent")
        );
        assert_eq!(
            serde_json::to_value(ListTag).expect("serialises"),
            json!("list")
        );
    }

    #[test]
    fn a_redirect_rails_next_action_is_stripes_shape() {
        let object = sample("pi_1").with_next_action(Some(NextAction::RedirectToUrl {
            redirect_to_url: RedirectToUrl {
                url: "https://webpayment.orange-money.test/pay/abc".to_owned(),
                return_url: Some("https://shop.example/order/1234/return".to_owned()),
            },
        }));
        let rendered = serde_json::to_value(&object).expect("serialises");
        assert_eq!(
            rendered.get("next_action"),
            Some(&json!({
                "type": "redirect_to_url",
                "redirect_to_url": {
                    "url": "https://webpayment.orange-money.test/pay/abc",
                    "return_url": "https://shop.example/order/1234/return"
                }
            }))
        );

        // And the SDK decodes it into its own `NextAction`.
        let decoded: vpay_sdk::PaymentIntent =
            serde_json::from_value(rendered).expect("the SDK decodes a redirect next_action");
        assert!(matches!(
            decoded.next_action,
            Some(vpay_sdk::NextAction::RedirectToUrl { .. })
        ));
    }

    #[test]
    fn a_return_url_the_rail_was_not_given_is_null_rather_than_absent() {
        let object = sample("pi_1").with_next_action(Some(NextAction::RedirectToUrl {
            redirect_to_url: RedirectToUrl {
                url: "https://webpayment.orange-money.test/pay/abc".to_owned(),
                return_url: None,
            },
        }));
        let rendered = serde_json::to_value(object).expect("serialises");
        assert_eq!(
            rendered
                .get("next_action")
                .and_then(|n| n.get("redirect_to_url"))
                .and_then(|r| r.get("return_url")),
            Some(&Value::Null)
        );
    }

    #[test]
    fn a_last_payment_error_renders_the_failure_vocabulary() {
        let mut object = sample("pi_1");
        object.last_payment_error = Some(LastPaymentErrorObject {
            code: "insufficient_funds".to_owned(),
            message: "The payer's balance was too low.".to_owned(),
        });
        // Still `requires_payment_method`: there is no `failed` status
        // (docs/flows/payment-lifecycle.md).
        assert_eq!(object.status, IntentStatus::RequiresPaymentMethod);

        let rendered = serde_json::to_value(&object).expect("serialises");
        assert_eq!(
            rendered.get("last_payment_error"),
            Some(&json!({
                "code": "insufficient_funds",
                "message": "The payer's balance was too low."
            }))
        );
        let decoded: vpay_sdk::PaymentIntent =
            serde_json::from_value(rendered).expect("the SDK decodes a last_payment_error");
        assert_eq!(
            decoded.last_payment_error.map(|e| e.code),
            Some("insufficient_funds".to_owned())
        );
    }

    #[test]
    fn the_status_renders_in_the_vocabulary_the_sdk_closes_over() {
        for (status, wire) in [
            (
                IntentStatus::RequiresPaymentMethod,
                "requires_payment_method",
            ),
            (IntentStatus::RequiresAction, "requires_action"),
            (IntentStatus::Processing, "processing"),
            (IntentStatus::Succeeded, "succeeded"),
            (IntentStatus::Canceled, "canceled"),
        ] {
            let mut object = sample("pi_1");
            object.status = status;
            let rendered = serde_json::to_value(&object).expect("serialises");
            assert_eq!(rendered.get("status"), Some(&json!(wire)), "{status:?}");
            serde_json::from_value::<vpay_sdk::PaymentIntent>(rendered)
                .unwrap_or_else(|e| panic!("the SDK must decode {wire}: {e}"));
        }
    }

    #[test]
    fn the_list_envelope_is_stripes_four_keys() {
        let list = ListObject::new(
            vec![sample("pi_2"), sample("pi_1")],
            true,
            "/v1/payment_intents",
        );
        let rendered = serde_json::to_value(&list).expect("serialises");
        let object = rendered.as_object().expect("an object");
        assert_eq!(object.len(), 4, "{object:?}");
        assert_eq!(object.get("object"), Some(&json!("list")));
        assert_eq!(object.get("has_more"), Some(&json!(true)));
        assert_eq!(object.get("url"), Some(&json!("/v1/payment_intents")));
        assert_eq!(
            object
                .get("data")
                .and_then(Value::as_array)
                .map(|d| d.len()),
            Some(2)
        );

        let decoded: vpay_sdk::List<vpay_sdk::PaymentIntent> =
            serde_json::from_value(rendered).expect("the SDK decodes the list envelope");
        assert!(decoded.has_more);
        assert_eq!(decoded.object, "list");
        // Newest first, as sent — the SDK does not reorder.
        assert_eq!(
            decoded.data.first().map(|p| p.id.clone()),
            Some("pi_2".to_owned())
        );
    }

    #[test]
    fn an_empty_page_is_an_empty_array_not_null() {
        let list: ListObject<PaymentIntentObject> =
            ListObject::new(Vec::new(), false, "/v1/payment_intents");
        let rendered = serde_json::to_value(list).expect("serialises");
        assert_eq!(rendered.get("data"), Some(&json!([])));
        assert_eq!(rendered.get("has_more"), Some(&json!(false)));
    }

    // ---------------------------------------------------------- events ----

    fn event_row(data: Value) -> vpay_db::EventRow {
        vpay_db::EventRow {
            id: "evt_1".to_owned(),
            seq: 12,
            // Never rendered: a merchant already knows who they are, and the
            // scope is the authorisation, not a field on the object.
            merchant_id: "merchant_a".to_owned(),
            livemode: false,
            event_type: "payment_intent.succeeded".to_owned(),
            object_id: "pi_1".to_owned(),
            data,
            fanout_state: "pending".to_owned(),
            created_at: time::OffsetDateTime::from_unix_timestamp(1_753_401_600)
                .expect("a fixed, valid timestamp"),
        }
    }

    /// The envelope `sdks/rust/src/model.rs`'s `Event` requires, key for key.
    #[test]
    fn an_event_renders_the_documented_envelope() {
        let row = event_row(json!({ "id": "pi_1", "object": "payment_intent" }));
        let object = EventObject::try_from(&row).expect("a well-formed row renders");
        let rendered = serde_json::to_value(&object).expect("serialises");

        assert_eq!(
            rendered,
            json!({
                "id": "evt_1",
                "object": "event",
                "type": "payment_intent.succeeded",
                "created": 1_753_401_600,
                "livemode": false,
                "data": { "object": { "id": "pi_1", "object": "payment_intent" } },
            })
        );
        // The row's own bookkeeping stays on the row. `seq` is a fan-out
        // cursor and `merchant_id` is the scope that authorised the read;
        // neither is a merchant's business, and `fanout_state` would tell
        // them about vpay's queue.
        let keys: Vec<&String> = rendered
            .as_object()
            .expect("an object")
            .keys()
            .collect::<Vec<_>>();
        for leaked in ["seq", "merchant_id", "fanout_state", "object_id"] {
            assert!(
                !keys.iter().any(|k| k.as_str() == leaked),
                "{leaked} leaked"
            );
        }
    }

    /// The decisive one for the deliverer: the bytes vpay signs must decode
    /// as `vpay_sdk::Event` in the SDK a merchant installs. Drop
    /// `object: "event"` — or rename `type`, or send `created` as a string —
    /// and this fails, which is the whole point of rendering the webhook body
    /// through this type instead of a `json!` in the worker.
    #[test]
    fn the_rendered_event_decodes_through_the_shipping_sdk() {
        let row = event_row(json!({ "id": "pi_1", "amount": 5000 }));
        let object = EventObject::try_from(&row).expect("renders");
        let bytes = serde_json::to_vec(&object).expect("serialises");

        let decoded: vpay_sdk::Event =
            serde_json::from_slice(&bytes).expect("the shipping SDK decodes the event envelope");
        assert_eq!(decoded.id, "evt_1");
        assert_eq!(decoded.object, "event");
        assert_eq!(decoded.kind, "payment_intent.succeeded");
        assert_eq!(decoded.created, 1_753_401_600);
        assert!(!decoded.livemode);
        assert_eq!(decoded.data.object.get("id"), Some(&json!("pi_1")));
    }

    /// The signature covers *bytes*, and `payload_sha256` asserts attempt two
    /// produces the same ones as attempt one. That only holds if key order is
    /// a function of the content and not of how the `Value` was built —
    /// `serde_json::Map` is a `BTreeMap` here because nothing in the graph
    /// enables `preserve_order`. If someone turns that feature on, this test
    /// fails rather than deliveries silently becoming `Poisoned` on retry.
    #[test]
    fn the_rendered_bytes_are_stable_whatever_order_the_data_was_built_in() {
        let one = serde_json::from_str::<Value>(r#"{"b":2,"a":1}"#).expect("valid JSON");
        let two = serde_json::from_str::<Value>(r#"{"a":1,"b":2}"#).expect("valid JSON");

        let first = serde_json::to_vec(&EventObject::try_from(&event_row(one)).expect("renders"))
            .expect("serialises");
        let second = serde_json::to_vec(&EventObject::try_from(&event_row(two)).expect("renders"))
            .expect("serialises");

        assert_eq!(first, second);
        assert!(
            String::from_utf8(first)
                .expect("ASCII JSON")
                .contains(r#""data":{"object":{"a":1,"b":2}}"#)
        );
    }

    /// `data_is_object` (migration 0018) makes this unreachable from the
    /// database. It is `Internal` rather than a rendered `null`, because a
    /// delivered `data.object: null` is a webhook a merchant's handler
    /// crashes on with no way to tell it apart from a real event.
    #[test]
    fn an_event_whose_data_is_not_an_object_is_internal_not_a_null_payload() {
        for bad in [json!(null), json!([1, 2]), json!("pi_1"), json!(7)] {
            let error = EventObject::try_from(&event_row(bad.clone()))
                .expect_err("a non-object data must not render");
            assert!(
                matches!(error, ApiError::Internal(ref m) if m.contains("events.data")),
                "{bad} gave {error:?}"
            );
        }
    }
}
