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
    /// confirm this intent from a browser (`@vaam-apps/vpay-stripe-js`).
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

/// A [`CheckoutSession`]'s lifecycle state — Step 9's D10, and deliberately
/// vpay's own three values rather than Stripe's.
///
/// There is no `Failed`, for the same reason [`IntentStatus`] has none: a
/// session whose intent reached a terminal non-success state is reported
/// [`CheckoutSessionStatus::Expired`] with
/// [`CheckoutSession::payment_status`] [`CheckoutPaymentStatus::Failed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutSessionStatus {
    /// The payer has not finished. The only state `expire` accepts.
    Open,
    /// The session's intent reached `succeeded`.
    Complete,
    /// 24 h elapsed, the merchant expired it, or the intent failed
    /// terminally — the three are one status, distinguished by
    /// [`CheckoutSession::payment_status`].
    Expired,
}

/// Whether a [`CheckoutSession`]'s intent has been paid — D10's second axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutPaymentStatus {
    /// Nothing has settled.
    Unpaid,
    /// The intent reached `succeeded`.
    Paid,
    /// The intent reached a terminal non-success state.
    Failed,
}

/// Which surface a [`CheckoutSession`] is rendered on.
///
/// `#[non_exhaustive]` for the same reason [`PaymentMethodType`] is: a third
/// mode would otherwise be a breaking change for a caller who matched on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CheckoutUiMode {
    /// vpay serves a top-level page and answers with [`CheckoutSession::url`].
    Hosted,
    /// The merchant frames vpay's page with `@vaam-apps/vpay-stripe-js`.
    Embedded,
}

impl CheckoutUiMode {
    /// The exact string this mode is named by on the wire.
    ///
    /// Hand-written beside the `serde` rename for the same reason
    /// [`PaymentMethodType::as_wire_str`] is: the form encoder does not go
    /// through `serde`, so without this the wire spelling would exist in
    /// only one of the two paths.
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            CheckoutUiMode::Hosted => "hosted",
            CheckoutUiMode::Embedded => "embedded",
        }
    }
}

impl std::fmt::Display for CheckoutUiMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire_str())
    }
}

/// A `/v1/checkout/sessions` object (Step 9's D1).
///
/// The session **references** an existing [`PaymentIntent`]; it never
/// creates one. Amount, currency and the allowed rails stay on the intent,
/// where every existing invariant already guards them — which is why there
/// is no `line_items`, no `mode` and no `amount_total` here, however
/// Stripe-shaped the rest of the field names are (D10).
///
/// `Debug` is **hand-written** below rather than derived, for the same
/// reason [`PaymentIntent`]'s is: [`Self::client_secret`] is a live payer
/// credential, and so — on a hosted session — is the fragment of
/// [`Self::url`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckoutSession {
    /// `cs_…`.
    pub id: String,
    /// Always `"checkout.session"`. Carried verbatim rather than asserted,
    /// so an unexpected value is visible to the caller instead of becoming
    /// a decode failure.
    pub object: String,
    /// `false` for a sandbox deployment's objects.
    pub livemode: bool,
    /// The `pi_…` this session drives — an **id**, on every `/v1` route.
    ///
    /// `@vaam-apps/vpay-stripe-js`'s browser-side `CheckoutSession` types the same
    /// field as the whole expanded intent, because the browser session read
    /// expands it (the checkout page confirms and polls the intent through
    /// the browser routes and cannot fetch it separately). A deliberate
    /// per-route difference, ruled on 2026-09-04, not a skew between the
    /// SDKs: nothing on `/v1` expands it, so a merchant integration reads an
    /// id here and calls [`PaymentIntentsResource::retrieve`] if it wants
    /// the object.
    ///
    /// [`PaymentIntentsResource::retrieve`]: crate::PaymentIntentsResource::retrieve
    pub payment_intent: String,
    /// Hosted or embedded — the two have different required URLs.
    pub ui_mode: CheckoutUiMode,
    /// The session's own lifecycle state.
    pub status: CheckoutSessionStatus,
    /// Whether the intent behind it has been paid.
    pub payment_status: CheckoutPaymentStatus,
    /// Where a paying payer is forwarded (hosted mode); `None` when
    /// embedded. May contain the literal `{CHECKOUT_SESSION_ID}`, which vpay
    /// substitutes at forward time and nothing here touches (D5).
    pub success_url: Option<String>,
    /// Where a payer who gave up is forwarded (hosted mode); `None` when
    /// embedded.
    pub cancel_url: Option<String>,
    /// Where vpay's framed page forwards the payer at the end (embedded
    /// mode); `None` when hosted.
    pub return_url: Option<String>,
    /// The page to send a payer to, hosted mode only; `None` when embedded.
    ///
    /// **Carries the session's `client_secret` in its fragment** (D6). It is
    /// the value to redirect a payer's browser to, and it is not a value to
    /// log — see this type's hand-written `impl Debug`.
    pub url: Option<String>,
    /// Unix seconds. 24 h from create (D10).
    pub expires_at: i64,
    /// Unix seconds.
    pub created: i64,
    /// `cs_…_secret_…` — the payer credential the browser presents to read
    /// this session, and what `@vaam-apps/vpay-stripe-js`'s
    /// `initEmbeddedCheckout`'s `fetchClientSecret` must return.
    ///
    /// A **different** credential from [`PaymentIntent::client_secret`]: it
    /// authorises reading this session, never confirming the intent.
    /// Present only on `POST /v1/checkout/sessions` and
    /// `GET /v1/checkout/sessions/{id}` responses — never on a `list()`
    /// item — so `#[serde(default)]` is what makes it decode to `None`
    /// there rather than failing.
    ///
    /// Never log this value: see this type's hand-written `impl Debug`.
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Replaces everything after the first `#` with a length marker.
///
/// A hosted session's [`CheckoutSession::url`] is
/// `{base}/c/{cs_id}#{client_secret}` (D6), so redacting `client_secret`
/// alone and rendering `url` verbatim would print the very value the
/// redaction exists to hide. The path stays visible, because knowing *which*
/// page a session points at is the whole diagnostic value of the field.
fn redact_url_fragment(url: &str) -> String {
    match url.split_once('#') {
        Some((head, fragment)) => format!("{head}#[{} chars redacted]", fragment.len()),
        None => url.to_owned(),
    }
}

