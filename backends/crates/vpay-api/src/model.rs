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
use time::OffsetDateTime;
use vpay_core::{IntentStatus, RefundStatus};

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
object_tag!(
    /// The `"account_holder"` discriminator.
    ///
    /// A resource of its own rather than an overloaded Stripe `customer`,
    /// per issue #47's own reasoning: a `customer` is a stored,
    /// merchant-owned object with a lifecycle, and this is a stateless rail
    /// query that creates nothing.
    AccountHolderTag,
    "account_holder"
);
object_tag!(
    /// The `"refund"` discriminator.
    RefundTag,
    "refund"
);
object_tag!(
    /// The `"customer"` discriminator.
    ///
    /// Stripe's own spelling for a stored, merchant-owned payer record. The
    /// distinction [`AccountHolderTag`] draws is unchanged and is now
    /// load-bearing in both directions: an `account_holder` is a *stateless
    /// rail query* that stores nothing and has no id, a `customer` is this —
    /// a `cus_…` with a lifecycle, metadata and a retention clock. A
    /// deployment can have both for one payer and they are not the same
    /// object.
    CustomerTag,
    "customer"
);
object_tag!(
    /// The `"invoice"` discriminator (S4b) — Stripe's own spelling.
    InvoiceTag,
    "invoice"
);
object_tag!(
    /// The `"line_item"` discriminator on a line of an invoice.
    ///
    /// **Stripe's own spelling for the object inside `invoice.lines`, and
    /// deliberately not `"invoiceitem"`.** Stripe has two objects here: an
    /// `invoiceitem` is a *pending* charge not yet attached to a document,
    /// and a `line_item` is what appears on `invoice.lines` once it is. vpay
    /// has only the second — `POST /v1/invoice_items` writes straight onto a
    /// draft invoice, because a pending-charge inbox is a subscription
    /// feature and subscriptions are not built (`docs/flows/invoices.md`).
    ///
    /// The consequence a merchant can see is stated rather than hidden: the
    /// same row is addressed as `ii_…` on `/v1/invoice_items` and rendered as
    /// a `line_item` inside `invoice.lines`. That matches what a
    /// Stripe-shaped handler switching on `object` expects to find in a
    /// `lines` list.
    LineItemTag,
    "line_item"
);
object_tag!(
    /// The `"checkout.session"` discriminator.
    ///
    /// Stripe's own spelling, dot and all, so a merchant switching an
    /// integration over does not have to special-case it. D10 is explicit
    /// that field names mirror Stripe *only* where the semantics match —
    /// this one does exactly.
    CheckoutSessionTag,
    "checkout.session"
);

/// Where to send a payer on a redirect rail.
///
/// Stripe's own `next_action.redirect_to_url` shape, so a merchant's existing
/// redirect handling works unchanged (`docs/flows/payment-lifecycle.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
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
/// # Examples
///
/// The externally-tagged shape a merchant's existing Stripe redirect handling
/// already reads:
///
/// ```
/// use serde_json::json;
/// use vpay_api::model::{NextAction, RedirectToUrl};
///
/// let action = NextAction::RedirectToUrl {
///     redirect_to_url: RedirectToUrl {
///         url: "https://rail.example/pay/abc".to_owned(),
///         return_url: Some("https://merchant.example/done".to_owned()),
///     },
/// };
/// assert_eq!(
///     serde_json::to_value(&action).expect("a wire DTO always serialises"),
///     json!({
///         "type": "redirect_to_url",
///         "redirect_to_url": {
///             "url": "https://rail.example/pay/abc",
///             "return_url": "https://merchant.example/done",
///         },
///     }),
/// );
/// ```
///
/// A rail that was given no `return_url` renders `null`, not a missing key —
/// an SDK reading `redirect_to_url.return_url` gets an answer either way:
///
/// ```
/// use serde_json::json;
/// use vpay_api::model::{NextAction, RedirectToUrl};
///
/// let action = NextAction::RedirectToUrl {
///     redirect_to_url: RedirectToUrl {
///         url: "https://rail.example/pay/abc".to_owned(),
///         return_url: None,
///     },
/// };
/// let rendered = serde_json::to_value(&action).expect("a wire DTO always serialises");
/// assert_eq!(rendered["redirect_to_url"]["return_url"], json!(null));
/// ```
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
/// # Examples
///
/// ```
/// use serde_json::json;
/// use vpay_api::model::LastPaymentErrorObject;
///
/// let refusal = LastPaymentErrorObject {
///     code: "insufficient_funds".to_owned(),
///     message: "The payer's account does not hold enough to cover this charge.".to_owned(),
/// };
/// assert_eq!(
///     serde_json::to_value(&refusal).expect("a wire DTO always serialises"),
///     json!({
///         "code": "insufficient_funds",
///         "message": "The payer's account does not hold enough to cover this charge.",
///     }),
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
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

/// An `account_holder`: whose mobile-money account a number is, or the fact
/// that the rail has no record of it (issue #47).
///
/// Four keys, always all four — this module's own rule, and here it is
/// load-bearing twice over: both SDKs model `name` as a *required, nullable*
/// field, and `name: null` is a **meaningful answer** rather than an absence
/// ("the rail does not know this number"), which an omitted key could not
/// express.
///
/// # There is no row behind this object
///
/// Every other type in this module is rendered from a `vpay_db` row. This
/// one is rendered from a rail's answer and is never stored — see
/// `crate::v1::account_holders`' module header for what that buys and what
/// it costs. It is why there is no `TryFrom<..Row>` impl beside it and no
/// `id`: there is nothing to address later.
///
/// # `livemode` is deliberately absent
///
/// [`PaymentIntentObject::livemode`] is read off the row, so an object
/// created under one setting cannot start describing itself differently.
/// With no row, the only available value would be the deployment's current
/// configuration read at render time — the same field name carrying a
/// weaker guarantee. Issue #47's proposal names four keys and this renders
/// those four; `docs/flows/account-holder-lookup.md` records it as a
/// decision rather than settling it here.
///
/// ```
/// use serde_json::json;
/// use vpay_api::model::{AccountHolderObject, AccountHolderTag};
///
/// let unknown = AccountHolderObject {
///     object: AccountHolderTag,
///     payment_method_type: "mtn_momo".to_owned(),
///     name: None,
///     verified: false,
/// };
/// assert_eq!(
///     serde_json::to_value(&unknown).expect("a wire DTO always serialises"),
///     json!({
///         "object": "account_holder",
///         "payment_method_type": "mtn_momo",
///         "name": null,
///         "verified": false,
///     }),
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AccountHolderObject {
    /// Always `"account_holder"`.
    pub object: AccountHolderTag,
    /// The rail the question was put to, echoed back — the same
    /// `providers.code` vocabulary `payment_method_types` uses.
    pub payment_method_type: String,
    /// The registered holder's name, or `null` when the rail has no record.
    ///
    /// **Never a fabricated value, and never a rail that could not be
    /// asked.** `null` means one specific thing: the rail answered and does
    /// not know this number. An unreachable rail is a classified error
    /// (ADR-0011), because a `200` with nulls would tell a caller a real
    /// account is unregistered — which is the anti-fraud control issue #47
    /// is about, inverted.
    pub name: Option<String>,
    /// `true` exactly when [`Self::name`] is present.
    ///
    /// Redundant with `name != null`, deliberately: it is what an SDK
    /// branches on, and it survives a client that models `name` loosely. It
    /// is **not** a claim that anything was cryptographically verified — it
    /// says the rail named a holder.
    pub verified: bool,
}

/// A `payment_intent`, exactly as `docs/api/README.md`'s object table and
/// `sdks/rust/tests/support/mod.rs`'s fixture describe it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
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
    /// The `cus_…` this intent is for, or `null` (S4a).
    ///
    /// The id, never the expanded object. `expand` is not implemented
    /// (`docs/api/README.md`), so there is no way for a merchant to ask for
    /// the object — and rendering it unasked would put a payer's name, email
    /// and phone number into every `payment_intent.*` webhook body, signed,
    /// delivered at-least-once and stored in `events` forever. The customer
    /// is one `GET /v1/customers/{id}` away for anybody who holds the
    /// merchant token; the webhook receiver deliberately is not given it.
    ///
    /// This key was **absent** until 2026-09-06 and the request field was
    /// accepted and dropped (`docs/api/README.md`'s "accepted and ignored"
    /// list). Both SDKs model it as optional so a vpay predating it still
    /// decodes, which is exactly why the server must not omit it — see
    /// [`RefundObject::fee`]'s note on the same trap.
    pub customer: Option<String>,
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
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
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

/// A `payment_intent` reference that may be the id **or** the whole object —
/// Stripe's `expand` shape, at the two places vpay uses it.
///
/// # Which surface renders which, and why
///
/// * `/v1/checkout/sessions` (create, retrieve, list) renders
///   [`Self::Id`]. A merchant already holds the intent — they created it —
///   so expanding would put a second, possibly stale copy of every amount on
///   the wire, and on the list it would multiply that by the page size.
/// * `GET /v1/browser/checkout/sessions/{id}` renders
///   [`Self::ExpandedWithSecret`]. vpay's own checkout page has *only* the
///   session id and the session secret: it needs the amount, the currency,
///   the status, `payment_method_types` (which rails to offer),
///   `next_action` and `last_payment_error` to render anything at all, and
///   the intent's `client_secret` to drive
///   `/v1/browser/payment_intents/{id}[/confirm]`. Two round trips for that
///   would mean the page cannot paint until both land.
/// * `GET /v1/browser/checkout/sessions/{id}/return` renders
///   [`Self::Expanded`] — everything above **except** the credential, which
///   the return page has no use for and must not be handed (D6; see
///   `crate::browser::checkout_sessions`).
///
/// # Why an enum and not `Option<PaymentIntentObject>` beside a `String`
///
/// Because the *type* is then what stops the return page from being handed a
/// credential. Two nullable fields would make it a runtime question — "did
/// this handler remember to clear it?" — answerable only by reading every
/// call site; three variants make each route name what it renders, and a
/// route that wanted to render a secret would have to say the word.
///
/// `#[serde(untagged)]`: the wire shape is a string or an object, with no
/// discriminator, exactly as Stripe's expansion is. Both merchant SDKs decode
/// it as a union.
///
/// `Box`, because [`PaymentIntentObject`] is much larger than a `String` and
/// an un-boxed variant would make every [`CheckoutSessionObject`] — including
/// each of the up-to-100 on a list page — carry that footprint.
///
/// No `Deserialize`: nothing in this crate decodes it. `Debug` is derived and
/// safe — [`Self::ExpandedWithSecret`] formats through
/// [`PaymentIntentWithSecret`]'s own hand-written, redacting impl.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ExpandableIntent {
    /// `"pi_…"` — the merchant surface's answer.
    Id(String),
    /// The twelve documented keys, and no credential.
    Expanded(Box<PaymentIntentObject>),
    /// The twelve keys plus `client_secret`.
    ExpandedWithSecret(Box<PaymentIntentWithSecret>),
}

impl ExpandableIntent {
    /// The id, whichever shape this is — so a caller that only wants to
    /// correlate never has to match.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Id(id) => id,
            Self::Expanded(intent) => &intent.id,
            Self::ExpandedWithSecret(intent) => &intent.intent.id,
        }
    }
}

/// The `status` a session is in once its horizon has passed (D10).
///
/// A constant rather than a literal inside
/// [`CheckoutSessionObject::expired_snapshot`], because the same label is
/// written by `vpay_db::CheckoutSessions::expire_due`'s `UPDATE` and the two
/// have to agree: the event's `data.object` claiming one status while the row
/// holds another is precisely the disagreement rendering-before-the-write
/// risks, and it is the reason `GET /v1/events` and `GET /v1/checkout/sessions`
/// can be compared at all.
const EXPIRED: &str = "expired";

/// A `checkout.session`, exactly as the wire contract in
/// `docs/plans/2026-09-04-step9-hosted-checkout.md` describes it.
///
/// # Why the three lifecycle fields are `String` and not enums
///
/// The same argument [`EventObject::kind`] and
/// [`LastPaymentErrorObject::code`] make: the vocabularies are closed by the
/// database (`ui_mode_is_known`, `status_is_known`,
/// `payment_status_is_known`, migration `0028`) where they are *written*, and
/// a value that failed to parse on the read path would turn a merchant's
/// `GET` into a `500` instead of showing them their session. `status` on
/// [`PaymentIntentObject`] is typed for the opposite reason — `IntentStatus`
/// is `vpay-core`'s own state machine and a handler must not be able to
/// render a status outside it — and no such machine exists for a session
/// (D10: the lifecycle is minimal and has no transitions a type could guard).
///
/// # `url`, and why it is `Option`
///
/// `null` for an embedded session, which has no page to redirect a payer to:
/// the merchant's own site mounts the iframe. Derived at render time from
/// `checkout.public_base_url` and never stored, so a deployment that moves
/// its checkout app does not have to rewrite every row — and so a session
/// created before the app was configured cannot render a link to a host that
/// no longer exists.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckoutSessionObject {
    /// `cs_…` — `vpay_core::ids::checkout_session_id`.
    pub id: String,
    /// Always `"checkout.session"`.
    pub object: CheckoutSessionTag,
    /// `false` for a sandbox deployment's objects. Taken from the row, not
    /// from configuration read at render time, for
    /// [`PaymentIntentObject::livemode`]'s reason.
    pub livemode: bool,
    /// The intent this session drives — the `pi_…` id on the merchant
    /// surface, and the whole object on the two browser reads. See
    /// [`ExpandableIntent`] for which surface renders which, and why.
    pub payment_intent: ExpandableIntent,
    /// `hosted` or `embedded`.
    pub ui_mode: String,
    /// `open`, `complete` or `expired` (D10).
    pub status: String,
    /// `unpaid`, `paid` or `failed` (D10).
    pub payment_status: String,
    /// Hosted mode's forward destination; `null` for embedded.
    ///
    /// May contain the literal `{CHECKOUT_SESSION_ID}` (D5), which vpay
    /// substitutes when it forwards the payer — and only then. It is echoed
    /// back here **unsubstituted**, exactly as the merchant wrote it, because
    /// this field is a record of their configuration rather than of one
    /// payer's journey.
    pub success_url: Option<String>,
    /// Hosted mode's abandon destination; `null` for embedded. Same
    /// `{CHECKOUT_SESSION_ID}` rule.
    pub cancel_url: Option<String>,
    /// Embedded mode's forward destination; `null` for hosted. Same
    /// `{CHECKOUT_SESSION_ID}` rule.
    pub return_url: Option<String>,
    /// Where to send the payer, for a hosted session on a deployment that
    /// serves a checkout page; `null` for an embedded one.
    ///
    /// **Carries the session's `client_secret` in its fragment** (D6), so
    /// this field is as sensitive as the credential itself and is rendered
    /// only where that credential is. It is `Option` on the object rather
    /// than a field of [`CheckoutSessionWithSecret`] because the list has to
    /// render *something* for it, and `null` beside a `hosted` session is the
    /// honest answer there: the merchant is not being told the link, they are
    /// being told they already have it.
    pub url: Option<String>,
    /// The `cus_…` this session is for, or `null` (S4a). The id, never the
    /// expanded object — [`PaymentIntentObject::customer`]'s reason,
    /// unchanged, and with the same force: this object is the `data.object`
    /// of every `checkout.session.expired`.
    pub customer: Option<String>,
    /// Unix **seconds** when this session stops being `open` on its own —
    /// like every other timestamp on this API, and not RFC 3339.
    pub expires_at: i64,
    /// Unix **seconds**.
    pub created: i64,
}

