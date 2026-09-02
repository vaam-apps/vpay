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
}