/// Redacts the two fields of a [`CheckoutSession`] that are live
/// credentials — `client_secret`, and the fragment of `url` — leaving every
/// other field exactly as a derived `Debug` would render it.
impl std::fmt::Debug for CheckoutSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckoutSession")
            .field("id", &self.id)
            .field("object", &self.object)
            .field("livemode", &self.livemode)
            .field("payment_intent", &self.payment_intent)
            .field("ui_mode", &self.ui_mode)
            .field("status", &self.status)
            .field("payment_status", &self.payment_status)
            .field("success_url", &self.success_url)
            .field("cancel_url", &self.cancel_url)
            .field("return_url", &self.return_url)
            .field("url", &self.url.as_deref().map(redact_url_fragment))
            .field("expires_at", &self.expires_at)
            .field("created", &self.created)
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

/// The event types `docs/flows/webhooks.md` commits to — the vocabulary a
/// merchant `match`es [`Event::kind`] against.
///
/// The mirror of `sdks/nodejs`'s `KnownEventType` union, and it exists for
/// the same reason: every one of them is a **real Stripe** type, because a
/// custom one is silently dropped by any handler switching exhaustively over
/// Stripe's own typed event union. Spelling them once here is what stops a
/// merchant matching on a string literal with a typo in it, which compiles
/// and never fires.
///
/// [`Event::kind`] is deliberately **not** this type. It stays a `String`, so
/// an event carrying a type this SDK version predates is still deliverable
/// rather than a decode failure in a merchant's handler
/// (`docs/flows/merchant-auth.md`, "Objects") — the same split
/// [`PaymentIntent::payment_method_types`] makes. Use [`Self::from_wire`] to
/// narrow, and handle `None` as "a type newer than this SDK".
///
/// `#[non_exhaustive]`, so vpay documenting a ninth type is not a breaking
/// change for a caller who matched on it.
///
/// ```
/// use vpay_sdk::KnownEventType;
///
/// assert_eq!(
///     KnownEventType::from_wire("checkout.session.expired"),
///     Some(KnownEventType::CheckoutSessionExpired)
/// );
/// assert_eq!(
///     KnownEventType::CheckoutSessionExpired.as_wire_str(),
///     "checkout.session.expired"
/// );
/// // A type this SDK version predates is not an error — it is `None`, and
/// // the event is still delivered and still readable.
/// assert_eq!(KnownEventType::from_wire("checkout.session.completed"), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum KnownEventType {
    /// A PaymentIntent was created. Nothing emits this today.
    PaymentIntentCreated,
    /// A PaymentIntent was submitted to a rail. Nothing emits this today.
    PaymentIntentProcessing,
    /// A payment settled. `data.object` is a [`PaymentIntent`].
    PaymentIntentSucceeded,
    /// A rail declined a payment. `data.object` is a [`PaymentIntent`],
    /// back in [`IntentStatus::RequiresPaymentMethod`] with
    /// [`PaymentIntent::last_payment_error`] populated.
    PaymentIntentPaymentFailed,
    /// A PaymentIntent was withdrawn. Nothing emits this today.
    PaymentIntentCanceled,
    /// A charge was refunded. Nothing emits this today.
    ChargeRefunded,
    /// A refund changed state. Nothing emits this today.
    ChargeRefundUpdated,
    /// A Checkout Session passed its 24-hour horizon without being paid and
    /// vpay's hourly sweep expired it (Step 9's D10).
    ///
    /// `data.object` is a [`CheckoutSession`] — the only one of these whose
    /// payload is neither a [`PaymentIntent`] nor a [`Refund`]. Decode it
    /// with [`Event::checkout_session`].
    ///
    /// **Not** emitted when a merchant expires a session itself through
    /// `POST /v1/checkout/sessions/{id}/expire`, and not emitted when a
    /// settlement moves a session — that transition already sends a
    /// `payment_intent.*` event for the same thing happening.
    CheckoutSessionExpired,
}