impl CheckoutSessionObject {
    /// Renders a stored session as the object a merchant reads.
    ///
    /// # Why this is not a `TryFrom`, and not a `From` either
    ///
    /// Not `TryFrom`, because — unlike [`PaymentIntentObject`] — there is
    /// nothing here that can fail: every column this reads is already a
    /// `String`, a `bool` or a timestamp, and the three closed vocabularies
    /// stay text on the wire (see the type's own doc). A `Result` nothing can
    /// populate is a `?` at every call site that documents a risk that does
    /// not exist.
    ///
    /// Not `From`, because `url` is not on the row and cannot be: it is built
    /// from `checkout.public_base_url`, which is deployment configuration
    /// this module deliberately cannot see. Taking it as a parameter is what
    /// keeps `model` a rendering layer — the same split
    /// [`PaymentIntentObject::with_next_action`] draws for `next_action`,
    /// which lives on the *charge*.
    ///
    /// `url` is the caller's answer and is expected to be `None` for an
    /// embedded session and for a deployment with no checkout app. Nothing
    /// here re-derives or second-guesses it.
    #[must_use]
    pub fn from_row(row: &vpay_db::CheckoutSessionRow, url: Option<String>) -> Self {
        Self {
            id: row.id.clone(),
            object: CheckoutSessionTag,
            livemode: row.livemode,
            // The id, always. A caller that means to expand says so with
            // `with_expanded_intent`, so the merchant surface cannot grow an
            // expansion by accident and the browser surface cannot forget to
            // ask for one.
            payment_intent: ExpandableIntent::Id(row.payment_intent_id.clone()),
            ui_mode: row.ui_mode.clone(),
            status: row.status.clone(),
            payment_status: row.payment_status.clone(),
            success_url: row.success_url.clone(),
            cancel_url: row.cancel_url.clone(),
            return_url: row.return_url.clone(),
            url,
            customer: row.customer_id.clone(),
            expires_at: row.expires_at.unix_timestamp(),
            created: row.created_at.unix_timestamp(),
        }
    }

    /// The object as it will stand once the expiry sweep's transaction
    /// commits — the `data.object` of a `checkout.session.expired` event.
    ///
    /// # Why the projection is applied before the write
    ///
    /// `vpay_db::CheckoutSessions::expire_due` takes `event_data` as an
    /// input, because the event is written inside the same transaction as the
    /// row it describes and therefore cannot be rendered from the result. So
    /// the caller renders what the row is *about to* say, exactly as
    /// `vpay_worker::handlers::intent_snapshot` does for a settlement. The
    /// one field patched is the one field that transition changes:
    /// `payment_status` is deliberately untouched by an expiry (an expired
    /// session that was already `paid` keeps saying so), and every other
    /// column is already what it will be.
    ///
    /// # Why `url` is `None`, and is not the caller's choice here
    ///
    /// [`Self::url`] carries the session's `client_secret` in its fragment
    /// (D6), and this object is **stored** in `events.data` and delivered
    /// at-least-once to every endpoint the merchant configured. A credential
    /// in a webhook body is a credential in the merchant's logs, in their
    /// queue, and in every replay of it — for a session that has just stopped
    /// being payable, which is the one case where the link is worth nothing
    /// to its holder and everything to anyone else. [`Self::from_row`] leaves
    /// that to the caller because three routes want three different answers;
    /// here there is only one right answer, so this is a constructor rather
    /// than a parameter.
    ///
    /// `return_token` is not on this object at all — it is a column, and
    /// [`Self::from_row`] never reads it.
    ///
    /// ```
    /// # use vpay_api::model::CheckoutSessionObject;
    /// # use time::OffsetDateTime;
    /// # let row = vpay_db::CheckoutSessionRow {
    /// #     id: "cs_0123456789abcdefghjkmnpq".to_owned(),
    /// #     seq: 1,
    /// #     merchant_id: "acme-cameroon-tenant".to_owned(),
    /// #     payment_intent_id: "pi_0123456789abcdefghjkmnpq".to_owned(),
    /// #     livemode: false,
    /// #     ui_mode: "hosted".to_owned(),
    /// #     status: "open".to_owned(),
    /// #     payment_status: "unpaid".to_owned(),
    /// #     success_url: None,
    /// #     cancel_url: None,
    /// #     return_url: None,
    /// #     customer_id: None,
    /// #     publishable_key: "pk_test_acmecameroonsandbox01".to_owned(),
    /// #     client_secret_suffix: "0".repeat(32),
    /// #     return_token: "wxyz0123456789abcdefghjkmnpqrstv".to_owned(),
    /// #     expires_at: OffsetDateTime::UNIX_EPOCH,
    /// #     created_at: OffsetDateTime::UNIX_EPOCH,
    /// #     updated_at: OffsetDateTime::UNIX_EPOCH,
    /// # };
    /// let expired = CheckoutSessionObject::expired_snapshot(&row);
    /// assert_eq!(expired.status, "expired");
    /// // …while the row it was built from still says `open`: the event
    /// // describes what the transaction is about to commit.
    /// assert_eq!(row.status, "open");
    /// assert_eq!(expired.url, None);
    /// ```
    #[must_use]
    pub fn expired_snapshot(row: &vpay_db::CheckoutSessionRow) -> Self {
        Self {
            status: EXPIRED.to_owned(),
            ..Self::from_row(row, None)
        }
    }

    /// Replaces the `pi_…` id with the whole intent object.
    ///
    /// A builder rather than a parameter of [`Self::from_row`], mirroring
    /// [`PaymentIntentObject::with_next_action`] and for the same reason: the
    /// default is the answer three of the five routes want, and a parameter
    /// would make every one of them pass `ExpandableIntent::Id(...)` — which
    /// is one transposition away from a list page that expands.
    ///
    /// The caller chooses [`ExpandableIntent::Expanded`] or
    /// [`ExpandableIntent::ExpandedWithSecret`], and *that* choice is the
    /// whole of "does this response carry the intent's credential". See
    /// `crate::browser::checkout_sessions`, which makes it once per route.
    #[must_use]
    pub fn with_expanded_intent(mut self, intent: ExpandableIntent) -> Self {
        self.payment_intent = intent;
        self
    }
}

/// The merchant, as a **payer** is shown them: one name and nothing else.
///
/// An object rather than a bare `merchant_name` string on the session, so the
/// two browser reads can grow a second payer-facing fact about the merchant —
/// a logo, a support address — without every consumer having to distinguish
/// `merchant_name` from `merchant.name`. That is the shape
/// `frontends/apps/checkout` is written against
/// (`src/lib/types.ts`'s `CheckoutMerchant`), and the field name is `name`
/// because that is the member its `isSessionEnvelope` guard requires; a
/// server rendering `display_name` here would make every session read
/// `error.unexpected` on the page.
///
/// Deliberately *not* the tenant id, the `client_id`, or anything else vpay
/// knows about the merchant. A payer is being asked to hand over money on the
/// strength of recognising who they are paying, and every other identifier
/// this system holds is an internal one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckoutMerchantObject {
    /// `merchant_clients[].display_name`. There is no fallback: a merchant
    /// with none configured renders no `merchant` member at all — see
    /// `ResourceConfig::merchant_display_name`.
    pub name: String,
}

/// A `checkout.session` as **vpay's own checkout page** reads it: the session,
/// its expanded intent, and the merchant's name.
///
/// # Why a wrapper, and not a field on [`CheckoutSessionObject`]
///
/// [`CheckoutSessionWithSecret`]'s reason, pointing the other way. That type
/// exists to keep a credential off the responses that must not carry it; this
/// one exists to keep a *deployment-configured* value off the responses that
/// have no business rendering it. `merchant` is meaningless on the merchant
/// surface — a merchant reading `GET /v1/checkout/sessions` already knows who
/// they are — and a field on the object would put it on all four `/v1`
/// responses and on every row of the list, where it would be one more thing an
/// SDK has to model and one more thing that has to stay in step with
/// configuration.
///
/// `#[serde(flatten)]`, so the wire shape is the session's own keys plus
/// `merchant` at the top level, which is what the page's envelope check reads.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckoutSessionForPayer {
    /// The documented keys, rendered by the same code every other surface
    /// uses.
    #[serde(flatten)]
    pub session: CheckoutSessionObject,
    /// Who the payer is paying — absent when the merchant configured no
    /// `display_name`, never an internal identifier in its place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merchant: Option<CheckoutMerchantObject>,
}

impl CheckoutSessionForPayer {
    /// Pairs a rendered session with the name a payer is shown.
    ///
    /// Takes the name rather than a `ResourceConfig`, for
    /// [`CheckoutSessionWithSecret::new`]'s reason: this module renders, and a
    /// model type that could read deployment configuration is a model type
    /// that can grow a second answer to a question the boundary already
    /// decided.
    #[must_use]
    pub fn new(session: CheckoutSessionObject, name: Option<String>) -> Self {
        Self {
            session,
            merchant: name.map(|name| CheckoutMerchantObject { name }),
        }
    }
}

/// A `checkout.session` **plus the payer credential that addresses it** —
/// what `POST /v1/checkout/sessions` and `GET /v1/checkout/sessions/{id}`
/// answer with.
///
/// # Why a wrapper, and not a field on [`CheckoutSessionObject`]
///
/// [`PaymentIntentWithSecret`]'s reason, transplanted: the field must reach
/// two responses and no others, and a field on the object would reach every
/// response that type can render — which includes `GET
/// /v1/checkout/sessions`, the list, whose every row would then carry a live
/// credential for a page that can read a payment intent's own secret. A list
/// response is the one most likely to be logged wholesale.
///
/// The **browser** routes render the bare object, never this: a payer
/// presenting the session secret already holds it, and the session read hands
/// back the *intent's* secret instead, which is what the page actually needs.
///
/// `return_token` is on neither type. It is not a merchant-facing value at
/// all: vpay builds the one URL that carries it (`{base}/c/{id}/return?t=…`)
/// and hands that to the *rail*, so rendering it on a `/v1` response would
/// publish a credential nobody has a use for.
///
/// No `Deserialize`: nothing decodes this. `Debug` is hand-written below,
/// because [`Self::client_secret`] is a live credential.
#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckoutSessionWithSecret {
    /// The documented keys, unchanged and rendered by the same code every
    /// other surface uses.
    #[serde(flatten)]
    pub session: CheckoutSessionObject,
    /// `cs_…_secret_…` — `vpay_core::ids::client_secret` of the row's `id`
    /// and its `client_secret_suffix`.
    ///
    /// Not `Option<String>` although Stripe's is nullable: every route that
    /// renders this type has the row in hand, and the row's suffix is
    /// `NOT NULL` (migration `0028`).
    pub client_secret: String,
}

/// Redacts [`CheckoutSessionWithSecret::client_secret`], leaving the rendered
/// session visible.
///
/// [`PaymentIntentWithSecret`]'s impl, and its reasoning, applied to the
/// other credential: this is the whole bearer value
/// `/v1/browser/checkout/sessions/{id}` accepts, and what that route hands
/// back is the *intent's* `client_secret` — so a session secret in a log line
/// is one hop from confirming a payment.
///
/// [`CheckoutSessionObject::url`] is redacted **too**, and only here, which
/// is the one place this impl departs from its sibling: the URL carries the
/// same credential in its fragment, so printing the session in full while
/// hiding the `client_secret` field would redact nothing at all. Its length
/// stays, for the same debugging reason the secret's does.
impl std::fmt::Debug for CheckoutSessionWithSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Rebuilt rather than mutated: `session` is borrowed, and a `Debug`
        // that had to clone to be safe would be a `Debug` someone eventually
        // "optimises" back into a leak.
        let mut redacted = self.session.clone();
        redacted.url = redacted
            .url
            .map(|url| format!("[{} chars redacted: carries the session secret]", url.len()));
        f.debug_struct("CheckoutSessionWithSecret")
            .field("session", &redacted)
            .field(
                "client_secret",
                &format_args!("[{} chars redacted]", self.client_secret.len()),
            )
            .finish()
    }
}

impl CheckoutSessionWithSecret {
    /// Pairs a rendered session with the secret derived from the row it came
    /// from.
    ///
    /// Takes the already-rendered object rather than the row, mirroring
    /// [`PaymentIntentWithSecret::new`]: the `url` has to be built from
    /// deployment configuration this module does not hold, so re-deriving
    /// here would either drop it or make this type depend on
    /// `ResourceConfig`.
    #[must_use]
    pub fn new(session: CheckoutSessionObject, client_secret: String) -> Self {
        Self {
            session,
            client_secret,
        }
    }
}

