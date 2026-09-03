//! The `/v1` object model, per `docs/flows/merchant-auth.md`'s "Objects"
//! table — Stripe's own field names and shapes, so a merchant's existing
//! Stripe types keep working. `docs/flows/payment-lifecycle.md` and
//! `docs/flows/failures.md` are the sources for [`IntentStatus`]'s five
//! variants and [`LastPaymentError`]'s `code` respectively.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A `PaymentIntent`'s lifecycle state.
///
/// Exactly the five values `vpay_core::state::IntentStatus` defines — there
/// is deliberately no `failed`: a rail-reported failure returns the intent
/// to `requires_payment_method` with [`PaymentIntent::last_payment_error`]
/// populated instead (`docs/flows/payment-lifecycle.md`). This SDK defines
/// its own copy rather than depending on `vpay-core` — a merchant-facing
/// crate mirrors the *wire* contract, not vpay's internal crate graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    /// No instrument attached yet — or the last one was refused, with
    /// [`PaymentIntent::last_payment_error`] saying why.
    RequiresPaymentMethod,
    /// Redirect rails only; carries [`PaymentIntent::next_action`].
    RequiresAction,
    /// Submitted to the rail. **Not** settled: wait for
    /// `payment_intent.succeeded` rather than treating this as payment.
    Processing,
    /// Settled.
    Succeeded,
    /// Withdrawn. One charge per intent, forever — a retry is a new intent.
    Canceled,
}

/// The rail codes `payment_method_types` may name.
///
/// Closed, and `#[non_exhaustive]` so adding a rail is not a breaking change
/// for a caller who matched on it. `sdks/nodejs/src/types.ts` closes the same
/// list (`PaymentMethodType = "mtn_momo" | "orange_money"`), and it is closed
/// on the *request* side only: a rail this SDK version predates must still be
/// readable in a response, so [`PaymentIntent::payment_method_types`] stays
/// `Vec<String>` exactly as Node's `PaymentIntent.payment_method_types` stays
/// `string[]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PaymentMethodType {
    /// MTN Mobile Money — a push rail: the payer approves on their handset.
    MtnMomo,
    /// Orange Money — a redirect rail: the payer is sent to the rail's page.
    OrangeMoney,
}

impl PaymentMethodType {
    /// The exact string this rail is named by on the wire.
    ///
    /// Hand-written beside the `serde` rename rather than derived from it:
    /// the form encoder does not go through `serde` (see `crate::form`), so
    /// without this the wire spelling would exist in only one of the two
    /// paths.
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            PaymentMethodType::MtnMomo => "mtn_momo",
            PaymentMethodType::OrangeMoney => "orange_money",
        }
    }
}

impl std::fmt::Display for PaymentMethodType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire_str())
    }
}

/// Where to send a payer on a redirect rail (Orange Money).
///
/// Mirrors Stripe's own `next_action.redirect_to_url` shape so a merchant's
/// existing redirect-handling code works unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedirectToUrl {
    /// Where to send the payer. Opaque — never parsed or rewritten.
    pub url: String,
    /// Where the rail returns the payer afterwards; `null` if the rail was
    /// not given one.
    pub return_url: Option<String>,
}

/// What a payer must do next. Only ever `redirect_to_url` today — push
/// rails never populate [`PaymentIntent::next_action`] at all, because there
/// is nothing for a browser to do while a payer types a PIN into their own
/// handset (`docs/flows/payment-lifecycle.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NextAction {
    /// Send the payer to [`RedirectToUrl::url`].
    RedirectToUrl {
        /// The destination and the return URL the rail was given.
        redirect_to_url: RedirectToUrl,
    },
}

/// Why a charge failed, in the closed vocabulary `docs/flows/failures.md`
/// owns. Kept as a plain `String` rather than a closed Rust enum: the
/// vocabulary is owned by vpay's core and may grow a code this SDK predates,
/// and a `#[serde(other)]` fallback variant would still lose the original
/// string — a `String` field never can.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastPaymentError {
    /// The failure code, from `docs/flows/failures.md`'s vocabulary. A
    /// `String`, deliberately — see this type's own documentation.
    pub code: String,
    /// The human-readable detail the rail's failure mapped to.
    pub message: String,
}