impl KnownEventType {
    /// The exact string this type is named by on the wire.
    ///
    /// Hand-written rather than derived from a `serde` rename, exactly as
    /// [`PaymentMethodType::as_wire_str`] is: [`Event::kind`] is a `String`
    /// and never goes through `serde` for this type, so without it the wire
    /// spelling would exist in only one of the two paths.
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            KnownEventType::PaymentIntentCreated => "payment_intent.created",
            KnownEventType::PaymentIntentProcessing => "payment_intent.processing",
            KnownEventType::PaymentIntentSucceeded => "payment_intent.succeeded",
            KnownEventType::PaymentIntentPaymentFailed => "payment_intent.payment_failed",
            KnownEventType::PaymentIntentCanceled => "payment_intent.canceled",
            KnownEventType::ChargeRefunded => "charge.refunded",
            KnownEventType::ChargeRefundUpdated => "charge.refund.updated",
            KnownEventType::CheckoutSessionExpired => "checkout.session.expired",
        }
    }

    /// The type a wire string names, or `None` for one this SDK version does
    /// not know.
    ///
    /// `None` is a normal answer and never an error: vpay may document a
    /// type after this SDK was published, and an event carrying it is still
    /// delivered, still verifiable and still readable through
    /// [`Event::kind`]. A parse that failed here would turn "newer server"
    /// into "broken handler".
    #[must_use]
    pub fn from_wire(wire: &str) -> Option<Self> {
        // A `match` rather than an iteration over a slice, so adding a
        // variant above without adding it here is a non-exhaustive-match
        // compile error.
        match wire {
            "payment_intent.created" => Some(KnownEventType::PaymentIntentCreated),
            "payment_intent.processing" => Some(KnownEventType::PaymentIntentProcessing),
            "payment_intent.succeeded" => Some(KnownEventType::PaymentIntentSucceeded),
            "payment_intent.payment_failed" => Some(KnownEventType::PaymentIntentPaymentFailed),
            "payment_intent.canceled" => Some(KnownEventType::PaymentIntentCanceled),
            "charge.refunded" => Some(KnownEventType::ChargeRefunded),
            "charge.refund.updated" => Some(KnownEventType::ChargeRefundUpdated),
            "checkout.session.expired" => Some(KnownEventType::CheckoutSessionExpired),
            _ => None,
        }
    }
}

impl std::fmt::Display for KnownEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire_str())
    }
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

    /// Decodes [`EventData::object`] as a [`CheckoutSession`], for a
    /// `checkout.session.*` event.
    ///
    /// The session on an event carries **no** `client_secret` and its `url`
    /// is always `None`: both are live payer credentials, and an event body
    /// is stored, delivered at-least-once and replayable. So a `None` `url`
    /// here does not mean the session was embedded — read
    /// [`CheckoutSession::ui_mode`] for that. Everything else is the object
    /// as it stood at the transition, which for
    /// [`KnownEventType::CheckoutSessionExpired`] means `status` is already
    /// [`CheckoutSessionStatus::Expired`] while `payment_status` is whatever
    /// the money did.
    ///
    /// # Errors
    /// See [`Event::payment_intent`].
    pub fn checkout_session(&self) -> Result<CheckoutSession, crate::Error> {
        serde_json::from_value(self.data.object.clone()).map_err(|e| {
            crate::Error::UnexpectedResponse {
                status: 0,
                body_prefix: format!("event.data.object did not decode as a checkout_session: {e}"),
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