/// A `refund`, as `GET /v1/refunds/{id}` serves it — exactly
/// `docs/flows/merchant-auth.md`'s object paragraph and both merchant SDKs'
/// `Refund`.
///
/// # One renderer, two surfaces, on purpose — the same argument [`EventObject`] makes
///
/// `docs/flows/webhooks.md` commits to `charge.refunded` and
/// `charge.refund.updated`, whose `data.object` is a refund. **Neither is
/// emitted by anything today** (`docs/status.md`), and this type is
/// deliberately what the writer of those events will have to use, exactly as
/// [`PaymentIntentObject`] is what the settlement transaction already uses
/// for `payment_intent.*`: delivery is at-least-once and unordered
/// (`docs/flows/webhooks.md`), a merchant who missed a delivery is told to
/// re-read the object, and if the API and the event body rendered a refund
/// differently the fallback would answer a different question from the one
/// the webhook asked. A second hand-built map of these ten keys is how
/// `created` ends up in milliseconds on one of them.
///
/// The wire shape is `docs/flows/merchant-auth.md`'s, and it is the shape
/// both merchant SDKs already decode (`vpay_sdk::Refund`,
/// `@vaam-apps/vpay-sdk`'s `Refund`) — they have carried it since before any
/// route served one.
///
/// # `fee` is the tenth field, and its absence is information
///
/// See [`fee`](RefundObject::fee). It was added by issue #46, reported by an
/// integrator whose settlement statement needs to say what a refund *cost*
/// and who was hardcoding `0` because vpay returned nothing. Nothing writes
/// it yet: no rail reports a refund fee to us, so every refund this
/// repository can currently produce carries `null` (`docs/status.md`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct RefundObject {
    /// `re_…` — `vpay_core::ids::refund_id`.
    pub id: String,
    /// Always `"refund"`.
    pub object: RefundTag,
    /// Integer minor units of [`currency`](Self::currency)
    /// (`docs/flows/money.md`). **The payer's money**, never net of
    /// [`fee`](Self::fee): a buyer sees the amount they paid.
    pub amount: i64,
    /// **Lowercase** on the wire (`xaf`), although the column holds the
    /// uppercase ISO-4217 code — lowercased once, here at the boundary, as
    /// [`PaymentIntentObject::currency`] is.
    pub currency: String,
    /// The `pi_…` this refunds. A refund is requested against an intent, not
    /// against a charge.
    pub payment_intent: String,
    /// `pending`, `succeeded`, `failed` or `canceled` — the refund's own
    /// state, independent of its intent's. Unlike an intent, a refund does
    /// have a terminal `failed`.
    ///
    /// # Typed, and what a parse failure means
    ///
    /// A [`RefundStatus`] and not a `String`, unlike [`EventObject::kind`].
    /// The argument for a `String` is that a label this crate cannot parse
    /// would turn a merchant's `GET /v1/refunds/{id}` into a `500` instead of
    /// showing them the refund; the reason it does not win here is that
    /// `refunds.status` holds exactly the four labels above and the database
    /// refuses a fifth — as the `refund_status` Postgres `ENUM` (migration
    /// `0017`) until 2026-09-07, and as the `refunds_status_enum_check`
    /// membership CHECK carrying the identical four labels since migration
    /// `0037` converted the column to `TEXT`. A fifth value cannot be
    /// written without a migration, so a value that fails to parse is not a
    /// vocabulary this code has not caught up with — it is a **corrupted
    /// row**, and rendering a refund whose state vpay cannot name would tell
    /// a merchant something nobody verified. `Internal` (500, paged) is the
    /// honest answer, and
    /// `every_stored_refund_status_renders_and_decodes_in_the_merchant_sdk`
    /// is what keeps the two vocabularies from drifting: adding a label to
    /// the migration without adding it here fails that case rather than a
    /// merchant's read.
    ///
    /// [`vpay_db::RefundRow::status`] stays a `String` — the parse belongs to
    /// this boundary, which is the same split every other vocabulary in this
    /// crate uses.
    pub status: RefundStatus,
    /// The merchant's own reason, or `null`. Free text on purpose — the
    /// vocabulary is theirs, not vpay's.
    pub reason: Option<String>,
    /// The merchant's own key/value pairs, echoed back. `Map<String, Value>`
    /// for the reason [`PaymentIntentObject::metadata`] gives.
    pub metadata: Map<String, Value>,
    /// Unix **seconds**, like every other `created` here.
    pub created: i64,
    /// What the rail charged **us** to execute this refund, in minor units of
    /// [`currency`](Self::currency) — never a second currency and never a
    /// float, per `docs/flows/money.md`.
    ///
    /// # `null` is not `0`
    ///
    /// `null` means the rail did not report a fee. `0` means it reported the
    /// movement was free. Collapsing the two is the whole reason issue #46
    /// was filed: the integrator's own domain type had no `Option`, so it
    /// sent a hardcoded `0` into a merchant's settlement statement — a number
    /// nobody measured, presented as one somebody did.
    ///
    /// # It is `null` on every object this repository can currently produce
    ///
    /// Not as a placeholder: as the honest answer. Orange's Web Payment
    /// product documents no refund API at all, and MTN refunds are the
    /// Disbursements product no deployment here has been issued a credential
    /// for, so `vpay_provider::Refunded::fee` — the only thing that could
    /// ever fill this — has no producer. `docs/status.md` names what must
    /// exist before that changes.
    ///
    /// The key is still always present, `null` and all: this module emits
    /// every documented key on every object (see the module header). Both
    /// SDKs model the field as *optional* so that a vpay predating it still
    /// decodes, which is exactly why the server must not omit it — an absent
    /// key would decode without complaint in either client, and a merchant
    /// would simply never learn the field exists.
    pub fee: Option<i64>,
}

/// Stripe's `list` envelope.
///
/// Generic over the element type although only `payment_intent` is listed
/// today: `refund` and `event` are the same envelope
/// (`docs/api/README.md`), and a second hand-written copy of these four keys
/// is how `has_more` ends up meaning different things on different endpoints.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
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
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    /// use vpay_api::model::ListObject;
    ///
    /// let page = ListObject::new(vec!["pi_a", "pi_b"], true, "/v1/payment_intents");
    /// assert_eq!(
    ///     serde_json::to_value(&page).expect("a wire DTO always serialises"),
    ///     json!({
    ///         "object": "list",
    ///         "data": ["pi_a", "pi_b"],
    ///         "has_more": true,
    ///         "url": "/v1/payment_intents",
    ///     }),
    /// );
    /// ```
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

/// Renders a row's `metadata`, which a `metadata_is_object` CHECK
/// (migrations `0014` and `0017`) guarantees is a JSON object.
///
/// A non-object would mean that CHECK is gone or was bypassed, which is this
/// layer's invariant failing rather than anything a merchant did — hence
/// `Internal`, which pages, rather than an empty object that would quietly
/// tell the merchant their metadata was lost.
///
/// `table` is a parameter, and it is the only thing that varies between
/// callers: `Internal`'s payload is logged and never rendered, so its whole
/// value is that it names the table and column an operator has to go and
/// look at. A second copy of this function per table would be four lines
/// duplicated to hold one string.
fn metadata_of(value: &Value, table: &'static str) -> Result<Map<String, Value>, ApiError> {
    value.as_object().cloned().ok_or_else(|| {
        ApiError::Internal(format!(
            "{table}.metadata is {} rather than an object",
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
#[serde(rename_all = "snake_case")]
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
#[serde(rename_all = "snake_case")]
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
    /// migrations 0018 and 0029) where it is *written*, and a value that
    /// failed to parse on the read path would turn a merchant's
    /// `GET /v1/events` into a 500 instead of showing them the event.
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
            metadata: metadata_of(&row.metadata, "payment_intents")?,
            description: row.description.clone(),
            customer: row.customer_id.clone(),
            created: row.created_at.unix_timestamp(),
            livemode: row.livemode,
        })
    }
}

/// A `customer` (S4a): the merchant-owned record of a payer they expect to
/// see again.
///
/// # The eight keys, and the one field of the row that is deliberately not
/// among them
///
/// `customers.last_used_at` is **not** on the wire. It is the retention
/// sweep's clock — vpay moves it whenever an intent or a session names the
/// customer — and a merchant who could read it would build on a value whose
/// motion is vpay's business and whose meaning may widen the day invoices
/// exist. `the_customer_object_is_the_documented_eight_keys` below is the
/// tripwire: adding it here would put it in every `customer.*` webhook body,
/// signed and stored in `events` forever, before anybody wrote it down.
///
/// The three identifiers are `Option<String>` and at least one of them is
/// always present — migration `0034`'s `at_least_one_identifier`, which this
/// type deliberately does not re-express: a Rust enum over "which one is
/// here" would be a second copy of a rule the database already owns, and it
/// would have to be exhaustive over a set that grows.
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CustomerObject {
    /// `cus_…` — `vpay_core::ids::customer_id`.
    pub id: String,
    /// Always `"customer"`.
    pub object: CustomerTag,
    /// The payer's name, or `null`.
    pub name: Option<String>,
    /// The payer's email, or `null`.
    pub email: Option<String>,
    /// The payer's phone number, canonicalised (`2376XXXXXXXX`, no `+`), or
    /// `null`.
    ///
    /// Echoed back in the **canonical** form rather than as the merchant
    /// typed it, and that is a wire contract rather than an implementation
    /// leak: it is the value a rail is given and the value a merchant would
    /// compare against `payer_ref`, so rendering the original spelling would
    /// make two systems that hold the same number disagree about it.
    pub phone: Option<String>,
    /// The payer's postal address **and** GPS point, or `null` (issue #67).
    ///
    /// **One nullable object, not eight nullable keys**, which is Stripe's
    /// shape and is also the shape that matches how vpay treats it: an
    /// address is replaced whole by an update, and rendering the components
    /// at the top level would invite a merchant to patch one of them and get
    /// an address assembled out of two.
    ///
    /// The object carries two keys Stripe's does not —
    /// [`AddressObject::latitude_microdeg`] and its pair — because an address
    /// in vpay is both halves. See that type.
    ///
    /// `null` when every component is absent — [`vpay_db::CustomerAddress::is_empty`].
    /// A merchant reading `address` therefore never has to distinguish "no
    /// address" from "an address with nothing in it", because vpay does not
    /// store the second.
    pub address: Option<AddressObject>,
    /// The merchant's own key/value pairs, echoed back.
    ///
    /// `Map<String, Value>` for [`PaymentIntentObject::metadata`]'s reason.
    pub metadata: Map<String, Value>,
    /// Unix **seconds** — not milliseconds, and not RFC 3339.
    pub created: i64,
    /// `false` for a sandbox deployment's objects. Taken from the row, not
    /// from configuration read at render time, for
    /// [`PaymentIntentObject::livemode`]'s reason.
    pub livemode: bool,
    /// `true` on an **anonymised** customer, and absent otherwise (migration
    /// `0041`).
    ///
    /// # Why the key comes and goes rather than being `false`
    ///
    /// Stripe's shape, and this is the object where following it costs
    /// nothing and diverging costs something: a merchant's Stripe-shaped code
    /// tests `if (customer.deleted)`, which reads the same either way, but
    /// this object is `customer.*`'s `data.object` and every key here is
    /// signed into `events` for ever. A `deleted: false` on every live
    /// customer would be a key vpay publishes on every create and every
    /// update in order to say nothing.
    ///
    /// # What it means, which is not "this row is flagged"
    ///
    /// `DELETE /v1/customers/{id}` **anonymises** a customer an intent, a
    /// session or an invoice references — the foreign keys are `NO ACTION`,
    /// so the payment record survives, and the payer's identifiers do not.
    /// Every identifier on an object rendering `deleted: true` is the literal
    /// `[redacted]`; `metadata` is untouched, because that is the merchant's
    /// own data and not the payer's. A customer with no payment history is
    /// hard-deleted instead and a later `GET` is a `404`, so this key is
    /// something a merchant sees only for the customers vpay *had* to keep.
    /// `docs/flows/customers.md` carries the whole rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<DeletedTrue>,
}

/// A postal address on a [`CustomerObject`] — Stripe's six formal components
/// **and** the GPS point (migration `0041`,
/// [issue #67](https://github.com/vaam-apps/vpay/issues/67)).
///
/// Every component is nullable and rendered even when `null`, for
/// [`CustomerObject::name`]'s reason: a key that appears and disappears is a
/// shape change in a signed webhook body, and both SDKs decode all eight as
/// nullable. The object as a whole is `null` when there is no address at all,
/// which is the one distinction worth making.
///
/// # The two coordinate keys are a deliberate divergence from Stripe
///
/// Stripe's `address` has **no** coordinate. vpay's has, because the
/// maintainer's decision of 2026-09-11 is that *"address in our system means
/// both formal as well as GPS"*: formal addressing is unreliable across the
/// markets vpay serves, and a point is how a place is actually found. So a
/// merchant's Stripe-shaped decoder meets two keys it does not know, which is
/// additive and safe, and a merchant porting the other way loses them. That
/// is stated in `docs/flows/customers.md`, in `docs/api/README.md` and in
/// both SDKs' types rather than left to be discovered from a response body.
///
/// # The unit is in the field name, and the value is an integer
///
/// `latitude_microdeg` and `longitude_microdeg` are whole millionths of a
/// degree, so 4.061°N is `4061000`. **No JSON float ever appears on this
/// object**, which is what the naming buys: a field called `latitude` would
/// be read as degrees by every merchant who has ever used another API, and
/// the first one to send `4.061` would be storing a value CrateStack's
/// `Value::from_plain_json` demotes to `f64` and vpay's own ADR-0007 denies
/// arithmetic on. `vpay_db::CustomerAddress::latitude_microdeg` carries the
/// measurement.
///
/// It is deliberately **not** a `Deserialize` type. The request side spells
/// the same eight fields in `vpay_api::v1::customers`, because the wire is
/// form-encoded and every incoming value is text — the same split
/// [`CustomerObject`] and `CreateParams` already make.
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AddressObject {
    /// Street address, line 1.
    pub line1: Option<String>,
    /// Street address, line 2 — apartment, suite, PO box.
    pub line2: Option<String>,
    /// City, district, suburb, town or village.
    pub city: Option<String>,
    /// State, county, province or region.
    pub state: Option<String>,
    /// ZIP or postal code.
    pub postal_code: Option<String>,
    /// ISO 3166-1 alpha-2, upper case — `CM`, `FR`, `NG`.
    pub country: Option<String>,
    /// Latitude in **microdegrees** — a whole number, `4061000` for 4.061°N.
    ///
    /// `null` unless [`Self::longitude_microdeg`] is also set: half a
    /// coordinate names no place, and the pair is refused on the way in with
    /// a `400` and by `address_coordinates_are_both_or_neither` beneath it.
    ///
    /// An anonymised customer renders `null` here where the formal
    /// components render the redaction marker, and that asymmetry is the
    /// point rather than an oversight: there is no integer that is not a
    /// possible place, so a marker value would be a coordinate.
    pub latitude_microdeg: Option<i64>,
    /// Longitude in **microdegrees**. See [`Self::latitude_microdeg`].
    pub longitude_microdeg: Option<i64>,
}

impl From<&vpay_db::CustomerAddress> for AddressObject {
    fn from(address: &vpay_db::CustomerAddress) -> Self {
        Self {
            line1: address.line1.clone(),
            line2: address.line2.clone(),
            city: address.city.clone(),
            state: address.state.clone(),
            postal_code: address.postal_code.clone(),
            country: address.country.clone(),
            latitude_microdeg: address.latitude_microdeg,
            longitude_microdeg: address.longitude_microdeg,
        }
    }
}

/// Redacts every component, printing a count rather than a value — the same
/// judgement `vpay_db::CustomerRow`'s hand-written `Debug` makes for the row
/// this type is rendered from (see that impl). `CustomerObject::fmt` reaches
/// this type through its own `address` field, and without this impl a
/// derived one would defeat that redaction outright: a name is how somebody
/// is addressed, but a street and a GPS point are where they can be found,
/// which makes them at least as identifying, not less.
///
/// The coordinate pair counts as **one** component rather than two, for the
/// same reason it does in `CustomerRow`: `address_coordinates_are_both_or_neither`
/// makes the pair the value, and reporting two would claim a customer with a
/// point and a city has three components rather than two.
impl std::fmt::Debug for AddressObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let components = [
            self.line1.is_some(),
            self.line2.is_some(),
            self.city.is_some(),
            self.state.is_some(),
            self.postal_code.is_some(),
            self.country.is_some(),
            self.latitude_microdeg.is_some() || self.longitude_microdeg.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();

        f.debug_struct("AddressObject")
            .field(
                "redacted",
                &format_args!("{{{components} component(s) redacted}}"),
            )
            .finish()
    }
}