/// A payment attempt against `/v1/payment_intents`.
///
/// `Debug` is **hand-written** below rather than derived, because
/// [`Self::client_secret`] — when present — is a live payer credential.
/// Mirrors `vpay_api::model::PaymentIntentWithSecret`'s own hand-written
/// impl in shape and in what it redacts, and `Credentials`'/`Client`'s impls
/// in this crate for the same reason — see `tests/debug_redaction.rs`.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct PaymentIntent {
    /// `pi_…`.
    pub id: String,
    /// Always `"payment_intent"`. Carried verbatim rather than asserted, so
    /// an unexpected value is visible to the caller instead of becoming a
    /// decode failure.
    pub object: String,
    /// Minor units — `docs/flows/money.md`. `5000` on a `xaf` intent is
    /// 5,000 FCFA, because XAF is zero-decimal.
    pub amount: i64,
    /// Lowercase on the wire, e.g. `"xaf"`.
    pub currency: String,
    /// The lifecycle state; there is no `failed` — see [`IntentStatus`].
    pub status: IntentStatus,
    /// The rail codes this intent may be confirmed against.
    ///
    /// `Vec<String>`, not `Vec<`[`PaymentMethodType`]`>`: a response naming a
    /// rail this SDK version predates must still decode. The *request* side
    /// ([`crate::CreatePaymentIntentParams::payment_method_types`]) is the
    /// closed enum, matching the Node SDK's split exactly.
    pub payment_method_types: Vec<String>,
    /// Redirect rails only; `None` on a push rail, which has nothing for a
    /// browser to do.
    pub next_action: Option<NextAction>,
    /// The last rail refusal, if any. Present *with*
    /// [`IntentStatus::RequiresPaymentMethod`] — never with a `failed`
    /// status, because there is none.
    pub last_payment_error: Option<LastPaymentError>,
    /// The merchant's own key/value pairs, echoed back.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// The merchant's own description, or `None`.
    pub description: Option<String>,
    /// Unix seconds.
    pub created: i64,
    /// `false` for a sandbox deployment's objects.
    pub livemode: bool,
    /// `pi_…_secret_…` — the payer credential `/v1/browser` accepts to
    /// confirm this intent from a browser (`@vpay/stripe-js`).
    ///
    /// Present only on `POST /v1/payment_intents` and
    /// `GET /v1/payment_intents/{id}` responses (Step 5c's D2,
    /// `vpay_api::model::PaymentIntentWithSecret`) — **never** on a
    /// `list()` item or an [`Event`]'s `data.object`, because either would
    /// hand a merchant's own integration (or a webhook body, signed and
    /// stored forever) a live credential for every intent in view. Those
    /// two responses render the plain `PaymentIntentObject`, which never
    /// carries the key at all, so `#[serde(default)]` is what makes this
    /// field decode to `None` there rather than failing.
    ///
    /// Never log this value: see this type's hand-written `impl Debug`.
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Redacts [`PaymentIntent::client_secret`] — the one field on this type
/// that is a live credential rather than public state — leaving every other
/// field visible exactly as a derived `Debug` would render it.
impl std::fmt::Debug for PaymentIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentIntent")
            .field("id", &self.id)
            .field("object", &self.object)
            .field("amount", &self.amount)
            .field("currency", &self.currency)
            .field("status", &self.status)
            .field("payment_method_types", &self.payment_method_types)
            .field("next_action", &self.next_action)
            .field("last_payment_error", &self.last_payment_error)
            .field("metadata", &self.metadata)
            .field("description", &self.description)
            .field("created", &self.created)
            .field("livemode", &self.livemode)
            .field(
                "client_secret",
                &self
                    .client_secret
                    .as_ref()
                    .map(|s| format!("[{} chars redacted]", s.len())),
            )
            .finish()
    }
}

/// A refund's lifecycle state. Independent of [`IntentStatus`] — a refund
/// never changes its intent's status (`docs/flows/payment-lifecycle.md`:
/// "Refunds do not change intent status").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundStatus {
    /// Submitted to the rail; not yet money back.
    Pending,
    /// The rail returned the funds.
    Succeeded,
    /// The rail refused the refund. Unlike an intent, a refund *does* have a
    /// terminal failure state.
    Failed,
    /// Withdrawn before the rail acted on it.
    Canceled,
}

/// A `/v1/refunds` object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Refund {
    /// `re_…`.
    pub id: String,
    /// Always `"refund"`.
    pub object: String,
    /// Minor units, like every amount here (`docs/flows/money.md`).
    pub amount: i64,
    /// Lowercase on the wire, e.g. `"xaf"`.
    pub currency: String,
    /// The `pi_…` this refunds.
    pub payment_intent: String,
    /// The refund's own state — independent of the intent's.
    pub status: RefundStatus,
    /// The merchant-supplied reason, or `None`.
    pub reason: Option<String>,
    /// The merchant's own key/value pairs, echoed back.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Unix seconds.
    pub created: i64,
}

/// The payload carried by an [`Event`] — kept as raw JSON with typed
/// accessors on [`Event`] itself, so an event naming an object type this
/// version of the SDK does not model is still deliverable rather than a
/// deserialization failure (`docs/flows/merchant-auth.md`: "the SDKs keep
/// `data.object` as raw JSON with typed accessors").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventData {
    /// The object this event is about, kept as raw JSON. Use
    /// [`Event::payment_intent`]/[`Event::refund`] to decode it.
    pub object: serde_json::Value,
}

/// A `/v1/events` object — one of the real Stripe event types
/// `docs/flows/webhooks.md` commits to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// `evt_…`. Delivery is at-least-once — dedupe on this, see
    /// [`crate::webhooks`].
    pub id: String,
    /// Always `"event"`.
    pub object: String,
    /// The wire field is `type`, which is a Rust keyword; `kind` is the
    /// field name and `#[serde(rename)]` keeps the wire spelling.
    ///
    /// A `String` rather than a closed enum: an event type this SDK version
    /// predates must still be deliverable
    /// (`docs/flows/merchant-auth.md`, "Objects").
    #[serde(rename = "type")]
    pub kind: String,
    /// Unix seconds.
    pub created: i64,
    /// `false` for a sandbox deployment's events.
    pub livemode: bool,
    /// The event payload.
    pub data: EventData,
}

impl Event {
    /// Decodes [`EventData::object`] as a [`PaymentIntent`], for a
    /// `payment_intent.*` event.
    ///
    /// # Errors
    /// [`crate::Error::UnexpectedResponse`] if the object does not decode as
    /// a `PaymentIntent` — a bounded, generic-looking status/body pair
    /// rather than a bespoke variant, because a decode mismatch here is the
    /// same *kind* of surprise as a malformed HTTP response: the caller
    /// asked for a shape the payload does not have.
    pub fn payment_intent(&self) -> Result<PaymentIntent, crate::Error> {
        serde_json::from_value(self.data.object.clone()).map_err(|e| {
            crate::Error::UnexpectedResponse {
                status: 0,
                body_prefix: format!("event.data.object did not decode as a payment_intent: {e}"),
            }
        })
    }

    /// Decodes [`EventData::object`] as a [`Refund`], for a `charge.refund*`
    /// event.
    ///
    /// # Errors
    /// See [`Event::payment_intent`].
    pub fn refund(&self) -> Result<Refund, crate::Error> {
        serde_json::from_value(self.data.object.clone()).map_err(|e| {
            crate::Error::UnexpectedResponse {
                status: 0,
                body_prefix: format!("event.data.object did not decode as a refund: {e}"),
            }
        })
    }
}

/// A merchant's available/pending balance, per currency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Balance {
    /// Always `"balance"`.
    pub object: String,
    /// Settled funds, one entry per currency.
    pub available: Vec<BalanceEntry>,
    /// Funds not yet settled, one entry per currency.
    pub pending: Vec<BalanceEntry>,
}

/// One currency's amount within a [`Balance`] bucket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceEntry {
    /// Minor units.
    pub amount: i64,
    /// Lowercase on the wire, e.g. `"xaf"`.
    pub currency: String,
}