/// What `DELETE /v1/customers/{id}` answers with.
///
/// Stripe's own deleted-object shape — `id`, `object`, `deleted: true` — and
/// deliberately **not** the customer it removed. Two reasons, and the second
/// is the one that decides it:
///
/// * a merchant who just deleted a record does not need it handed back, and
/// * this response is the one most likely to be logged by an integration
///   confirming an erasure, so returning the payer's name, email and phone in
///   it would write the personal data into a log *because* it was deleted.
///
/// `deleted` is a [`DeletedTrue`] rather than a `bool` for the reason every
/// `object` field is a tag: `false` is not a value this response can carry,
/// and a `bool` field is one typo away from telling a merchant an erasure did
/// not happen when it did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct DeletedObject<T> {
    /// The id that was deleted.
    pub id: String,
    /// The `object` the id named. A **type parameter** rather than a field of
    /// one fixed tag, since 2026-09-07: three resources answer this shape
    /// (`customer`, `invoice`, `line_item`), Stripe's deleted shape carries
    /// the deleted object's own type, and an SDK switches on it. Making it
    /// generic is what stops a fourth resource answering `"customer"` for
    /// something that is not one — the compiler picks the tag from the call
    /// site's own value.
    pub object: T,
    /// Always `true`.
    pub deleted: DeletedTrue,
}

/// Serialises as `true` and holds nothing — [`DeletedObject::deleted`]'s
/// type. The [`object_tag!`] device applied to a boolean.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeletedTrue;

impl Serialize for DeletedTrue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}

impl TryFrom<&vpay_db::CustomerRow> for CustomerObject {
    type Error = ApiError;

    /// Renders a stored customer as the object a merchant reads.
    ///
    /// # Errors
    ///
    /// [`ApiError::Internal`] for a `metadata` that is not a JSON object — a
    /// state migration `0034`'s `metadata_is_object` CHECK makes impossible,
    /// so seeing one means the schema and this code disagree. Nothing a
    /// *caller* can send reaches an `Err` here; see this module's header on
    /// why the conversion is fallible at all.
    fn try_from(row: &vpay_db::CustomerRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id.clone(),
            object: CustomerTag,
            name: row.name.clone(),
            email: row.email.clone(),
            phone: row.phone.clone(),
            // `None` and not an object of eight nulls: an address with nothing
            // in it is not an address, and rendering one would make a
            // merchant write `customer.address?.line1 ?? null` where
            // `customer.address` already answers the question.
            address: (!row.address.is_empty()).then(|| AddressObject::from(&row.address)),
            metadata: metadata_of(&row.metadata, "customers")?,
            created: row.created_at.unix_timestamp(),
            livemode: row.livemode,
            // Derived from the row rather than passed in, so the one place
            // that can say "this payer was erased" is the column migration
            // `0041` will not let lie about it.
            deleted: row.anonymized_at.is_some().then_some(DeletedTrue),
        })
    }
}

/// Redacts the personal identifiers a customer object carries — `name`,
/// `email`, `phone`, and, through [`AddressObject`]'s own `Debug`, every
/// address component and the GPS pair — leaving every other field exactly as
/// a derived `Debug` would render it. The merchant collected these values
/// and holds them in their database; redacting them here would hide their
/// own data from them while doing nothing about the copy they already have.
/// The redaction place that *counts* is the server: vpay's `CustomerRow`
/// redacts the same set, because vpay's logs are not the merchant's. See
/// `docs/flows/customers.md`.
///
/// A street address and a GPS point are at least as identifying as a name,
/// so `address` cannot be the one field here left to a derive: delegating to
/// [`AddressObject`]'s own (also hand-written) `Debug` is what keeps this
/// impl from being a redaction with a hole in it.
impl std::fmt::Debug for CustomerObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        /// `[N chars redacted]`, or `None` — so "which identifiers does this
        /// customer have?" is still answerable from a log line.
        fn redacted(value: &Option<String>) -> String {
            value.as_ref().map_or_else(
                || "None".to_owned(),
                |value| format!("[{} chars redacted]", value.chars().count()),
            )
        }

        f.debug_struct("CustomerObject")
            .field("id", &self.id)
            .field("object", &self.object)
            .field("name", &format_args!("{}", redacted(&self.name)))
            .field("email", &format_args!("{}", redacted(&self.email)))
            .field("phone", &format_args!("{}", redacted(&self.phone)))
            .field("address", &self.address)
            .field("metadata", &self.metadata)
            .field("created", &self.created)
            .field("livemode", &self.livemode)
            .field("deleted", &self.deleted)
            .finish()
    }
}

/// One line on an invoice.
///
/// Stripe's `line_item`, narrowed to what a vpay invoice actually has: no
/// price, no product, no proration, no period. `docs/flows/invoices.md` lists
/// what is missing and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct InvoiceLineObject {
    /// `ii_…` — `vpay_core::ids::invoice_item_id`. The same id
    /// `/v1/invoice_items/{id}` addresses; see [`LineItemTag`] for why the
    /// `object` differs from the route.
    pub id: String,
    /// Always `"line_item"`.
    pub object: LineItemTag,
    /// The text on the document.
    pub description: String,
    /// How many.
    pub quantity: i64,
    /// The price of one, in integer minor units.
    pub unit_amount: i64,
    /// `quantity * unit_amount`, in integer minor units — checked by the
    /// database, never computed here.
    pub amount: i64,
    /// Lower-case ISO-4217, always the parent invoice's.
    pub currency: String,
    /// `false` for a sandbox deployment's objects.
    pub livemode: bool,
}

impl From<&vpay_db::InvoiceItemRow> for InvoiceLineObject {
    /// Infallible, unlike every other conversion in this module, and that is
    /// a property of the table rather than an oversight: `invoice_items` has
    /// no `jsonb` column, no enum column and no nullable text — nothing that
    /// could be stored in a shape this renderer would have to reject. See
    /// this module's header for why the others are `TryFrom`.
    fn from(row: &vpay_db::InvoiceItemRow) -> Self {
        Self {
            id: row.id.clone(),
            object: LineItemTag,
            description: row.description.clone(),
            quantity: row.quantity,
            unit_amount: row.unit_amount,
            amount: row.amount,
            currency: row.currency_code.to_lowercase(),
            livemode: row.livemode,
        }
    }
}

/// When each of an invoice's transitions happened, in unix seconds.
///
/// Stripe's `status_transitions`, and one nested object rather than four
/// top-level keys for the reason Stripe has it: they are answers to one
/// question ("what happened to this document, and when"), and a handler that
/// renders a timeline reads them together.
///
/// Every field is `null` until the transition happens, and at most two of
/// them are ever non-null — `finalized_at` plus whichever terminal one
/// applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct InvoiceStatusTransitions {
    /// When the invoice was issued and got its number.
    pub finalized_at: Option<i64>,
    /// When a settlement paid it.
    pub paid_at: Option<i64>,
    /// When the merchant voided it.
    pub voided_at: Option<i64>,
    /// When the merchant wrote it off.
    pub marked_uncollectible_at: Option<i64>,
}

/// An `invoice` (S4b): a merchant's bill to one customer.
///
/// # `lines` is expanded, always, and `hosted_invoice_url` is derived
///
/// Two of the keys below do not come from the `invoices` row:
///
/// * `lines` is a second query (`vpay_db::Invoices::items_for_invoice`).
///   Expanded unconditionally rather than behind Stripe's `expand[]`, because
///   an invoice without its lines is a total with no explanation, and vpay
///   does not implement `expand[]` at all (`docs/api/README.md`). The
///   envelope is the ordinary [`ListObject`] so a Stripe-shaped client's
///   `invoice.lines.data` works unchanged; `has_more` is always `false`
///   because the list is never paged, and that is stated in
///   `docs/flows/invoices.md` rather than left to be discovered.
/// * `hosted_invoice_url` is the **checkout session** for this invoice's
///   payment intent, not a page of its own. vpay has no invoice page: the
///   payer sees the existing hosted checkout, which shows the merchant's name
///   and the amount. It is `null` until `POST /v1/invoices/{id}/pay` has run.
///
/// # `amount_*` are integer minor units, and `currency` is lower-case
///
/// `docs/flows/money.md` and Stripe's own convention respectively. XAF is
/// zero-decimal, so `5000` means 5,000 FCFA and there is no division
/// anywhere.
///
/// # Nineteen keys, and the count is the tripwire
///
/// `the_invoice_object_is_the_documented_nineteen_keys` below is what keeps
/// `docs/api/README.md`'s listing honest, and it exists for the reason
/// [`CustomerObject`]'s twin does: this struct is the `data.object` of all
/// four `invoice.*` event types, so a nineteenth key is signed, delivered
/// at-least-once and stored in `events` **forever** — the one place vpay
/// cannot retract a field it has published. `InvoiceRow` carries `seq`,
/// `merchant_id` and `updated_at`, none of which belongs on a merchant's
/// wire.
///
/// The README said *seventeen* from the day this object landed until the S4b
/// review on 2026-09-07. The object was eighteen the whole time and no test
/// of any name held the number. It is **nineteen** since 2026-09-10, when
/// migration `0042` added `amount_refunded` (issue #91, D5) — the count, the
/// test's name and `docs/api/README.md` moved in the same commit, which is
/// what the tripwire is for.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct InvoiceObject {
    /// `in_…` — `vpay_core::ids::invoice_id`.
    pub id: String,
    /// Always `"invoice"`.
    pub object: InvoiceTag,
    /// The `cus_…` this invoice bills. Never `null`, unlike a payment
    /// intent's — migration `0036` makes the column `NOT NULL`.
    pub customer: String,
    /// Lower-case ISO-4217.
    pub currency: String,
    /// One of [`vpay_core::InvoiceStatus`]' five labels.
    pub status: vpay_core::InvoiceStatus,
    /// `{prefix}-{000001}`, or `null` while this is a draft. The number a
    /// human quotes; [`Self::id`] is the one an API call uses.
    pub number: Option<String>,
    /// The total, in integer minor units. Summed from [`Self::lines`] while
    /// the invoice is a draft, frozen at finalize.
    pub amount_due: i64,
    /// How much has been paid — `0`, or [`Self::amount_due`]. Partial
    /// payments are out of scope (`docs/flows/invoices.md`).
    pub amount_paid: i64,
    /// `amount_due - amount_paid`, and the amount
    /// `POST /v1/invoices/{id}/pay` creates an intent for.
    pub amount_remaining: i64,
    /// What has been given back out of [`Self::amount_paid`] — a **gross**
    /// total that is not subtracted from it, so a refunded invoice is still
    /// `paid` with [`Self::amount_remaining`] at `0` (D5; migration `0042`).
    ///
    /// `0` on every invoice in every deployment today: no rail can refund
    /// (`docs/status.md`). Rendered anyway, and never omitted when zero,
    /// because a key that appears only sometimes is a key a merchant's typed
    /// client has to guess at.
    pub amount_refunded: i64,
    /// Unix **seconds**, or `null`. **Advisory**: nothing in vpay acts on it
    /// — there is no dunning and no automatic transition.
    pub due_date: Option<i64>,
    /// The merchant's note on the document.
    pub description: Option<String>,
    /// The merchant's own key/value pairs, echoed back.
    pub metadata: Map<String, Value>,
    /// The `pi_…` paying, or that paid, this invoice — `null` until
    /// `POST /v1/invoices/{id}/pay`.
    pub payment_intent: Option<String>,
    /// Where to send the payer. The checkout session for [`Self::payment_intent`],
    /// or `null`. See the struct doc.
    pub hosted_invoice_url: Option<String>,
    /// Every line, newest last — the order they were added.
    pub lines: ListObject<InvoiceLineObject>,
    /// When each transition happened.
    pub status_transitions: InvoiceStatusTransitions,
    /// Unix **seconds** — not milliseconds, and not RFC 3339.
    pub created: i64,
    /// `false` for a sandbox deployment's objects.
    pub livemode: bool,
}

impl InvoiceObject {
    /// The `url` an invoice's `lines` envelope carries.
    ///
    /// A real, mounted path rather than Stripe's `/v1/invoices/{id}/lines`
    /// (which vpay does not serve): a `url` pointing at a `404` is a claim
    /// this repository does not make. `/v1/invoice_items` is where a line is
    /// created, read, changed and removed.
    const LINES_URL: &'static str = "/v1/invoice_items";

    /// Renders a stored invoice, its lines and its hosted URL as the object a
    /// merchant reads.
    ///
    /// Three inputs rather than one, and each is a query the caller has
    /// already made: the row, its lines
    /// (`vpay_db::Invoices::items_for_invoice`) and the checkout session's
    /// URL. Taking them as parameters rather than fetching them keeps this
    /// module free of I/O — the property that lets every handler and the
    /// worker share one renderer.
    ///
    /// # Errors
    ///
    /// [`ApiError::Internal`] for a `metadata` that is not a JSON object or a
    /// `status` outside [`vpay_core::InvoiceStatus`] — states migration
    /// `0036`'s `metadata_is_object` and `invoices_status_enum_check` make
    /// impossible, so seeing one means the schema and this code disagree.
    /// Nothing a *caller* can send reaches an `Err` here.
    pub fn render(
        row: &vpay_db::InvoiceRow,
        lines: &[vpay_db::InvoiceItemRow],
        hosted_invoice_url: Option<String>,
    ) -> Result<Self, ApiError> {
        let status = vpay_core::InvoiceStatus::from_wire(&row.status).ok_or_else(|| {
            ApiError::Internal(format!(
                "invoices.status is `{}`, which is not an invoice status",
                row.status
            ))
        })?;

        Ok(Self {
            id: row.id.clone(),
            object: InvoiceTag,
            customer: row.customer_id.clone(),
            currency: row.currency_code.to_lowercase(),
            status,
            number: row.number.clone(),
            amount_due: row.amount_due,
            amount_paid: row.amount_paid,
            amount_remaining: row.amount_remaining,
            amount_refunded: row.amount_refunded,
            due_date: row.due_date.map(OffsetDateTime::unix_timestamp),
            description: row.description.clone(),
            metadata: metadata_of(&row.metadata, "invoices")?,
            payment_intent: row.payment_intent_id.clone(),
            hosted_invoice_url,
            lines: ListObject::new(
                lines.iter().map(InvoiceLineObject::from).collect(),
                // Never `true`: the list is not paged. Stated on the struct.
                false,
                Self::LINES_URL,
            ),
            status_transitions: InvoiceStatusTransitions {
                finalized_at: row.finalized_at.map(OffsetDateTime::unix_timestamp),
                paid_at: row.paid_at.map(OffsetDateTime::unix_timestamp),
                voided_at: row.voided_at.map(OffsetDateTime::unix_timestamp),
                marked_uncollectible_at: row
                    .marked_uncollectible_at
                    .map(OffsetDateTime::unix_timestamp),
            },
            created: row.created_at.unix_timestamp(),
            livemode: row.livemode,
        })
    }
}