/// A paginated collection, per Stripe's own `list` object shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct List<T> {
    /// Always `"list"`.
    pub object: String,
    /// This page's objects, newest first.
    pub data: Vec<T>,
    /// Whether another page exists after this one — page with
    /// `starting_after` set to the last id here.
    pub has_more: bool,
    /// The path this list was read from.
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payment_method_type_has_one_wire_spelling_by_both_routes() {
        // The rail codes are written twice — once for `serde`, once for the
        // form encoder, which does not go through `serde` (see `crate::form`).
        // If those two ever disagree, a request and a response would name the
        // same rail differently. These are the exact strings
        // `sdks/nodejs/src/types.ts` closes `PaymentMethodType` over.
        for (value, wire) in [
            (PaymentMethodType::MtnMomo, "mtn_momo"),
            (PaymentMethodType::OrangeMoney, "orange_money"),
        ] {
            assert_eq!(value.as_wire_str(), wire);
            assert_eq!(value.to_string(), wire);
            assert_eq!(
                serde_json::to_string(&value).expect("serializes"),
                format!("\"{wire}\"")
            );
            assert_eq!(
                serde_json::from_str::<PaymentMethodType>(&format!("\"{wire}\""))
                    .expect("deserializes"),
                value
            );
        }
    }

    #[test]
    fn an_unknown_rail_code_is_still_readable_in_a_response() {
        // The closed enum is a *request*-side type. A response naming a rail
        // this SDK version predates must decode, not fail — which is why
        // `PaymentIntent::payment_method_types` is `Vec<String>`.
        let intent: PaymentIntent = serde_json::from_str(
            r#"{"id":"pi_1","object":"payment_intent","amount":1,"currency":"xaf",
                "status":"processing","payment_method_types":["some_future_rail"],
                "next_action":null,"last_payment_error":null,"metadata":{},
                "description":null,"created":1,"livemode":false}"#,
        )
        .expect("an unmodelled rail code still decodes");
        assert_eq!(intent.payment_method_types, vec!["some_future_rail"]);
        // Same JSON also has no `client_secret` key at all — the shape a
        // `list()` item or an event's `data.object` actually sends
        // (`vpay_api::model::PaymentIntentObject` never renders the field).
        // `#[serde(default)]` is what makes a missing key decode to `None`
        // instead of a decode failure.
        assert_eq!(intent.client_secret, None);
    }

    #[test]
    fn a_create_or_retrieve_response_carrying_client_secret_decodes_it() {
        // The shape `POST /v1/payment_intents` and `GET
        // /v1/payment_intents/{id}` actually send
        // (`vpay_api::model::PaymentIntentWithSecret`, Step 5c's D2):
        // the twelve documented keys, flattened, plus `client_secret`.
        let intent: PaymentIntent = serde_json::from_str(
            r#"{"id":"pi_1","object":"payment_intent","amount":5000,"currency":"xaf",
                "status":"requires_payment_method","payment_method_types":["mtn_momo"],
                "next_action":null,"last_payment_error":null,"metadata":{},
                "description":null,"created":1,"livemode":false,
                "client_secret":"pi_1_secret_abc123"}"#,
        )
        .expect("a create/retrieve response with client_secret decodes");
        assert_eq!(intent.client_secret.as_deref(), Some("pi_1_secret_abc123"));
    }

    #[test]
    fn client_secret_never_appears_in_debug_output_but_its_absence_or_length_does() {
        let with_secret: PaymentIntent = serde_json::from_str(
            r#"{"id":"pi_1","object":"payment_intent","amount":5000,"currency":"xaf",
                "status":"requires_payment_method","payment_method_types":["mtn_momo"],
                "next_action":null,"last_payment_error":null,"metadata":{},
                "description":null,"created":1,"livemode":false,
                "client_secret":"pi_1_secret_abc123"}"#,
        )
        .expect("decodes");
        let rendered = format!("{with_secret:?}");
        assert!(
            !rendered.contains("pi_1_secret_abc123"),
            "Debug output must not contain the client_secret"
        );
        assert!(
            rendered.contains("chars redacted"),
            "Debug output must contain the redaction marker"
        );
        // Every other field is still visible — this is a redaction, not a
        // blackout.
        assert!(rendered.contains("pi_1"));

        let without_secret: PaymentIntent = serde_json::from_str(
            r#"{"id":"pi_2","object":"payment_intent","amount":5000,"currency":"xaf",
                "status":"requires_payment_method","payment_method_types":["mtn_momo"],
                "next_action":null,"last_payment_error":null,"metadata":{},
                "description":null,"created":1,"livemode":false}"#,
        )
        .expect("decodes");
        assert_eq!(without_secret.client_secret, None);
        let rendered_none = format!("{without_secret:?}");
        assert!(rendered_none.contains("client_secret: None"));
    }
}