impl TryFrom<&vpay_db::RefundRow> for RefundObject {
    type Error = ApiError;

    /// Renders a stored refund as the object a merchant reads, on both
    /// surfaces [`RefundObject`] serves.
    ///
    /// # Errors
    ///
    /// [`ApiError::Internal`] for a `status` outside [`RefundStatus`] or a
    /// `metadata` that is not a JSON object — both states
    /// `refunds_status_enum_check` (migration `0037`, which replaced the
    /// `refund_status` enum) and the `metadata_is_object` CHECK
    /// (`backends/migrations/0017_create-refunds.sql`) make impossible, so
    /// seeing one means the schema and this code disagree and the row is
    /// corrupt. Nothing a *caller* can send reaches an `Err` here; see this
    /// module's header on why the conversion is fallible at all.
    fn try_from(row: &vpay_db::RefundRow) -> Result<Self, Self::Error> {
        let status = RefundStatus::from_wire(&row.status).ok_or_else(|| {
            ApiError::Internal(format!(
                "refunds.status holds `{}`, which is not a RefundStatus",
                row.status
            ))
        })?;

        Ok(Self {
            id: row.id.clone(),
            object: RefundTag,
            amount: row.amount,
            currency: row.currency_code.to_lowercase(),
            payment_intent: row.payment_intent_id.clone(),
            status,
            reason: row.reason.clone(),
            metadata: metadata_of(&row.metadata, "refunds")?,
            created: row.created_at.unix_timestamp(),
            // Carried, never defaulted. `row.fee.unwrap_or(0)` would compile,
            // pass every other assertion about this object, and tell a
            // merchant a movement was free when nobody asked the rail —
            // which is exactly the bug issue #46 reports one layer up.
            fee: row.fee,
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
            "customer": null,
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
            customer: None,
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
            // `customer` since 2026-09-06 (S4a). It was **absent** before
            // that and the request field was dropped, so a merchant SDK
            // predating it models the key as optional — which is exactly why
            // the server must not omit it: an absent key decodes without
            // complaint in either client, and a merchant would simply never
            // learn the field exists.
            "customer",
            "created",
            "livemode",
        ] {
            assert!(object.contains_key(key), "`{key}` is missing");
        }
        assert_eq!(
            object.len(),
            13,
            "an undocumented key was added: {object:?}"
        );
        for null_key in [
            "next_action",
            "last_payment_error",
            "description",
            "customer",
        ] {
            assert_eq!(
                object.get(null_key),
                Some(&Value::Null),
                "`{null_key}` must be present and null, not absent"
            );
        }
    }

    /// The browser wrapper is the thirteen keys **plus one**, at the top level
    /// — not a nested object.
    ///
    /// `sdks/stripe-js/src/types.ts`'s `PaymentIntent` is written against
    /// exactly this shape, and its `testing/browser-stub.ts` builds it key by
    /// key. A missing `#[serde(flatten)]` would nest the intent under
    /// `intent` and every browser call would decode to something with no
    /// `status` on it.
    #[test]
    fn the_browser_wrapper_is_the_thirteen_keys_plus_the_client_secret() {
        let rendered = serde_json::to_value(PaymentIntentWithSecret::new(
            sample("pi_1"),
            "pi_1_secret_abc".to_owned(),
        ))
        .expect("serialises");
        let object = rendered.as_object().expect("an object");

        assert_eq!(
            object.len(),
            14,
            "the browser object must be exactly the thirteen documented keys plus \
             `client_secret` — got a different key count"
        );
        assert_eq!(
            object.get("client_secret").and_then(Value::as_str),
            Some("pi_1_secret_abc")
        );
        // Every one of the thirteen is still there, at the top level, byte for
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
    /// pins `len() == 13`; this says out loud what that number is protecting,
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
        assert_eq!(object.len(), 13);
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
        assert_eq!(
            serde_json::to_value(RefundTag).expect("serialises"),
            json!("refund")
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

    // --------------------------------------------------------- refunds ----

    /// One `refunds` row (migrations `0017` + `0031`), with the fee left
    /// unreported — which is the only thing either rail can currently
    /// produce.
    fn refund_row(fee: Option<i64>) -> vpay_db::RefundRow {
        vpay_db::RefundRow {
            id: "re_1".to_owned(),
            payment_intent_id: "pi_1".to_owned(),
            amount: 2_000,
            // Uppercase on the row, lowercase on the wire.
            currency_code: "XAF".to_owned(),
            status: "pending".to_owned(),
            reason: None,
            metadata: json!({ "order_id": "1234" }),
            fee,
            created_at: time::OffsetDateTime::from_unix_timestamp(1_753_401_600)
                .expect("a fixed, valid timestamp"),
        }
    }

    /// A customer row with every field populated, including the two
    /// `docs/flows/customers.md` promises are **not** on the wire.
    ///
    /// `seq`, `last_used_at` and `updated_at` carry distinctive values on
    /// purpose: a render that leaked one would put a recognisable number in
    /// the assertion below rather than something that could be mistaken for
    /// `created`.
    fn customer_row() -> vpay_db::CustomerRow {
        vpay_db::CustomerRow {
            id: "cus_1".to_owned(),
            seq: 4_242,
            merchant_id: "acme-cameroon-tenant".to_owned(),
            livemode: false,
            name: Some("Ada Ngo".to_owned()),
            email: Some("ada@example.cm".to_owned()),
            phone: Some("237600000200".to_owned()),
            address: vpay_db::CustomerAddress::default(),
            metadata: json!({ "order_id": "1234" }),
            anonymized_at: None,
            last_used_at: time::OffsetDateTime::from_unix_timestamp(1_784_937_600)
                .expect("a fixed, valid timestamp"),
            created_at: time::OffsetDateTime::from_unix_timestamp(1_753_401_600)
                .expect("a fixed, valid timestamp"),
            updated_at: time::OffsetDateTime::from_unix_timestamp(1_784_937_600)
                .expect("a fixed, valid timestamp"),
        }
    }

    /// The object `docs/flows/customers.md` documents, key for key.
    ///
    /// # This test is named by three places and did not exist
    ///
    /// `CustomerObject`'s own doc comment and `docs/flows/customers.md` both
    /// cited `the_customer_object_is_the_documented_seven_keys` as the
    /// tripwire that keeps `customers.last_used_at` off the wire. Until
    /// 2026-09-07 nothing of that name existed, and the claim was measured:
    /// adding `last_used_at` to [`CustomerObject`] and rendering it left
    /// `vpay-api` 422/422 green, `vpay-sdk` green, the fourteen
    /// container-backed cases in
    /// `backends/tests/integration/tests/customers.rs` green, and the Node
    /// SDK's 190 green. Nothing in the repository objected.
    ///
    /// **And the count those three places gave was wrong.** The object is
    /// `id`, `object`, `name`, `email`, `phone`, `metadata`, `created`,
    /// `livemode` — *eight* keys, which is also exactly what
    /// `docs/flows/customers.md`'s own table lists row for row while its
    /// prose said seven. The name below is the measured count, and the two
    /// documents were corrected to match rather than the other way round.
    ///
    /// # Why the count is the assertion and not only the key list
    ///
    /// `last_used_at` is the retention sweep's clock. A leak of it is not a
    /// cosmetic extra key: this object is `customer.deleted`'s `data.object`,
    /// so an extra key is signed, delivered at-least-once and stored in
    /// `events` **forever** — the one place vpay cannot retract a field it
    /// published. `the_refund_object_is_the_documented_ten_keys`' device,
    /// applied to the object where the cost of being wrong is highest.
    ///
    /// The whole-value comparison at the end is what makes this a statement
    /// about the *rendering* as well as the key set: `phone` canonical,
    /// `created` in unix **seconds**, and `metadata` a map rather than a
    /// string.
    ///
    /// # Eight -> nine on 2026-09-10 (issue #67)
    ///
    /// `address` is the ninth, and it is rendered `null` here rather than
    /// omitted: the fixture has no address, and a key that appears and
    /// disappears is a shape change in a signed body. `deleted` is the one
    /// key that *is* conditional and it is absent from this object — a live
    /// customer says nothing about being deleted. The erased shape is
    /// [`an_anonymised_customer_renders_deleted_true_and_no_identifier`](self).
    #[test]
    fn the_customer_object_is_the_documented_nine_keys() {
        let rendered = serde_json::to_value(
            CustomerObject::try_from(&customer_row()).expect("a well-formed row renders"),
        )
        .expect("serialises");
        let object = rendered.as_object().expect("an object");

        for key in [
            "id", "object", "name", "email", "phone", "address", "metadata", "created", "livemode",
        ] {
            assert!(object.contains_key(key), "`{key}` is missing");
        }

        for internal in [
            "last_used_at",
            "updated_at",
            "seq",
            "merchant_id",
            "anonymized_at",
        ] {
            assert!(
                !object.contains_key(internal),
                "`{internal}` is internal and must never reach the wire — it would be signed \
                 into every `customer.deleted` body and stored in `events` forever: {object:?}"
            );
        }

        assert!(
            !object.contains_key("deleted"),
            "`deleted` is Stripe's conditional key and belongs on an erased customer only; \
             rendering it `false` on every live one would publish a key that says nothing \
             into every `customer.created` body vpay ever signs: {object:?}"
        );

        assert_eq!(
            object.len(),
            9,
            "an undocumented key was added to the customer object: {object:?}"
        );

        assert_eq!(
            rendered,
            json!({
                "id": "cus_1",
                "object": "customer",
                "name": "Ada Ngo",
                "email": "ada@example.cm",
                "phone": "237600000200",
                "address": null,
                "metadata": { "order_id": "1234" },
                "created": 1_753_401_600,
                "livemode": false,
            })
        );
    }

    /// An address renders as **one nested object with all eight components**,
    /// nulls included, and never as eight top-level keys.
    ///
    /// The fixed-key shape is what a merchant's Stripe-shaped code reads, and
    /// the nulls are what stops the object's shape depending on which
    /// components a payer happened to fill in — the same rule
    /// [`a_phone_only_customer_renders_the_absent_identifiers_as_null`](self)
    /// states for the identifiers, applied one level down.
    ///
    /// Six of the eight are Stripe's; `latitude_microdeg` and
    /// `longitude_microdeg` are vpay's own (the maintainer, 2026-09-11) and
    /// are rendered `null` here because this fixture's payer gave a street
    /// and no point. `an_address_may_be_a_point_with_no_street_at_all` is the
    /// other direction.
    #[test]
    fn an_address_renders_as_one_nested_object_with_every_component() {
        let mut row = customer_row();
        row.address = vpay_db::CustomerAddress {
            line1: Some("12 Rue Njo-Njo".to_owned()),
            city: Some("Douala".to_owned()),
            country: Some("CM".to_owned()),
            ..vpay_db::CustomerAddress::default()
        };

        let rendered = serde_json::to_value(
            CustomerObject::try_from(&row).expect("a row with an address renders"),
        )
        .expect("serialises");

        assert_eq!(
            rendered.get("address"),
            Some(&json!({
                "line1": "12 Rue Njo-Njo",
                "line2": null,
                "city": "Douala",
                "state": null,
                "postal_code": null,
                "country": "CM",
                "latitude_microdeg": null,
                "longitude_microdeg": null,
            })),
            "the address is one object of eight keys: {rendered:?}"
        );
    }

    /// An address may be a **point with no street at all**, and it still
    /// renders as an address rather than as `null`.
    ///
    /// This is the case the maintainer's decision of 2026-09-11 exists for.
    /// Across vpay's markets a payer often cannot give a street that resolves
    /// to anything, and the coordinate is the only half they have. If
    /// `CustomerAddress::is_empty` did not count the two coordinate fields,
    /// this object would render `address: null` over a stored pair — the wire
    /// disagreeing with the row about whether vpay holds a payer's position,
    /// which is the disagreement that matters most to be wrong about.
    ///
    /// The mutation: drop the two `is_empty` conjuncts in `vpay-db` and this
    /// fails on the first assertion.
    #[test]
    fn an_address_may_be_a_point_with_no_street_at_all() {
        let mut row = customer_row();
        row.address = vpay_db::CustomerAddress {
            latitude_microdeg: Some(4_061_000),
            longitude_microdeg: Some(9_786_000),
            ..vpay_db::CustomerAddress::default()
        };

        let rendered = serde_json::to_value(
            CustomerObject::try_from(&row).expect("a row with only a point renders"),
        )
        .expect("serialises");

        assert_eq!(
            rendered.get("address"),
            Some(&json!({
                "line1": null,
                "line2": null,
                "city": null,
                "state": null,
                "postal_code": null,
                "country": null,
                "latitude_microdeg": 4_061_000,
                "longitude_microdeg": 9_786_000,
            })),
            "a customer whose whole address is a point has an address: {rendered:?}"
        );

        // And the value is a JSON **number**, not a string and not a float.
        // `serde_json::Value::is_i64` is the assertion that a future change
        // to `f64` would fail: `4061000.0` is `is_f64`, serialises as
        // `4061000.0`, and every other assertion here would still pass.
        let latitude = rendered
            .pointer("/address/latitude_microdeg")
            .expect("the key is present");
        assert!(
            latitude.is_i64(),
            "a coordinate on this API is an integer count of microdegrees; a float here \
             would be a value CrateStack demotes and ADR-0007 denies arithmetic on: \
             {latitude:?}"
        );
        let address = rendered
            .pointer("/address")
            .expect("the address is present")
            .to_string();
        assert!(
            !address.contains('.'),
            "no decimal point may appear anywhere in a rendered address — an email may \
             carry one, a coordinate may not: {address}"
        );
    }

    /// An erased payer's coordinates come back **`null`**, and not the
    /// redaction marker.
    ///
    /// The six formal components carry the marker because it says "there was
    /// a payer here and vpay erased them", which is a fact worth keeping.
    /// These two cannot: they are integers on the wire and in the column, and
    /// there is no integer that is not a possible place — a marker value
    /// would BE a coordinate, somewhere real, on an object claiming the payer
    /// is gone. So for these two the erasure is the absence, and
    /// `anonymized_at` (not on the wire) plus `deleted: true` (which is)
    /// carry the "there was somebody here" that the marker carries elsewhere.
    ///
    /// Asserted as its own case rather than only inside
    /// [`an_anonymised_customer_renders_deleted_true_and_no_identifier`](self)
    /// because the mutation it catches is the plausible one: making
    /// `CustomerAddress::redacted` uniform. That would not compile with a
    /// `&str` marker, so the shape it would really take is dropping these two
    /// from `redacted` entirely — leaving the payer's real point on the
    /// object vpay signs and stores for ever. This is the assertion that
    /// names it.
    #[test]
    fn an_erased_payers_coordinates_are_null_and_not_a_marker() {
        let mut row = customer_row();
        row.address = vpay_db::CustomerAddress {
            line1: Some("12 Rue Njo-Njo".to_owned()),
            latitude_microdeg: Some(4_061_000),
            longitude_microdeg: Some(9_786_000),
            ..vpay_db::CustomerAddress::default()
        };

        let erased = row.redacted(
            time::OffsetDateTime::from_unix_timestamp(1_784_937_600)
                .expect("a fixed, valid timestamp"),
        );
        let rendered = serde_json::to_value(
            CustomerObject::try_from(&erased).expect("an anonymised row renders"),
        )
        .expect("serialises");

        assert_eq!(
            rendered.pointer("/address/latitude_microdeg"),
            Some(&Value::Null),
            "the payer's position survived the erasure into the body vpay signs and stores \
             for ever: {rendered}"
        );
        assert_eq!(
            rendered.pointer("/address/longitude_microdeg"),
            Some(&Value::Null)
        );
        assert!(
            !rendered.to_string().contains("4061000") && !rendered.to_string().contains("9786000"),
            "neither half of the payer's point may appear anywhere in the erased object: \
             {rendered}"
        );
        // The key is still THERE, null — an erased customer and a live one
        // without a point have the same shape, so the presence of the key
        // says nothing about whether this payer ever gave a position.
        assert!(
            rendered
                .pointer("/address")
                .and_then(Value::as_object)
                .is_some_and(|address| address.len() == 8),
            "the address is eight keys whether or not the payer had a point: {rendered}"
        );
        // And the formal half is still the marker, so this test cannot pass
        // by the redaction having stopped redacting.
        assert_eq!(
            rendered.pointer("/address/line1"),
            Some(&json!(vpay_db::REDACTED))
        );
    }

    /// An anonymised customer renders `deleted: true`, the merchant's own
    /// `metadata`, and **not one identifier of the payer's**.
    ///
    /// This is the object a merchant reads after `DELETE /v1/customers/{id}`
    /// on a customer with payment history, and it is `customer.deleted`'s
    /// `data.object` — signed and stored in `events` for ever. So the
    /// assertion is not "the fields changed": it walks every value in the
    /// rendered object and refuses to find any of the three literals the
    /// fixture was built with, which is what catches a component this code
    /// forgot to redact rather than only the ones it remembered.
    #[test]
    fn an_anonymised_customer_renders_deleted_true_and_no_identifier() {
        let mut row = customer_row();
        row.address = vpay_db::CustomerAddress {
            line1: Some("12 Rue Njo-Njo".to_owned()),
            city: Some("Douala".to_owned()),
            country: Some("CM".to_owned()),
            ..vpay_db::CustomerAddress::default()
        };

        let erased = row.redacted(
            time::OffsetDateTime::from_unix_timestamp(1_784_937_600)
                .expect("a fixed, valid timestamp"),
        );
        let rendered = serde_json::to_value(
            CustomerObject::try_from(&erased).expect("an anonymised row renders"),
        )
        .expect("serialises");
        let object = rendered.as_object().expect("an object");

        assert_eq!(object.get("deleted"), Some(&json!(true)));
        assert_eq!(object.len(), 10, "nine keys plus `deleted`: {object:?}");

        // `metadata` is the MERCHANT's data, not the payer's, and destroying
        // it would be vpay deleting a merchant's records to keep a promise
        // made to somebody else.
        assert_eq!(object.get("metadata"), Some(&json!({ "order_id": "1234" })));
        assert_eq!(object.get("id"), Some(&json!("cus_1")));
        assert_eq!(object.get("created"), Some(&json!(1_753_401_600)));

        let text = rendered.to_string();
        for identifier in [
            "Ada Ngo",
            "ada@example.cm",
            "237600000200",
            "Njo-Njo",
            "Douala",
        ] {
            assert!(
                !text.contains(identifier),
                "`{identifier}` survived the redaction into the body vpay signs and stores \
                 for ever: {text}"
            );
        }
        assert_eq!(object.get("name"), Some(&json!(vpay_db::REDACTED)));
        assert_eq!(
            object.get("address"),
            Some(&json!({
                "line1": vpay_db::REDACTED,
                "line2": vpay_db::REDACTED,
                "city": vpay_db::REDACTED,
                "state": vpay_db::REDACTED,
                "postal_code": vpay_db::REDACTED,
                "country": vpay_db::REDACTED,
                // NULL and not the marker, and the asymmetry is the whole of
                // the reason this is asserted as a literal object rather than
                // key by key: these two are integers on the wire, so the
                // marker is not a value they can take, and there is no
                // integer that is not a possible place.
                // `an_erased_payers_coordinates_are_null_and_not_a_marker`
                // is where that is the *subject* rather than a corner of a
                // bigger assertion.
                "latitude_microdeg": null,
                "longitude_microdeg": null,
            })),
            "every component, including the ones this payer never filled in: which fields a \
             record carried is itself information about the person"
        );
    }

    /// One stored invoice, its one line and a hosted URL — the fixture the
    /// key-set assertion below renders.
    fn invoice_row() -> vpay_db::InvoiceRow {
        vpay_db::InvoiceRow {
            id: "in_1".to_owned(),
            seq: 7,
            merchant_id: "acme-cameroon-tenant".to_owned(),
            livemode: false,
            customer_id: "cus_1".to_owned(),
            currency_code: "XAF".to_owned(),
            status: "open".to_owned(),
            number: Some("A7K3M9QP-000001".to_owned()),
            amount_due: 5000,
            amount_paid: 0,
            amount_remaining: 5000,
            amount_refunded: 0,
            due_date: None,
            description: Some("September hosting".to_owned()),
            metadata: json!({ "order_id": "1234" }),
            payment_intent_id: None,
            finalized_at: time::OffsetDateTime::from_unix_timestamp(1_753_401_600).ok(),
            paid_at: None,
            voided_at: None,
            marked_uncollectible_at: None,
            created_at: time::OffsetDateTime::from_unix_timestamp(1_753_401_600)
                .expect("a fixed, valid timestamp"),
            updated_at: time::OffsetDateTime::from_unix_timestamp(1_784_937_600)
                .expect("a fixed, valid timestamp"),
        }
    }

    /// One line of that invoice.
    fn invoice_line_row() -> vpay_db::InvoiceItemRow {
        vpay_db::InvoiceItemRow {
            id: "ii_1".to_owned(),
            seq: 3,
            invoice_id: "in_1".to_owned(),
            merchant_id: "acme-cameroon-tenant".to_owned(),
            livemode: false,
            description: "Hosting".to_owned(),
            quantity: 1,
            unit_amount: 5000,
            amount: 5000,
            currency_code: "XAF".to_owned(),
            created_at: time::OffsetDateTime::from_unix_timestamp(1_753_401_600)
                .expect("a fixed, valid timestamp"),
            updated_at: time::OffsetDateTime::from_unix_timestamp(1_753_401_600)
                .expect("a fixed, valid timestamp"),
        }
    }

    /// The object `docs/api/README.md` documents, key for key.
    ///
    /// # The count was wrong and nothing held it
    ///
    /// `docs/api/README.md` said **seventeen keys** from the day S4b landed
    /// until the review on 2026-09-07. The object was eighteen then and is
    /// **nineteen** since migration `0042` added `amount_refunded`, and —
    /// unlike
    /// the customer's and the refund's, whose counts each name a test — no
    /// test of any name existed: adding a key to [`InvoiceObject`] and
    /// rendering it was caught by nothing in the repository. This is
    /// `the_customer_object_is_the_documented_eight_keys`' device applied to
    /// the object that inherited its documentation habit without its
    /// tripwire.
    ///
    /// # Why the count is the assertion and not only the key list
    ///
    /// This object is the `data.object` of `invoice.created`,
    /// `invoice.finalized`, `invoice.paid` and `invoice.voided`. A twentieth
    /// key is signed, delivered at-least-once and stored in `events`
    /// **forever**. `InvoiceRow`'s `seq`, `merchant_id` and `updated_at` are
    /// each one field's inattention away from being there, so they are named.
    ///
    /// The whole-value comparison is what makes this a statement about the
    /// *rendering* too: `currency` lower-cased from the stored `XAF`, every
    /// timestamp in unix **seconds**, `lines` the ordinary list envelope
    /// pointing at a route that exists, and `metadata` a map rather than a
    /// string.
    #[test]
    fn the_invoice_object_is_the_documented_nineteen_keys() {
        let rendered = serde_json::to_value(
            InvoiceObject::render(
                &invoice_row(),
                &[invoice_line_row()],
                Some("https://checkout.vpay.test/c/cs_1#secret".to_owned()),
            )
            .expect("a well-formed row renders"),
        )
        .expect("serialises");
        let object = rendered.as_object().expect("an object");

        for key in [
            "id",
            "object",
            "customer",
            "currency",
            "status",
            "number",
            "amount_due",
            "amount_paid",
            "amount_remaining",
            "amount_refunded",
            "due_date",
            "description",
            "metadata",
            "payment_intent",
            "hosted_invoice_url",
            "lines",
            "status_transitions",
            "created",
            "livemode",
        ] {
            assert!(object.contains_key(key), "`{key}` is missing");
        }

        for internal in ["seq", "merchant_id", "updated_at", "currency_code"] {
            assert!(
                !object.contains_key(internal),
                "`{internal}` is internal and must never reach the wire — it would be signed \
                 into every `invoice.*` body and stored in `events` forever: {object:?}"
            );
        }

        assert_eq!(
            object.len(),
            19,
            "an undocumented key was added to the invoice object: {object:?}"
        );

        assert_eq!(
            rendered,
            json!({
                "id": "in_1",
                "object": "invoice",
                "customer": "cus_1",
                "currency": "xaf",
                "status": "open",
                "number": "A7K3M9QP-000001",
                "amount_due": 5000,
                "amount_paid": 0,
                "amount_remaining": 5000,
                "amount_refunded": 0,
                "due_date": null,
                "description": "September hosting",
                "metadata": { "order_id": "1234" },
                "payment_intent": null,
                "hosted_invoice_url": "https://checkout.vpay.test/c/cs_1#secret",
                "lines": {
                    "object": "list",
                    "has_more": false,
                    "url": "/v1/invoice_items",
                    "data": [{
                        "id": "ii_1",
                        "object": "line_item",
                        "description": "Hosting",
                        "quantity": 1,
                        "unit_amount": 5000,
                        "amount": 5000,
                        "currency": "xaf",
                        "livemode": false,
                    }],
                },
                "status_transitions": {
                    "finalized_at": 1_753_401_600,
                    "paid_at": null,
                    "voided_at": null,
                    "marked_uncollectible_at": null,
                },
                "created": 1_753_401_600,
                "livemode": false,
            })
        );
    }

    /// `amount_refunded` is rendered **from the row**, and a non-zero value
    /// is what proves it.
    ///
    /// Every other assertion about this key — here, in both merchant SDKs'
    /// fixtures, in the integration suite — reads `0`, which is also what
    /// `InvoiceRow`'s zero value, a hard-coded literal in [`InvoiceObject::render`]
    /// and `vpay_sdk::Invoice`'s `#[serde(default)]` all produce. Measured on
    /// 2026-09-11: replacing `amount_refunded: row.amount_refunded` with
    /// `amount_refunded: 0` in that renderer left all 342 cases in this crate
    /// GREEN, and no case anywhere in the workspace reads the key off a wire
    /// response at all. This is the case that fails.
    ///
    /// It is a *money* key: it tells a merchant how much of a bill they have
    /// already given back, and a renderer that answered `0` regardless would
    /// tell them they still hold money they do not.
    #[test]
    fn an_invoices_amount_refunded_is_rendered_from_the_row_and_not_from_a_constant() {
        let mut row = invoice_row();
        row.status = vpay_core::InvoiceStatus::Paid.as_wire_str().to_owned();
        row.amount_paid = 5000;
        row.amount_remaining = 0;
        row.amount_refunded = 2500;

        let rendered = serde_json::to_value(
            InvoiceObject::render(&row, &[invoice_line_row()], None)
                .expect("a paid, part-refunded row renders"),
        )
        .expect("serialises");

        assert_eq!(
            rendered.get("amount_refunded"),
            Some(&json!(2500)),
            "the wire value is the row's, not a constant: {rendered:?}"
        );
        // …and the three amounts beside it did not move, which is the whole
        // of D5 on the wire as well as in the table.
        assert_eq!(rendered.get("amount_paid"), Some(&json!(5000)));
        assert_eq!(rendered.get("amount_remaining"), Some(&json!(0)));
        assert_eq!(rendered.get("status"), Some(&json!("paid")));
    }

    /// A phone-only customer renders `null` for the two absent identifiers
    /// rather than omitting the keys.
    ///
    /// The maintainer's decision of 2026-09-05 is that a phone number alone
    /// is a complete customer, and both SDKs decode `name`/`email` as
    /// nullable. Omitting the keys instead of nulling them would still
    /// deserialise in both — and would silently change the shape of a
    /// `customer.deleted` body for exactly the population vpay exists for.
    #[test]
    fn a_phone_only_customer_renders_the_absent_identifiers_as_null() {
        let mut row = customer_row();
        row.name = None;
        row.email = None;

        let rendered =
            serde_json::to_value(CustomerObject::try_from(&row).expect("a phone-only row renders"))
                .expect("serialises");
        let object = rendered.as_object().expect("an object");

        assert_eq!(object.len(), 9, "still nine keys: {object:?}");
        assert_eq!(object.get("name"), Some(&Value::Null));
        assert_eq!(object.get("email"), Some(&Value::Null));
        assert_eq!(
            object.get("phone").and_then(Value::as_str),
            Some("237600000200")
        );
    }

    /// A customer's debug output redacts every personal identifier —
    /// `name`, `email`, `phone` as character counts, and a present `address`
    /// (formal components **and** the GPS pair, landed the same day as this
    /// test) as a component count through [`AddressObject`]'s own `Debug` —
    /// while leaving every other field visible. The merchant collected this
    /// data and holds it in their database; redacting it in logs would hide
    /// their own information from them. The actual redaction place that
    /// counts is the server, where `CustomerRow`'s Debug impl prevents this
    /// data from reaching vpay's logs at all.
    ///
    /// Every negative assertion below checks a value this test's own fixture
    /// actually carries — not a stand-in like Stripe's `"John Doe"` — because
    /// a check against a value that was never going to be printed passes
    /// whether or not redaction works.
    #[test]
    fn a_customer_object_debug_output_redacts_personal_identifiers() {
        let row = vpay_db::CustomerRow {
            address: vpay_db::CustomerAddress {
                line1: Some("12 Rue de la Paix".to_owned()),
                line2: Some("Apt 4".to_owned()),
                city: Some("Douala".to_owned()),
                state: Some("Littoral".to_owned()),
                postal_code: Some("00237".to_owned()),
                country: Some("CM".to_owned()),
                latitude_microdeg: Some(4_061_000),
                longitude_microdeg: Some(9_702_000),
            },
            ..customer_row()
        };
        let customer = CustomerObject::try_from(&row).expect("a well-formed row renders");

        let formatted = format!("{customer:?}");

        // Redactions are visible but values are hidden.
        assert!(
            formatted.contains("[") && formatted.contains("chars redacted]"),
            "identifier redactions must be visible in debug output: {formatted}"
        );
        assert!(
            formatted.contains("component(s) redacted"),
            "address redaction must be visible in debug output: {formatted}"
        );

        // Specific identifiers are redacted — checked against the values
        // this fixture actually carries (see `customer_row`).
        assert!(
            !formatted.contains("Ada Ngo"),
            "name must not appear in debug output: {formatted}"
        );
        assert!(
            !formatted.contains("ada@example.cm"),
            "email must not appear in debug output: {formatted}"
        );
        assert!(
            !formatted.contains("237600000200"),
            "phone must not appear in debug output: {formatted}"
        );

        // The address — formal components and the GPS pair — is redacted
        // too. A payer's coordinates name where they can be found as
        // precisely as their street does, so this is not optional.
        assert!(
            !formatted.contains("Rue de la Paix"),
            "street address must not appear in debug output: {formatted}"
        );
        assert!(
            !formatted.contains("Douala"),
            "city must not appear in debug output: {formatted}"
        );
        assert!(
            !formatted.contains("4061000") && !formatted.contains("9702000"),
            "GPS coordinates must not appear in debug output: {formatted}"
        );

        // Other fields are still visible and useful.
        assert!(
            formatted.contains("cus_1"),
            "customer id must be visible: {formatted}"
        );
        assert!(
            formatted.contains("CustomerTag"),
            "object type tag must be visible: {formatted}"
        );
    }

    /// The object `docs/flows/merchant-auth.md` documents, key for key.
    ///
    /// Ten keys since issue #46, and the count is the tripwire: an eleventh
    /// key added here reaches `charge.refunded`'s `data.object` — signed,
    /// delivered at-least-once and stored in `events` forever — before
    /// anybody writes it down.
    #[test]
    fn the_refund_object_is_the_documented_ten_keys() {
        let rendered = serde_json::to_value(
            RefundObject::try_from(&refund_row(None)).expect("a well-formed row renders"),
        )
        .expect("serialises");
        let object = rendered.as_object().expect("an object");

        for key in [
            "id",
            "object",
            "amount",
            "currency",
            "payment_intent",
            "status",
            "reason",
            "metadata",
            "created",
            "fee",
        ] {
            assert!(object.contains_key(key), "`{key}` is missing");
        }
        assert_eq!(
            object.len(),
            10,
            "an undocumented key was added: {object:?}"
        );

        assert_eq!(
            rendered,
            json!({
                "id": "re_1",
                "object": "refund",
                "amount": 2_000,
                "currency": "xaf",
                "payment_intent": "pi_1",
                "status": "pending",
                "reason": null,
                "metadata": { "order_id": "1234" },
                "created": 1_753_401_600,
                "fee": null,
            })
        );
    }

    /// **The absent-vs-zero distinction, which is the whole of issue #46.**
    ///
    /// `serde_json::Value` equality above would also pass for a renderer that
    /// emitted `0` in place of `null` if the expectation moved with it, and
    /// `Option<i64>` makes `row.fee.unwrap_or(0)` a one-word edit that
    /// compiles. This case is the one that fails: `null` when the rail said
    /// nothing, `0` only when the rail said the movement was free, and a
    /// reported fee carried through untouched.
    #[test]
    fn an_unreported_refund_fee_renders_null_and_a_reported_zero_renders_zero() {
        let fee_of = |fee| {
            serde_json::to_value(
                RefundObject::try_from(&refund_row(fee)).expect("a well-formed row renders"),
            )
            .expect("serialises")
            .get("fee")
            .cloned()
            .expect("`fee` is always present")
        };

        assert_eq!(
            fee_of(None),
            Value::Null,
            "a rail that reported no fee must render `null`; `0` would tell a merchant the \
             movement was free, which nobody measured"
        );
        assert_eq!(
            fee_of(Some(0)),
            json!(0),
            "a rail that reported the movement was free must render `0`, not `null`"
        );
        assert_eq!(fee_of(Some(250)), json!(250));
    }

    /// **`amount` is the payer's money and is never net of `fee`.**
    ///
    /// `docs/flows/merchant-auth.md` says so, and the issue's first invariant
    /// is "the buyer's refund never nets a fee" — the reason `fee` is a
    /// second field rather than an adjustment to the first. That is a
    /// property of the *renderer*, and until this case existed nothing held
    /// it: `amount: row.amount - row.fee.unwrap_or(0)` passed all 244 of
    /// this crate's tests (measured 2026-09-05), because every other refund
    /// case that asserts an amount uses an unreported fee, and the two that
    /// report one read only the `fee` key back.
    ///
    /// A buyer who is refunded 2,000 and shown 1,750 has been charged for a
    /// movement they did not ask for; the fee is the *merchant's* settlement
    /// line, which is the whole boundary the issue draws.
    #[test]
    fn a_reported_fee_never_moves_the_payers_amount() {
        let amount_of = |fee| {
            serde_json::to_value(
                RefundObject::try_from(&refund_row(fee)).expect("a well-formed row renders"),
            )
            .expect("serialises")
            .get("amount")
            .cloned()
            .expect("`amount` is always present")
        };

        // The row's own amount, whatever the rail charged to move it.
        for fee in [None, Some(0), Some(250), Some(2_000), Some(9_999)] {
            assert_eq!(
                amount_of(fee),
                json!(2_000),
                "a fee of {fee:?} changed the amount the payer gets back"
            );
        }
    }

    /// The contract's point, as for the payment intent: a merchant's own
    /// shipping client decodes what this renders, `fee` included.
    #[test]
    fn the_merchant_sdk_deserialises_the_refund_this_renders() {
        let rendered =
            serde_json::to_string(&RefundObject::try_from(&refund_row(None)).expect("renders"))
                .expect("serialises");
        let decoded: vpay_sdk::Refund =
            serde_json::from_str(&rendered).expect("the SDK decodes the object vpay renders");

        assert_eq!(decoded.id, "re_1");
        assert_eq!(decoded.object, "refund");
        assert_eq!(decoded.amount, 2_000);
        assert_eq!(decoded.currency, "xaf");
        assert_eq!(decoded.payment_intent, "pi_1");
        assert_eq!(decoded.status, vpay_sdk::RefundStatus::Pending);
        assert_eq!(decoded.reason, None);
        assert_eq!(decoded.created, 1_753_401_600);
        assert_eq!(
            decoded.metadata.get("order_id").map(String::as_str),
            Some("1234"),
            "the merchant's own pairs survive the round trip"
        );
        assert_eq!(decoded.fee, None, "an unreported fee decodes as unknown");

        let free =
            serde_json::to_string(&RefundObject::try_from(&refund_row(Some(0))).expect("renders"))
                .expect("serialises");
        let decoded: vpay_sdk::Refund = serde_json::from_str(&free).expect("the SDK decodes it");
        assert_eq!(
            decoded.fee,
            Some(0),
            "the SDK must keep `0` distinct from `None`, or the distinction dies one layer \
             further out than the wire"
        );

        // And a reason the merchant did supply arrives as their own text,
        // not as one of vpay's: the vocabulary is theirs and is not closed.
        let mut row = refund_row(Some(250));
        row.reason = Some("requested_by_customer".to_owned());
        let with_reason = serde_json::to_string(&RefundObject::try_from(&row).expect("renders"))
            .expect("serialises");
        let decoded: vpay_sdk::Refund =
            serde_json::from_str(&with_reason).expect("the SDK decodes it");
        assert_eq!(decoded.reason.as_deref(), Some("requested_by_customer"));
        assert_eq!(decoded.fee, Some(250), "a reported fee arrives unchanged");
    }

    /// A refund `status` the database cannot hold — `refunds_status_enum_check`
    /// since migration `0037`, the `refund_status` enum before it — is a
    /// schema/code disagreement, not a status to invent.
    #[test]
    fn a_refund_status_outside_the_vocabulary_is_internal_rather_than_guessed() {
        let mut row = refund_row(None);
        row.status = "cancelled".to_owned();

        let err = RefundObject::try_from(&row).expect_err("an unmodelled label must not render");
        assert!(
            matches!(err, ApiError::Internal(ref message) if message.contains("refunds.status")),
            "{err:?}"
        );
    }

    /// Every value `refunds.status` can hold — the four labels migration
    /// `0017` created as the `refund_status` enum and migration `0037`
    /// re-closed as `refunds_status_enum_check` — parses into
    /// [`RefundStatus`] **and** decodes in the merchant SDK's own closed
    /// enum.
    ///
    /// This is the case that pays for [`RefundObject::status`] being typed:
    /// the four labels the database can produce are exactly the four this
    /// crate models and exactly the four both SDKs model, so adding a fifth
    /// to the migration without adding it to `vpay_core::RefundStatus` and to
    /// both SDKs fails here rather than in a merchant's client.
    #[test]
    fn every_stored_refund_status_renders_and_decodes_in_the_merchant_sdk() {
        for status in ["pending", "succeeded", "failed", "canceled"] {
            let mut row = refund_row(None);
            row.status = status.to_owned();
            let rendered = serde_json::to_string(
                &RefundObject::try_from(&row)
                    .unwrap_or_else(|error| panic!("`{status}` is a stored label: {error:?}")),
            )
            .expect("serialises");
            let decoded: vpay_sdk::Refund = serde_json::from_str(&rendered)
                .unwrap_or_else(|error| panic!("the SDK decodes `{status}`: {error}"));
            assert_eq!(
                serde_json::to_value(decoded.status).expect("serialises"),
                json!(status)
            );
        }
    }

    /// A `null` `reason` is rendered as `null`, not dropped — and one the
    /// merchant did supply is rendered as their text.
    ///
    /// `Option<String>` with no `skip_serializing_if`, deliberately: the SDKs
    /// declare `reason` present-and-nullable, and an omitted key is a
    /// different wire shape from a null one for anything that counts keys —
    /// including `the_refund_object_is_the_documented_ten_keys`.
    #[test]
    fn an_absent_reason_is_null_rather_than_a_missing_key() {
        let rendered = serde_json::to_value(
            RefundObject::try_from(&refund_row(None)).expect("the fixture row renders"),
        )
        .expect("serialises");
        let object = rendered.as_object().expect("an object");
        assert_eq!(object.get("reason"), Some(&Value::Null));
        assert_eq!(object.len(), 10);

        let mut row = refund_row(None);
        row.reason = Some("duplicate".to_owned());
        let rendered =
            serde_json::to_value(RefundObject::try_from(&row).expect("the fixture row renders"))
                .expect("serialises");
        let object = rendered.as_object().expect("an object");
        assert_eq!(object.get("reason"), Some(&json!("duplicate")));
        assert_eq!(object.len(), 10);
    }

    /// `metadata` that is not a JSON object is this layer's invariant failing
    /// — migration `0017`'s `metadata_is_object` CHECK makes it impossible —
    /// so it is `Internal` (500, paged) and never an empty object quietly
    /// telling the merchant their metadata was lost.
    #[test]
    fn a_refund_whose_metadata_is_not_an_object_is_internal_not_empty() {
        let mut row = refund_row(None);
        row.metadata = json!(["not", "an", "object"]);
        match RefundObject::try_from(&row) {
            Err(ApiError::Internal(message)) => {
                assert!(
                    message.contains("refunds.metadata"),
                    "the operator-facing message names the column: {message}"
                );
            }
            other => panic!("expected an Internal error, got {other:?}"),
        }
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

    /// A refund reaching a merchant through **both** refund event types,
    /// which is the surface that matters most for issue #46: `data.object`
    /// is the wire object, so a webhook body and an API response cannot
    /// disagree about a field, and the webhook is the one a merchant's
    /// settlement job reads.
    ///
    /// The key must be **present and `null`**, not absent: both merchant
    /// SDKs model `fee` as optional so an older vpay still decodes, so an
    /// omitted key would decode without complaint and the merchant would
    /// simply never learn the field exists.
    ///
    /// # The event row here is hand-built, because nothing writes one
    ///
    /// Neither `charge.refunded` nor `charge.refund.updated` has ever been
    /// emitted: both are in the `type_is_a_documented_event` vocabulary
    /// (migrations `0018`/`0029`) and no code path writes either. So this
    /// case seeds `events.data` itself with what the renderer produced,
    /// rather than driving a transition that would produce it. What it
    /// proves is the *contract* — that `data.object` is this object, key for
    /// key — and not that a refund event works.
    ///
    /// Both types are covered because `docs/flows/merchant-auth.md`,
    /// `docs/flows/webhooks.md` and `docs/status.md` all claim the same value
    /// on both, and until this loop existed only `charge.refunded` was ever
    /// rendered.
    #[test]
    fn a_refund_delivered_as_either_refund_event_carries_fee_present_and_null() {
        for (wire_type, known) in [
            ("charge.refunded", vpay_sdk::KnownEventType::ChargeRefunded),
            (
                "charge.refund.updated",
                vpay_sdk::KnownEventType::ChargeRefundUpdated,
            ),
        ] {
            let refund = RefundObject::try_from(&refund_row(None)).expect("renders");
            let mut row = event_row(serde_json::to_value(&refund).expect("serialises"));
            row.event_type = wire_type.to_owned();
            row.object_id = "re_1".to_owned();

            let rendered = serde_json::to_value(EventObject::try_from(&row).expect("renders"))
                .expect("serialises");

            let object = rendered
                .pointer("/data/object")
                .and_then(Value::as_object)
                .expect("data.object is the refund");
            assert_eq!(
                object.get("fee"),
                Some(&Value::Null),
                "`fee` must be present and null in a delivered {wire_type} body, not absent"
            );
            assert_eq!(
                object.len(),
                10,
                "the delivered payload is the same ten keys the object has: {object:?}"
            );

            // And it still decodes as a refund in the SDK a merchant installs
            // — through the event accessor they would actually call.
            let bytes = serde_json::to_vec(&EventObject::try_from(&row).expect("renders"))
                .expect("serialises");
            let event: vpay_sdk::Event =
                serde_json::from_slice(&bytes).expect("the shipping SDK decodes the envelope");
            assert_eq!(
                vpay_sdk::KnownEventType::from_wire(&event.kind),
                Some(known)
            );
            let decoded = event.refund().expect("data.object decodes as a refund");
            assert_eq!(decoded.id, "re_1");
            assert_eq!(decoded.fee, None);
        }
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

    // ------------------------------------------------- checkout sessions

    fn session_row() -> vpay_db::CheckoutSessionRow {
        vpay_db::CheckoutSessionRow {
            id: "cs_0123456789abcdefghjkmnpq".to_owned(),
            seq: 7,
            merchant_id: "acme-cameroon-tenant".to_owned(),
            payment_intent_id: "pi_3MtwBwLkdIwHu7ix28a3tqPa".to_owned(),
            livemode: false,
            ui_mode: "hosted".to_owned(),
            status: "open".to_owned(),
            payment_status: "unpaid".to_owned(),
            success_url: Some("https://shop.example/ok?sid={CHECKOUT_SESSION_ID}".to_owned()),
            cancel_url: Some("https://shop.example/cancel".to_owned()),
            return_url: None,
            customer_id: None,
            publishable_key: "pk_test_acmecameroonsandbox01".to_owned(),
            client_secret_suffix: "neverlogthissessioncredential000".to_owned(),
            return_token: "neverlogthisreturntoken000000000".to_owned(),
            expires_at: time::OffsetDateTime::from_unix_timestamp(1_757_000_000)
                .expect("a valid instant"),
            created_at: time::OffsetDateTime::from_unix_timestamp(1_756_913_600)
                .expect("a valid instant"),
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
        }
    }

    /// The JSON block in `docs/plans/2026-09-04-step9-hosted-checkout.md`'s
    /// "The wire contract", pasted and compared key for key.
    ///
    /// The plan — not this module — is the specification every other lane
    /// built against, so it is what the assertion is written from. Every key
    /// is present including the null ones, for
    /// `every_documented_key_is_present_including_the_null_ones`' reason: a
    /// `Value` comparison alone would also pass for an object that omitted
    /// `return_url`, and both merchant SDKs model these as required members.
    #[test]
    fn a_checkout_session_is_the_wire_contracts_own_json() {
        let row = session_row();
        // The shape `v1::checkout_sessions::hosted_url` mints: the
        // publishable key as a query parameter (the page reads it
        // server-side, where a fragment never arrives) and the credential in
        // the fragment (which never leaves the browser).
        let url = format!(
            "https://checkout.example/c/{}?key={}#{}",
            row.id,
            row.publishable_key,
            vpay_core::ids::client_secret(&row.id, &row.client_secret_suffix)
        );
        let object = CheckoutSessionObject::from_row(&row, Some(url.clone()));
        let rendered = serde_json::to_value(CheckoutSessionWithSecret::new(
            object,
            vpay_core::ids::client_secret(&row.id, &row.client_secret_suffix),
        ))
        .expect("a wire DTO always serialises");

        assert_eq!(
            rendered,
            json!({
                "id": "cs_0123456789abcdefghjkmnpq",
                "object": "checkout.session",
                "livemode": false,
                "payment_intent": "pi_3MtwBwLkdIwHu7ix28a3tqPa",
                "ui_mode": "hosted",
                "status": "open",
                "payment_status": "unpaid",
                "success_url": "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
                "cancel_url": "https://shop.example/cancel",
                "return_url": null,
                "url": url,
                "customer": null,
                "expires_at": 1_757_000_000,
                "created": 1_756_913_600,
                "client_secret": vpay_core::ids::client_secret(&row.id, &row.client_secret_suffix),
            })
        );

        let object = rendered.as_object().expect("an object");
        for key in [
            "id",
            "object",
            "livemode",
            "payment_intent",
            "ui_mode",
            "status",
            "payment_status",
            "success_url",
            "cancel_url",
            "return_url",
            "url",
            // `customer` since 2026-09-06 (S4a); the id, never the object.
            "customer",
            "expires_at",
            "created",
            "client_secret",
        ] {
            assert!(object.contains_key(key), "{key} is missing");
        }
        assert_eq!(
            object.len(),
            15,
            "an undocumented key appeared: {rendered:#}"
        );

        // D5: the placeholder is echoed back exactly as the merchant wrote
        // it. Substituting here would make the object a record of one
        // payer's journey rather than of the merchant's configuration.
        assert_eq!(
            object.get("success_url"),
            Some(&json!("https://shop.example/ok?sid={CHECKOUT_SESSION_ID}"))
        );
    }

    /// The `data.object` of a `checkout.session.expired` event: the fourteen
    /// documented keys, `status` already `expired`, and **no credential of
    /// any kind**.
    ///
    /// This is the one rendering whose output is *stored* (`events.data`,
    /// migration 0018), signed, POSTed to every endpoint the merchant
    /// configured, and replayed on every rung of the retry ladder. So the
    /// assertions that matter are the negative ones, and they are made
    /// against the serialised **string** rather than the parsed object: a
    /// `Value` comparison would pass for a body that carried the credential
    /// under a key this test did not think to look at.
    ///
    /// Decisive: change `expired_snapshot` to pass a `Some(url)` through, or
    /// to render `CheckoutSessionWithSecret`, and this fails.
    #[test]
    fn an_expired_session_snapshot_is_the_fourteen_keys_and_carries_no_credential() {
        let row = session_row();
        let secret = vpay_core::ids::client_secret(&row.id, &row.client_secret_suffix);

        let rendered = serde_json::to_value(CheckoutSessionObject::expired_snapshot(&row))
            .expect("a wire DTO always serialises");
        let object = rendered.as_object().expect("an object");

        for key in [
            "id",
            "object",
            "livemode",
            "payment_intent",
            "ui_mode",
            "status",
            "payment_status",
            "success_url",
            "cancel_url",
            "return_url",
            "url",
            // The `cus_…` and never the expanded customer: this body is
            // stored in `events.data`, signed and delivered at-least-once,
            // and the expanded object is a payer's name, email and phone
            // number.
            "customer",
            "expires_at",
            "created",
        ] {
            assert!(object.contains_key(key), "{key} is missing: {rendered:#}");
        }
        assert_eq!(
            object.len(),
            14,
            "an undocumented key appeared in a webhook body: {rendered:#}"
        );
        assert!(
            !object.contains_key("client_secret"),
            "a delivered event must not carry the session credential: {rendered:#}"
        );

        // The projection, and only the projection: `status` moves, and
        // nothing else does. `payment_status` in particular is untouched by
        // an expiry — the money is a fact about the intent.
        assert_eq!(object.get("status"), Some(&json!("expired")));
        assert_eq!(object.get("payment_status"), Some(&json!("unpaid")));
        assert_eq!(object.get("url"), Some(&Value::Null));
        // `url` is null because it carries a credential, *not* because the
        // session was embedded. A reader must be able to tell.
        assert_eq!(object.get("ui_mode"), Some(&json!("hosted")));
        assert_eq!(
            object.get("payment_intent"),
            Some(&json!("pi_3MtwBwLkdIwHu7ix28a3tqPa")),
            "the id, never the expanded intent — that carries a second credential"
        );
        // The row is not mutated: the caller renders what the transaction is
        // *about to* commit, and then hands the row to nobody else.
        assert_eq!(row.status, "open");

        // Every negative assertion, on the bytes.
        let body = serde_json::to_string(&rendered).expect("serialises");
        assert!(
            !body.contains(&secret),
            "the joined client_secret is in a webhook body: {body}"
        );
        assert!(
            !body.contains(&row.client_secret_suffix),
            "the stored half of it is in a webhook body: {body}"
        );
        assert!(
            !body.contains(&row.return_token),
            "the return token is in a webhook body: {body}"
        );
        assert!(
            !body.contains("neverlog"),
            "not even a prefix of either credential: {body}"
        );
        assert!(
            !body.contains("_secret_"),
            "nothing shaped like a credential at all: {body}"
        );
    }

    /// The **merchant** surface renders `payment_intent` as a bare id; the
    /// two **browser** reads render it expanded, with the intent's own
    /// `client_secret` on one of them and never on the other.
    ///
    /// Three shapes on one field, so the assertion that matters is that they
    /// are distinguishable and that the third is a strict subset of the
    /// second. Swap `Expanded` for `ExpandedWithSecret` in
    /// `browser::checkout_sessions::retrieve_for_return` and the last block
    /// here still passes — it tests the type, not the route; the route-level
    /// proof is
    /// `the_return_read_never_renders_the_intents_client_secret` in
    /// `backends/tests/integration/tests/checkout_sessions.rs`, which drives
    /// the real router.
    #[test]
    fn payment_intent_is_an_id_on_v1_and_the_whole_object_on_the_browser_reads() {
        let row = session_row();
        let intent = sample("pi_3MtwBwLkdIwHu7ix28a3tqPa");
        let secret = "pi_3MtwBwLkdIwHu7ix28a3tqPa_secret_neverlogthisintentcredential00";

        // /v1 — the id, and `ExpandableIntent::id()` agrees with it.
        let merchant = CheckoutSessionObject::from_row(&row, None);
        assert_eq!(merchant.payment_intent.id(), "pi_3MtwBwLkdIwHu7ix28a3tqPa");
        let rendered = serde_json::to_value(&merchant).expect("serialises");
        assert_eq!(
            rendered.get("payment_intent"),
            Some(&json!("pi_3MtwBwLkdIwHu7ix28a3tqPa"))
        );

        // /v1/browser session read — the whole object, plus the credential.
        let with_secret =
            merchant
                .clone()
                .with_expanded_intent(ExpandableIntent::ExpandedWithSecret(Box::new(
                    PaymentIntentWithSecret::new(intent.clone(), secret.to_owned()),
                )));
        assert_eq!(
            with_secret.payment_intent.id(),
            "pi_3MtwBwLkdIwHu7ix28a3tqPa"
        );
        let rendered = serde_json::to_value(&with_secret).expect("serialises");
        let expanded = rendered
            .get("payment_intent")
            .and_then(Value::as_object)
            .expect("`untagged` renders the object with no discriminator");
        assert_eq!(expanded.len(), 14, "the thirteen keys plus client_secret");
        assert_eq!(expanded.get("client_secret"), Some(&json!(secret)));
        // The fields the page cannot paint without.
        assert_eq!(expanded.get("amount"), Some(&json!(5000)));
        assert_eq!(expanded.get("currency"), Some(&json!("xaf")));
        assert_eq!(
            expanded.get("payment_method_types"),
            Some(&json!(["mtn_momo"]))
        );
        assert!(expanded.contains_key("status"));
        assert!(expanded.contains_key("next_action"));
        assert!(expanded.contains_key("last_payment_error"));

        // /v1/browser return read — the same thirteen keys, and no credential.
        let without = merchant.with_expanded_intent(ExpandableIntent::Expanded(Box::new(intent)));
        let rendered = serde_json::to_value(&without).expect("serialises");
        let expanded = rendered
            .get("payment_intent")
            .and_then(Value::as_object)
            .expect("an object");
        assert_eq!(
            expanded.len(),
            13,
            "the thirteen documented keys, and no more"
        );
        assert!(
            !expanded.contains_key("client_secret"),
            "the return read must not render the intent's credential: {rendered:#}"
        );
        assert!(
            !serde_json::to_string(&rendered)
                .expect("serialises")
                .contains(secret),
            "the credential must not appear anywhere in the return read's body"
        );
    }

    /// Both credentials a session carries are redacted in `Debug` — and so is
    /// the `url`, because it carries one of them in its fragment.
    ///
    /// Decisive: delete the `url` line from
    /// `CheckoutSessionWithSecret`'s `Debug` impl and the second assertion
    /// fails. That is the one this test exists for — redacting
    /// `client_secret` while printing a `url` that ends in `#cs_…_secret_…`
    /// would redact nothing at all, and it is exactly what a derived `Debug`
    /// plus a hand-written field would produce.
    #[test]
    fn a_checkout_sessions_debug_output_redacts_the_secret_and_the_url_that_carries_it() {
        let row = session_row();
        let secret = vpay_core::ids::client_secret(&row.id, &row.client_secret_suffix);
        let url = format!(
            "https://checkout.example/c/{}?key={}#{secret}",
            row.id, row.publishable_key
        );
        let rendered = CheckoutSessionWithSecret::new(
            CheckoutSessionObject::from_row(&row, Some(url)),
            secret.clone(),
        );

        let formatted = format!("{rendered:?}");
        assert!(
            !formatted.contains(&secret),
            "Debug output must not contain the client_secret: {formatted}"
        );
        assert!(
            !formatted.contains(&row.client_secret_suffix),
            "…nor the suffix half of it, which is what a leaked `url` would carry: {formatted}"
        );
        assert!(
            !formatted.contains("neverlog"),
            "not even a prefix: {formatted}"
        );
        assert!(
            formatted.contains("redacted"),
            "the redaction must be visible, not a silently dropped field: {formatted}"
        );
        // And the session is still useful to whoever is reading the log.
        assert!(
            formatted.contains("cs_0123456789abcdefghjkmnpq"),
            "{formatted}"
        );
        assert!(
            formatted.contains("pi_3MtwBwLkdIwHu7ix28a3tqPa"),
            "{formatted}"
        );
        assert!(formatted.contains("shop.example/cancel"), "{formatted}");
    }

    /// The object `docs/flows/merchant-auth.md` documents, key for key.
    ///
    /// Fourteen keys since issue #70, and the count is the tripwire: an
    /// undocumented key added here reaches every `checkout.session.*` event
    /// body — signed, delivered at-least-once and stored in `events` forever
    /// — before anybody writes it down. A merchant SDK that sees a key the
    /// server documents reaches a decode failure; one that sees a key the
    /// server doesn't document reads it silently. This asserts the field first.
    #[test]
    fn the_checkout_session_object_is_the_documented_fourteen_keys() {
        let rendered = serde_json::to_value(CheckoutSessionObject::from_row(
            &session_row(),
            Some("https://checkout.example/c/cs_1#secret".to_owned()),
        ))
        .expect("serialises");
        let object = rendered.as_object().expect("an object");

        for key in [
            "id",
            "object",
            "livemode",
            "payment_intent",
            "ui_mode",
            "status",
            "payment_status",
            "success_url",
            "cancel_url",
            "return_url",
            "url",
            "customer",
            "expires_at",
            "created",
        ] {
            assert!(object.contains_key(key), "`{key}` is missing");
        }
        assert_eq!(
            object.len(),
            14,
            "an undocumented key was added: {object:?}"
        );

        assert_eq!(
            rendered,
            json!({
                "id": "cs_0123456789abcdefghjkmnpq",
                "object": "checkout.session",
                "livemode": false,
                "payment_intent": "pi_3MtwBwLkdIwHu7ix28a3tqPa",
                "ui_mode": "hosted",
                "status": "open",
                "payment_status": "unpaid",
                "success_url": "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
                "cancel_url": "https://shop.example/cancel",
                "return_url": null,
                "url": "https://checkout.example/c/cs_1#secret",
                "customer": null,
                "expires_at": 1_757_000_000,
                "created": 1_756_913_600,
            })
        );
    }
}
