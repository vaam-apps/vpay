//! [`ApiError`], the HTTP layer's Tier-2 composite error, and the single
//! place a Stripe-shaped error envelope is rendered.
//!
//! Per [ADR-0011](../../../../docs/adr/0011-error-modelling.md) a boundary
//! *derives* its answer from [`Classify`] instead of deciding it: the status
//! is `category().http_status()`, the `type` is `category().stripe_type()`,
//! the `code` and the message are the error's own. A handler therefore
//! returns `Result<_, ApiError>` and uses `?`; it never picks a status and
//! never formats a merchant-facing sentence.
//!
//! Precisely: [`ApiError::into_response`] below calls
//! `crate::error_envelope_with_param`, and that is the only production call
//! to it. `crate::error_envelope` is a thin three-argument wrapper around
//! the same function, kept because `lib.rs`'s test pins the envelope shape
//! through it; nothing in production calls it, so it is `#[cfg(test)]`. Both
//! are `pub(crate)` — no intra-doc link above for exactly that reason —
//! which is what makes "two handlers answer the same `DbError` differently"
//! *impossible* rather than merely discouraged: a handler outside this crate
//! cannot reach either function, and one inside it would have to add a
//! `pub(crate)` call that review would see.
//!
//! **A composite never re-classifies.** Every `Classify` method delegates
//! wholesale for the wrapped variants — not just `category()`. Forwarding
//! the category alone would silently discard a leaf's deliberate override:
//! `ProviderError::Rejected` overrides `code()` to `charge_declined`,
//! `retry()` to `Retry::NewAttempt` and `severity()` to whatever the
//! [`vpay_core::FailureCode`] deserves (a blocked *partner* account pages),
//! while its category (`Conflict`) defaults to
//! `invalid_state`/`Retry::Never`/`Info`. A category-only delegation would
//! answer a declined charge with the wrong code and log a blocked partner
//! account as one more merchant typo, and `vpay_worker::JobError` (the
//! sibling composite, same shape) would answer the identical error
//! differently — the exact drift the ADR exists to stop.
//!
//! ## What goes to the merchant, and what goes to the log
//!
//! `public_message()` is the *only* thing that reaches a caller. The full
//! `Display` **and** the `source` chain go to the log, because a leaf's
//! `Display` names hosts, tables and library text on purpose (ADR-0011:
//! `Display` is for operators) and none of that may cross the wire. The two
//! are pinned apart by a test that puts a recognisable string inside a
//! `sqlx::Error` and asserts it appears in the log line and not in the body.
//!
//! ## No `request_id` field here, deliberately
//!
//! Correlating a merchant's "I got a 500" with a log line needs a request
//! id, and this module still does not invent one — because it no longer has
//! to. [`crate::router`] mounts `tower-http`'s `SetRequestIdLayer`
//! (`MakeRequestUuid`, honouring a client-supplied `x-request-id`),
//! `TraceLayer` with a `make_span_with` that records the id on the span
//! enclosing every handler, and `PropagateRequestIdLayer`, which copies it
//! onto the response — in that order, so the id exists before the span is
//! built and reaches the caller afterwards. Every event this module emits
//! therefore inherits `request_id` from that span automatically, with no
//! field of its own.
//!
//! Generating a second id *here* would produce an id that appears in the log
//! and in no response header, which is worse than none — which is why the
//! answer is a layer rather than a variant field.
//! `Category::Internal`'s generic message promises the merchant a request id
//! ("Contact support with the request id"); the header
//! `PropagateRequestIdLayer` sets is what that promise now points at.
//! `crate`'s own tests pin all three halves: a generated id is a UUID, a
//! supplied id comes back unchanged, and the id appears in the log line this
//! module writes while serving that request.
//!
//! ## The one response that is not an envelope
//!
//! `/healthz` answers `503` with the bare text `database unreachable`, not
//! an envelope. That is deliberate and stays: it is an *infrastructure*
//! probe, read by a supervisor or an orchestrator, not by an SDK — the
//! Stripe error shape exists so `vpay-sdk` clients can surface `.message`,
//! and nothing polling a health endpoint parses that. It is also the one
//! route whose failure must not depend on this module working. Every other
//! response in this crate goes through [`ApiError`].

use std::fmt;

use axum::Json;
use axum::extract::rejection::{FormRejection, JsonRejection, PathRejection, QueryRejection};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use vpay_core::{Category, Classify, Retry, Severity};
use vpay_ledger::LedgerError;

use crate::error_envelope_with_param;
use crate::resource_auth::AuthRejection;

/// How many leading characters of an `Idempotency-Key` may be echoed back.
///
/// A key is chosen by the merchant and can carry anything they put in it —
/// an order id, a customer reference, occasionally something they would not
/// want in a log or a support ticket. Eight characters is enough to tell two
/// of their own keys apart when debugging and not enough to reconstruct one.
const KEY_HINT_CHARS: usize = 8;

/// The longest `param` this crate will put in an envelope, and the only
/// characters allowed in one.
///
/// Stripe's `param` names a field of the request (`amount`,
/// `payment_method_types[0]`), and an SDK uses it to point at a form input.
/// It is therefore *ours* — a name from the API's own vocabulary — never a
/// value the caller sent. [`ApiError::invalid_param`] takes an
/// `impl Into<String>`, though, so nothing in the type system stops a future
/// handler from passing a header, a path segment or a whole request body
/// through it. This bound is what makes that mistake harmless: anything that
/// is not a plausible field name renders as [`FALLBACK_PARAM`] instead, so
/// the envelope cannot become a reflection channel and stays small.
const PARAM_MAX_CHARS: usize = 64;

/// The advisory an SDK's retry loop reads, and the two values it takes.
///
/// Stripe's own header. stripe-node's `RequestSender._shouldRetry` consults
/// it *above* its status-code rules: `false` stops a retry it would
/// otherwise make, `true` forces one it would otherwise skip. vpay needs
/// both directions, which is the whole reason this exists rather than being
/// left to the status code
/// (`docs/plans/2026-09-03-step5b-stripe-sdk.md` §0 S2):
///
/// - stripe-node retries **409 unconditionally**, so without the header it
///   silently re-POSTs a [`ApiError::Conflict`] twice — a lifecycle refusal
///   ("this intent already has a charge") that no amount of waiting fixes.
/// - stripe-node retries **no 4xx**, so without the header it never retries
///   [`ApiError::IdempotencyKeyInFlight`]'s `400` — the one refusal on this
///   surface that resolves on its own the moment the first attempt lands.
///
/// The value is **derived from [`Classify::retry`], never chosen here**
/// (ADR-0011: a boundary renders a classification, it does not make one).
/// `Retry::AfterBackoff` is `true`; `Retry::Never` and `Retry::NewAttempt`
/// are `false` — a new attempt is a *different* request (a new
/// `PaymentIntent`, per `docs/flows/payment-lifecycle.md`), and telling an
/// SDK to replay the same bytes under the same `Idempotency-Key` would be a
/// lie about what would happen.
///
/// Emitted on every response this impl renders, not only the ones a Stripe
/// SDK is likely to see: a header whose presence depended on the category
/// would be one more thing to keep consistent, and `false` is the honest
/// answer for a `400` that will never succeed.
///
/// # A replay re-emits it, and does not re-decide it
///
/// This impl is still the *only* place the advisory is **decided**, and a
/// replay does not pass through it: `v1::payment_intents::replay` rebuilds a
/// response from what `idempotency_keys` stored. Since migration `0025` that
/// includes the header's own text (`idempotency_keys.response_retry`), which
/// `PostRequest::finish` reads back off the rendered response and `replay`
/// writes out unchanged. So a merchant retrying under a key whose stored
/// answer was a `409` gets that `409` **with** `stripe-should-retry: false`,
/// and stripe-node stops applying its own "retry every 409" rule to a refusal
/// waiting cannot fix.
///
/// The fix deliberately *not* taken was re-deriving the advisory from the
/// stored status at replay time. That is exactly what ADR-0011 forbids and
/// what this header exists to avoid: the stored status is `409` both for
/// "your intent already has a charge" (`Retry::Never`) and for any future
/// `Category` that maps there with a different policy, so a status-only rule
/// would emit the *opposite* of the advice from a second, hand-maintained
/// table. Storing the rendered bytes keeps one decision in one place.
///
/// `v1::payment_intents::tests::a_replayed_response_carries_the_advisory_it_was_stored_with`
/// and the integration suite's
/// `a_replayed_error_carries_the_same_retry_advisory_the_original_did` pin
/// that, end to end, against a real Postgres.
pub(crate) const STRIPE_SHOULD_RETRY_HEADER: HeaderName =
    HeaderName::from_static("stripe-should-retry");
const RETRY_YES: HeaderValue = HeaderValue::from_static("true");
const RETRY_NO: HeaderValue = HeaderValue::from_static("false");

/// What an unusable `param` renders as. Deliberately a real, if unhelpful,
/// field name rather than an empty string or a dropped key: an SDK reading
/// `error.param` gets something well-formed, and "the request" is the honest
/// answer when the name we were handed is not one.
const FALLBACK_PARAM: &str = "request";

/// The longest `message` an envelope carries.
///
/// `InvalidParam`'s message is the *only* public message a call site writes
/// freehand — every other one is a fixed sentence from `vpay-core` or a leaf.
/// A handler that interpolates something caller-supplied into it (a rejected
/// value, a header) would otherwise reflect it back at whatever length it
/// arrived, and axum's default body limit is 2 MB. 200 characters is more
/// than any sentence in `docs/api/README.md` and small enough that the
/// envelope stays an envelope.
const MESSAGE_MAX_CHARS: usize = 200;

/// Everything the HTTP boundary can fail with: the leaves it calls into, plus
/// the failures that belong to the request itself.
///
/// Not `#[non_exhaustive]`: this is workspace-internal, and the SDKs model
/// the *wire* (the envelope below), not this type — ADR-0011. The wrapped
/// variants are wider than what exists today on purpose: `Db` and `Auth` are
/// reachable now, and `Provider`, `Money`, `Currency`, `Ledger` and `Config`
/// become reachable the moment a `/v1` handler exists (Phase 3). Adding them now
/// costs one line each and means the first real handler cannot be tempted to
/// hand-roll an envelope because the composite "does not cover" its error.
/// [`ApiError::CheckoutNotConfigured`]'s message when the *deployment* serves
/// no checkout page.
///
/// Names the configuration key, because that is the whole of what makes this
/// answer actionable — a merchant cannot fix it, and whoever they forward it
/// to can, in one edit. The key name is vpay's own configuration vocabulary
/// and not a value the caller sent, so echoing it reflects nothing.
pub const CHECKOUT_BASE_URL_MISSING: &str = "This vpay deployment does not serve a checkout page: `checkout.public_base_url` is not \
     configured. Confirm the PaymentIntent directly instead.";

/// [`ApiError::CheckoutNotConfigured`]'s message when the deployment *does*
/// serve a checkout page but this **tenant** has no publishable key to put in
/// the payer link.
///
/// A distinct sentence from [`CHECKOUT_BASE_URL_MISSING`] under the same
/// `code`, because the fix is in a different place: one is a deployment-wide
/// block, the other is one line in this merchant's own registration. Naming
/// the merchant is deliberately *not* done — the caller is that merchant and
/// already knows.
pub const PUBLISHABLE_KEY_MISSING: &str = "This account has no publishable key registered, so vpay cannot build a checkout link for \
     it: every URL the checkout page is reached by carries one. Add a `publishable_keys` entry \
     to this merchant's registration.";

/// [`ApiError::CheckoutNotConfigured`]'s message when a confirm discovers an
/// **open checkout session** on a deployment that serves no checkout page.
///
/// The third sentence under the same `code`, and the only one a merchant
/// cannot reach by asking for something this deployment does not do: `create`
/// refuses with [`CHECKOUT_BASE_URL_MISSING`] before a session can exist, so
/// the single way here is an operator deleting `checkout.public_base_url`
/// while sessions are still open.
///
/// It is an error rather than a fallback to the merchant's own
/// `charges.return_url`, and that is the whole point of the constant. A
/// session-driven payer sent to the merchant's URL instead of vpay's return
/// page is forwarded one step too early: the session never reaches
/// `complete`, the merchant's page is asked "did they pay?" by a browser that
/// has no answer, and nothing anywhere reports it
/// (`docs/plans/step9-notes/lane-2.md` §3). Refusing the confirm is loud, and
/// nothing has been submitted to a rail when it fires.
pub const CHECKOUT_SESSION_WITHOUT_CHECKOUT_APP: &str = "A checkout session drives this payment, but `checkout.public_base_url` is not configured, so \
     vpay cannot tell the rail where to send the payer back. Restore it, or expire the session.";

/// The state a Checkout Session was found in when it refused a confirm on
/// the PaymentIntent it drives — the payload of
/// [`ApiError::CheckoutSessionNotOpen`].
///
/// # Why a type and not the row's `status` string
///
/// [`Classify::code`] returns `&'static str`, and the two codes this
/// refusal answers with are chosen by *which* state this is. Carrying the
/// database's `status` text would make that a fallible match on a `String`
/// with an "and otherwise?" arm in the middle of the error renderer; a
/// two-variant enum makes it total, and makes "the session is `open`" a
/// state this error cannot be constructed in at all.
///
/// It deliberately does **not** mirror the column's three labels
/// (`vpay_db`'s `open`/`complete`/`expired`): the third is not a refusal,
/// and a type that could express it would need a call site to decide what an
/// `open` refusal means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedSession {
    /// Past its 24-hour horizon, or expired by the hourly sweep, or expired
    /// by the merchant through `POST /v1/checkout/sessions/{id}/expire`.
    ///
    /// One variant for all three, because they are one fact to whoever reads
    /// the answer: the checkout was abandoned and this intent will not be
    /// paid through it. Splitting them would tell a payer's browser which
    /// *mechanism* ended a session, which nobody can act on.
    Expired,
    /// The settlement transaction finished the session — its intent reached
    /// `succeeded` in the same commit.
    Complete,
}

impl ClosedSession {
    /// The `error.code` an SDK branches on.
    ///
    /// Two codes rather than one code plus a `param`, and the argument is
    /// what `param` means on this API: [`ApiError::param`] renders it only
    /// for [`ApiError::InvalidParam`], whose own docs say it "must be a field
    /// name and never a value". A session's state is not a field of the
    /// request — the request that trips this carries no reference to a
    /// session at all — so putting it in `param` would tell an SDK to point
    /// a merchant's form at a parameter they never sent.
    ///
    /// Neither is the category default (`invalid_state`), for
    /// [`ApiError::IdempotencyKeyInFlight`]'s reason: a merchant must be able
    /// to tell "your payer abandoned this checkout" from "this intent is
    /// already processing", because the first is fixed by offering a new
    /// checkout session and the second by waiting.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Expired => "checkout_session_expired",
            Self::Complete => "checkout_session_complete",
        }
    }

    /// The sentence a merchant reads, naming the session that refused.
    ///
    /// Beside [`Self::code`] rather than inside `public_message`'s match, so
    /// the machine-readable half and the human half of one state are written
    /// together: a third state added here is a compile error in both at once,
    /// and the renderer stays the dispatch table it is everywhere else.
    ///
    /// Two sentences, because the remedies differ. An expired checkout is
    /// fixed by offering the payer a new session; a complete one means the
    /// money is already in, and telling a merchant to "create a new checkout
    /// session" there would invite a second charge attempt on a payment that
    /// succeeded.
    #[must_use]
    pub fn message(self, session_id: &str) -> String {
        let sentence = match self {
            Self::Expired => concat!(
                "for this PaymentIntent has expired, so it can no longer be confirmed. ",
                "Create a new checkout session for this intent to offer the payer another link.",
            ),
            Self::Complete => {
                "for this PaymentIntent is already complete, so it cannot be confirmed again."
            }
        };
        format!("The checkout session {session_id} {sentence}")
    }
}

impl fmt::Display for ClosedSession {
    /// The **stored** label, so an operator reading a log line can match it
    /// against `checkout_sessions.status` without a translation table.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Expired => "expired",
            Self::Complete => "complete",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Postgres failed under a request. `#[error(transparent)]` throughout
    /// the wrapped variants: the composite adds no words of its own, so the
    /// log shows the leaf's `Display` verbatim and `source()` continues
    /// straight into the leaf's own source (the `sqlx::Error`) with nothing
    /// duplicated between the two.
    #[error(transparent)]
    Db(#[from] vpay_db::DbError),

    /// A payment rail failed, declined, or is not built yet. Includes
    /// `ProviderError::Rejected`, which is a rail *decision* rather than a
    /// system failure (`docs/flows/errors.md`) — it keeps its own code and
    /// `Retry::NewAttempt` through the delegation below.
    #[error(transparent)]
    Provider(#[from] vpay_provider::ProviderError),

    /// An amount was negative, mixed currencies, or overflowed. The first
    /// two are the caller's; the third is ours and pages.
    #[error(transparent)]
    Money(#[from] vpay_core::MoneyError),

    /// A currency code the system does not know.
    #[error(transparent)]
    Currency(#[from] vpay_core::money::UnknownCurrency),

    /// A ledger transaction this layer built did not balance.
    ///
    /// `Internal`, and it pages: no caller builds a ledger transaction, so
    /// an unbalanced one is this code's own invariant failing in the money
    /// path. Carried here rather than left to a future handler to convert by
    /// hand — a composite that does not cover a leaf its layer can meet is
    /// exactly the invitation to hand-roll an envelope that ADR-0011 exists
    /// to remove. `LedgerError::Money(..)` delegates onward to `MoneyError`,
    /// so a caller's bad amount reaching the ledger still answers 400 rather
    /// than paging.
    #[error(transparent)]
    Ledger(#[from] LedgerError),

    /// The deployment is misconfigured for what this request needs — a rail
    /// with no host, a client with no keys. Reachable from a request path
    /// because per-merchant, per-rail configuration is resolved when the
    /// request is served, not only at boot: `500`, never retried, and an
    /// operator's problem rather than the merchant's.
    #[error(transparent)]
    Config(#[from] vpay_config::ConfigError),

    /// The bearer token was missing, malformed, or did not validate. The
    /// conversion is what lets `AuthRejection`'s `IntoResponse` route
    /// through this type, so there is exactly one envelope renderer.
    #[error(transparent)]
    Auth(#[from] AuthRejection),

    /// No route matched. The router's fallback, and the one variant a
    /// merchant sees today.
    #[error("no route matches {method} {path}")]
    UnknownRoute {
        /// The request method, for the log only — see [`Self::public_message`].
        method: String,
        /// The request path, for the log only.
        path: String,
    },

    /// A request parameter was present but unusable in a way no leaf error
    /// covers: a field of the right type with a value the endpoint refuses.
    /// The parameter *name* is surfaced in the envelope's `param` field,
    /// which is what Stripe SDKs read to point at a form field.
    #[error("invalid request parameter `{param}`: {message}")]
    InvalidParam {
        /// The offending parameter, as named in the request body — echoed
        /// to the caller, so it must be a field name and never a value.
        param: String,
        /// What is wrong with it, written for the merchant: this variant's
        /// message *is* the public message.
        message: String,
    },

    /// An `Idempotency-Key` was replayed with a different request body.
    ///
    /// Carries only a hint, never the key: build it with
    /// [`ApiError::idempotency_key_reused`], which truncates. The public
    /// message truncates again on the way out, so even a hand-constructed
    /// variant cannot echo a whole key.
    #[error("Idempotency-Key {key_hint} was reused with a different request body")]
    IdempotencyKeyReused {
        /// The first `KEY_HINT_CHARS` characters of the key, plus an
        /// ellipsis.
        key_hint: String,
    },

    /// A request under this `Idempotency-Key` has not finished yet, so this
    /// one cannot proceed and there is nothing stored to replay.
    ///
    /// **Its own variant, not [`Self::Conflict`].** Both are things a
    /// merchant retries, and that is where the resemblance ends: a
    /// `Conflict` is about the *object* ("this intent already has a charge"),
    /// is permanent until the merchant does something different, and renders
    /// as `invalid_request_error`/`invalid_state`. This is about the
    /// *request*, resolves on its own the moment the first attempt lands, and
    /// tells the caller to do nothing but wait. Rendered as a `Conflict` — as
    /// it was until this variant existed — an SDK reading `code` could not
    /// tell "your intent moved on without you" from "your own earlier call is
    /// still running", and the only difference on the wire was a sentence.
    ///
    /// **On the status code.** [`Category::Idempotency`]'s policy row says
    /// `400`/`idempotency_error` (`vpay-core`'s `http_status`), so that is
    /// what this answers — ADR-0011 derives the status from the category and
    /// never lets a variant pick one. Stripe answers `409` for this case and
    /// `400` for a replayed key with a different body, i.e. it splits a
    /// status across one `type`, which this policy table cannot express.
    /// Moving to `409` is therefore an ADR-level change (a new `Category`, or
    /// `Category::Idempotency` becoming `409` — which would also move
    /// [`Self::IdempotencyKeyReused`], pinned at `400` by
    /// `a_reused_key_with_a_different_body_is_the_400_envelope`), and is left
    /// as a maintainer decision rather than taken here. The `code` is what an
    /// SDK should branch on either way.
    ///
    /// Carries a hint of the key for the **log only** — the public message is
    /// a fixed sentence, so a merchant's key cannot reach a response body
    /// through this variant at all. Build it with
    /// [`ApiError::idempotency_key_in_flight`], which truncates.
    #[error("a request under Idempotency-Key {key_hint} is still in flight")]
    IdempotencyKeyInFlight {
        /// The first `KEY_HINT_CHARS` characters of the key, plus an
        /// ellipsis. Operator-facing: it is what correlates this refusal with
        /// the log line of the request that is still holding the key.
        key_hint: String,
    },

    /// No such object — or none this client may see. **Both**: a merchant
    /// asking for another merchant's `pi_…` gets this, byte for byte
    /// identical to asking for one that never existed, so the API cannot be
    /// used to discover which ids exist under some other tenant. That is why
    /// [`Self::Forbidden`] is *not* what a foreign id answers.
    ///
    /// `resource` is a `&'static str` on purpose: it names one of our own
    /// object types (`payment_intent`, `refund`), it is rendered into the
    /// public message, and a `String` there would be one refactor away from
    /// being something the caller sent.
    #[error("no such {resource}: {id}")]
    NotFound {
        /// The object type, in the API's own vocabulary — `payment_intent`,
        /// not `payment_intents` and not a table name.
        resource: &'static str,
        /// The id as the caller spelled it. Echoed back (Stripe does the
        /// same, and a merchant grepping their logs for it needs it) but
        /// bounded on the render path like every other reflected value.
        id: String,
    },

    /// The object exists and the request is well-formed, but the object's
    /// current state does not allow it: confirming an intent that is already
    /// `processing`, cancelling one the rail already has, a second charge on
    /// an intent that has one.
    ///
    /// `409`, and the message is written at the call site because *which*
    /// state and *which* action is the only useful thing to say. It is a
    /// public sentence, so it must name our own vocabulary and never echo a
    /// value the caller sent.
    ///
    /// Reaching this is not permission to skip the compare-and-swap: the
    /// state a handler read may already be stale by the time it writes, so
    /// the `UPDATE ... WHERE status = $expected` is what actually enforces
    /// the lifecycle. This variant is how *that* update's zero-rows answer
    /// reaches a merchant.
    #[error("conflict: {message}")]
    Conflict {
        /// What is wrong, written for the merchant: this variant's message
        /// *is* the public message.
        message: String,
    },

    /// The client authenticated, and is not allowed to do this.
    ///
    /// Deliberately rare: object-level tenancy answers [`Self::NotFound`]
    /// instead (see there). This is for a *scope* the token does not carry —
    /// a decision about the credential rather than about an object, where
    /// telling the caller plainly is right because they can see their own
    /// scopes.
    #[error("the client is not permitted to perform that action")]
    Forbidden,

    /// A merchant asked vpay to create a Checkout Session on a deployment
    /// that serves no checkout page — `checkout.public_base_url` is absent
    /// (Step 9).
    ///
    /// **Its own variant, not [`Self::Config`] and not [`Self::Conflict`].**
    /// `Config` wraps `vpay_config::ConfigError`, and there is no config
    /// error here: the file is *valid*, it simply describes a deployment
    /// without the optional block. `Conflict` is about an object's state, and
    /// no object is involved. What a merchant needs from this answer is a
    /// code they can branch on — "this deployment cannot do hosted checkout"
    /// is a permanent, actionable fact about the vpay they are talking to,
    /// distinguishable from "your intent is wrong" and from "vpay is down".
    ///
    /// # On the status code, and where this deviates from the plan
    ///
    /// `docs/plans/2026-09-04-step9-hosted-checkout.md`'s lane 1 brief asks
    /// for `503 checkout_not_configured`. The `code` is exactly that; the
    /// **status is 500**, and that is a deliberate departure rather than an
    /// oversight.
    ///
    /// ADR-0011 derives the status from the [`Category`], never from a call
    /// site, and only [`Category::Storage`] answers `503`. Classifying this
    /// as storage would be wrong twice over: it would tell an operator
    /// Postgres was unreachable, and — the part that actually costs
    /// something — `Category::Storage`'s `Retry::AfterBackoff` would tell a
    /// merchant's SDK to retry a request that cannot succeed until someone
    /// deploys a configuration change. [`Category::Configuration`] says
    /// exactly what is true ("the deployment is misconfigured for this
    /// operation … fixed by a deploy, never by retrying"), and its status is
    /// `500`.
    ///
    /// Making this a `503` honestly would mean either a new `Category` or
    /// moving `Category::Configuration` to `503`, and both are ADR-level
    /// changes affecting every error in the workspace. That is a maintainer's
    /// decision and is recorded in `docs/plans/step9-notes/lane-1.md` rather
    /// than taken here.
    ///
    /// # Three gaps, one code
    ///
    /// A missing `checkout.public_base_url` and a tenant with no
    /// `publishable_keys` are the same *fact* from a merchant's side — "this
    /// vpay cannot do hosted checkout for me" — and the same fix shape, an
    /// operator editing YAML and deploying. So they share the `code` an SDK
    /// branches on and differ only in the sentence, which is what tells
    /// whoever the merchant contacts *which* key to add. The message is a
    /// `&'static str` chosen from the three constants beside this type, never
    /// caller text: it is rendered to the caller, and a `String` here would
    /// be one refactor away from echoing something they sent.
    ///
    /// The third, [`CHECKOUT_SESSION_WITHOUT_CHECKOUT_APP`], is the same fact
    /// discovered on a **confirm** rather than on a create (Step 9, lane 1b):
    /// an open session drives the charge and there is no base URL to build
    /// its return page from. Same code, same fix, same deploy — and the same
    /// argument for refusing rather than falling back, which that constant
    /// makes in full.
    #[error("checkout is not configured on this deployment: {0}")]
    CheckoutNotConfigured(&'static str),

    /// A confirm was refused because the Checkout Session driving the
    /// PaymentIntent is no longer `open`.
    ///
    /// **Its own variant, not [`Self::Conflict`].** Both are 409s about an
    /// object's state and both come out of the same category; the difference
    /// is the `code`, and the `code` is the entire machine-readable content
    /// of this answer. `invalid_state` is what a confirm on an intent that is
    /// already `processing` answers, and a merchant handed that string cannot
    /// tell "your own retry raced you" from "your payer walked away and we
    /// told you so an hour ago" — which are fixed by waiting and by offering
    /// a new checkout session respectively. See [`ClosedSession::code`] for
    /// why there are two codes here and not one code with a `param`.
    ///
    /// # Why it exists at all
    ///
    /// A session's `status` is a promise vpay has already made to the
    /// merchant: the hourly sweep emits `checkout.session.expired`, and
    /// `POST /v1/checkout/sessions/{id}/expire` is the merchant's own
    /// statement that the checkout is over. Neither of those retracts the
    /// payer's credential — the intent's `client_secret` is minted at create
    /// and lives as long as the intent (`docs/flows/browser-checkout.md`) —
    /// so without this refusal a payer holding a stale checkout link could
    /// still pay. The charge would then succeed while
    /// `settle_for_intent`'s own `WHERE status = 'open'` guard correctly
    /// declined to touch the session, leaving `expired`/`unpaid` under a
    /// `succeeded` intent and a merchant who was told the opposite.
    ///
    /// # It carries the session id, and that is safe on both surfaces
    ///
    /// Unlike [`Self::NotFound`]'s `id`, this one is **not** caller text: it
    /// is read off the row vpay found, so it reflects nothing. On `/v1` the
    /// caller is the merchant that owns the session; on `/v1/browser` the
    /// caller has already proved it holds the intent's `client_secret`,
    /// which is a stronger credential than a `cs_…` id. It is still rendered
    /// through `bounded_message`, like every other value on this path.
    #[error(
        "checkout session {session_id} is `{state}`, so its payment intent cannot be confirmed"
    )]
    CheckoutSessionNotOpen {
        /// The `cs_…` that refuses the confirm, so a merchant reading the
        /// message can go straight to `GET /v1/checkout/sessions/{id}` and
        /// to the `checkout.session.expired` event they were already sent.
        session_id: String,
        /// Which of the two closed states it is in — the thing that picks the
        /// `code`.
        state: ClosedSession,
    },

    /// An invariant this layer guarantees was violated — the "should be
    /// impossible" arm. `String` rather than a wrapped error because there
    /// is no error type to wrap: it is reached when the code discovers a
    /// state it constructed and believed impossible. Pages, and the payload
    /// is never shown to a caller.
    #[error("internal error: {0}")]
    Internal(String),
}

impl ApiError {
    /// Builds an [`ApiError::IdempotencyKeyReused`] from the raw key,
    /// truncating it to a hint on the way in.
    ///
    /// Takes the raw key so no call site has to remember to truncate — the
    /// obvious way to build the variant is the safe one. See
    /// this module's `KEY_HINT_CHARS` for why the key is not echoed whole.
    #[must_use]
    pub fn idempotency_key_reused(key: &str) -> Self {
        Self::IdempotencyKeyReused {
            key_hint: key_hint(key),
        }
    }

    /// Builds an [`ApiError::IdempotencyKeyInFlight`] from the raw key,
    /// truncating it to a hint on the way in.
    ///
    /// Same shape and same reason as [`Self::idempotency_key_reused`]: the
    /// obvious way to build the variant is the one that cannot put a whole
    /// merchant-chosen key into a log line.
    #[must_use]
    pub fn idempotency_key_in_flight(key: &str) -> Self {
        Self::IdempotencyKeyInFlight {
            key_hint: key_hint(key),
        }
    }

    /// Builds an [`ApiError::InvalidParam`]. `param` is a field name and
    /// `message` is shown to the merchant verbatim, so neither may carry a
    /// value the caller sent us for anything else.
    ///
    /// Both are bounded on the way *out* rather than here (see this module's
    /// `PARAM_MAX_CHARS` and `MESSAGE_MAX_CHARS`): the render path is the
    /// last place a hand-built variant can still be caught, and a
    /// constructor-only bound would be a bound a `Self::InvalidParam { .. }`
    /// literal skips.
    #[must_use]
    pub fn invalid_param(param: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidParam {
            param: param.into(),
            message: message.into(),
        }
    }

    /// Classifies a [`serde_json::Error`] raised while **we** were producing
    /// JSON — serialising a response body, re-encoding a stored payload — as
    /// the internal error it is.
    ///
    /// Spelled out at the call site instead of being a
    /// `impl From<serde_json::Error> for ApiError`, and that is the whole
    /// point. One type means two opposite things here: deserialising a
    /// request body, where the caller sent bad JSON and the answer is 400;
    /// and serialising something this crate built, where the answer is 500
    /// and someone should look. A blanket `From` has to pick one, and `?`
    /// would then silently apply it to both — answering 500 for a merchant's
    /// malformed body, or (worse, because it hides a bug) 400 for our own
    /// broken response. The caller-input direction already has its own typed
    /// conversions ([`JsonRejection`] below), so every remaining bare
    /// `serde_json::Error` in this crate is ours, and naming it is cheap.
    ///
    /// The library's text goes into the `Internal` payload, which is logged
    /// and never rendered — `Internal`'s public message is the category's
    /// generic sentence.
    #[must_use]
    pub fn internal_serialization(error: serde_json::Error) -> Self {
        Self::Internal(format!("serialising a response body: {error}"))
    }

    /// The envelope's `param` field, if this error is about one named
    /// parameter. `None` for everything else, and the field is then absent
    /// from the body rather than present and `null` — Stripe omits it, and
    /// an SDK that checks `"param" in error` should see the same thing.
    #[must_use]
    pub fn param(&self) -> Option<&str> {
        match self {
            // Bounded here, on the render path, rather than in the
            // constructor: this is the last point at which a variant built
            // as a struct literal can still be caught. See `bounded_param`.
            Self::InvalidParam { param, .. } => Some(bounded_param(param)),
            _ => None,
        }
    }

    /// The `source` chain rendered for a log line, outermost cause first.
    ///
    /// Walked explicitly because `Display` only ever renders one level: a
    /// `DbError`'s text names Postgres, but the `sqlx::Error` underneath it
    /// is where the actual reason lives (a DNS failure, a refused
    /// connection, a constraint name). Logging the `Display` alone would
    /// throw that away, and it is the half an operator needs at 3am. Empty
    /// when the error has no source.
    #[must_use]
    pub fn source_chain(&self) -> String {
        vpay_core::error::source_chain(self)
    }

    /// Emits the operator-facing half of this error, at the level its
    /// [`Severity`] maps to.
    ///
    /// `tracing` has four levels and [`Severity`] has four values, but
    /// `Error` and `Page` both map to `ERROR` — so a `Page` additionally
    /// carries `alert = true`, which is what an alerting rule selects on. A
    /// level alone could not express the difference, and losing it would
    /// mean either paging on every `DbError` or never paging at all.
    ///
    /// It is also where `vpay_error_events_total` and — for a `Page` —
    /// `vpay_alert_events_total` are incremented, through
    /// [`vpay_core::metrics::record_error_event`]. In this function rather
    /// than in a `tracing` layer that scraped the events, so that the
    /// counter and the `alert = true` field are the same decision read from
    /// the same [`Classify`] impl: a layer would be a second classification
    /// that could disagree with this one, and the disagreement would be
    /// invisible until a page failed to fire.
    fn log(&self) {
        let category = self.category();
        let chain = self.source_chain();
        vpay_core::metrics::record_error_event(self);
        match self.severity() {
            Severity::Info => tracing::info!(
                category = ?category,
                code = self.code(),
                error = %self,
                source_chain = %chain,
                "api error"
            ),
            Severity::Warn => tracing::warn!(
                category = ?category,
                code = self.code(),
                error = %self,
                source_chain = %chain,
                "api error"
            ),
            Severity::Error => tracing::error!(
                category = ?category,
                code = self.code(),
                error = %self,
                source_chain = %chain,
                "api error"
            ),
            Severity::Page => tracing::error!(
                alert = true,
                category = ?category,
                code = self.code(),
                error = %self,
                source_chain = %chain,
                "api error"
            ),
        }
    }
}

/// The first [`KEY_HINT_CHARS`] characters of `key`, plus an ellipsis.
///
/// Character-wise, not byte-wise: a key is merchant-supplied text and
/// slicing it at byte 8 would panic on a multi-byte boundary — ADR-0007
/// denies panics in production code, and this runs on a request path.
/// Idempotent, so applying it to an already-truncated hint is a no-op: that
/// is what lets [`ApiError::public_message`] truncate defensively without
/// producing `abcdefgh……`.
fn key_hint(key: &str) -> String {
    let mut hint: String = key.chars().take(KEY_HINT_CHARS).collect();
    hint.push('…');
    hint
}

/// `param` if it is a plausible field name, [`FALLBACK_PARAM`] otherwise.
///
/// The shape a name must have: one to [`PARAM_MAX_CHARS`] characters drawn
/// from `a-z`, `0-9`, `_`, `.`, `[` and `]` — enough for every name
/// `docs/api/README.md` uses, including Stripe's nested form spelling
/// (`payment_method_types[0]`, `metadata.order_id`), and not enough for a
/// sentence, a URL, a header value or a JSON document. Rejecting rather than
/// truncating is deliberate: a truncated wrong name still points an SDK at a
/// field that does not exist, whereas `request` is at least true.
fn bounded_param(param: &str) -> &str {
    let shaped = !param.is_empty()
        && param.chars().count() <= PARAM_MAX_CHARS
        && param.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '[' | ']')
        });
    if shaped { param } else { FALLBACK_PARAM }
}

/// The first [`MESSAGE_MAX_CHARS`] characters of `message`, with an ellipsis
/// iff anything was dropped.
///
/// Character-wise for the same reason as [`key_hint`]: the text may be
/// merchant-derived and slicing at byte 200 would panic on a multi-byte
/// boundary, which ADR-0007 denies on a request path.
fn bounded_message(message: &str) -> String {
    let mut out: String = message.chars().take(MESSAGE_MAX_CHARS).collect();
    if message.chars().nth(MESSAGE_MAX_CHARS).is_some() {
        out.push('…');
    }
    out
}

impl Classify for ApiError {
    fn category(&self) -> Category {
        match self {
            Self::Db(e) => e.category(),
            Self::Provider(e) => e.category(),
            Self::Money(e) => e.category(),
            Self::Currency(e) => e.category(),
            Self::Ledger(e) => e.category(),
            Self::Config(e) => e.category(),
            Self::Auth(e) => e.category(),
            // 404, and `invalid_request_error` on the wire — the same shape
            // a missing object gets, because to a caller "no such URL" and
            // "no such object" are the same class of mistake.
            Self::UnknownRoute { .. } => Category::NotFound,
            Self::InvalidParam { .. } => Category::InvalidRequest,
            Self::IdempotencyKeyReused { .. } | Self::IdempotencyKeyInFlight { .. } => {
                Category::Idempotency
            }
            Self::NotFound { .. } => Category::NotFound,
            Self::Conflict { .. } => Category::Conflict,
            Self::Forbidden => Category::Forbidden,
            // An operator's problem, fixed by a deploy, never by retrying —
            // which is `Category::Configuration`'s own definition. See the
            // variant for why it is not `Storage` even though the plan asked
            // for that category's status.
            Self::CheckoutNotConfigured(_) => Category::Configuration,
            // The object exists, the request is well-formed, and the object's
            // state forbids it — `Category::Conflict`'s own definition, and
            // the same category `Self::Conflict` carries. Only the `code`
            // below differs, which is exactly the shape
            // `IdempotencyKeyInFlight` established.
            Self::CheckoutSessionNotOpen { .. } => Category::Conflict,
            // The only variant that pages. If this is ever logged, something
            // this layer promised was true was not.
            Self::Internal(_) => Category::Internal,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Db(e) => e.code(),
            Self::Provider(e) => e.code(),
            Self::Money(e) => e.code(),
            Self::Currency(e) => e.code(),
            Self::Ledger(e) => e.code(),
            Self::Config(e) => e.code(),
            Self::Auth(e) => e.code(),
            // Overrides `NotFound`'s `resource_missing`: an unrecognised URL
            // is not a missing object, and an SDK branching on the code
            // should be able to tell "you called an endpoint vpay does not
            // implement" from "that payment intent does not exist". This is
            // the code the 404 has always carried.
            Self::UnknownRoute { .. } => "unknown_route",
            // Deliberately the category default rather than a per-parameter
            // code: the machine-readable part of "which parameter" is the
            // envelope's `param` field, which is where Stripe SDKs look. A
            // code per parameter would be an open-ended vocabulary.
            Self::InvalidParam { .. } => Category::InvalidRequest.default_code(),
            Self::IdempotencyKeyReused { .. } => Category::Idempotency.default_code(),
            // The one *other* deliberate override in this enum, and the
            // whole reason the variant exists: `idempotency_key_in_use` (the
            // category default, above) means "you changed the body", which
            // is a merchant bug they must fix. This means "your own earlier
            // request has not finished", which they fix by waiting. Same
            // status and same `type`; only the code tells them apart, so an
            // SDK's retry logic branches on this string.
            Self::IdempotencyKeyInFlight { .. } => "idempotency_key_in_flight",
            // The category defaults, spelled through the category rather than
            // as literals: `resource_missing`, `invalid_state` and
            // `forbidden` are Stripe's own codes and `vpay-core` owns them
            // (docs/flows/errors.md's policy table). `UnknownRoute` above is
            // the one deliberate override in this enum, and it says why.
            Self::NotFound { .. } => Category::NotFound.default_code(),
            Self::Conflict { .. } => Category::Conflict.default_code(),
            Self::Forbidden => Category::Forbidden.default_code(),
            // The third deliberate override in this enum. The category
            // default, `misconfigured`, is what *every* configuration failure
            // says, and a merchant integrating hosted checkout has to be able
            // to tell "this deployment has no checkout page" from "vpay's YAML
            // is broken in some other way" — the first is a permanent
            // capability answer they design around, the second is an outage.
            Self::CheckoutNotConfigured(_) => "checkout_not_configured",
            // The fourth deliberate override, and the one that is two codes
            // rather than one: `ClosedSession` owns the choice so that this
            // match stays total and a third session state cannot be added
            // without deciding what it answers. See that type for why the
            // distinction is a `code` and not a `param`.
            Self::CheckoutSessionNotOpen { state, .. } => state.code(),
            Self::Internal(_) => Category::Internal.default_code(),
        }
    }

    fn retry(&self) -> Retry {
        match self {
            Self::Db(e) => e.retry(),
            Self::Provider(e) => e.retry(),
            Self::Money(e) => e.retry(),
            Self::Currency(e) => e.retry(),
            Self::Ledger(e) => e.retry(),
            Self::Config(e) => e.retry(),
            Self::Auth(e) => e.retry(),
            // No overrides: none of this layer's own failures heals on its
            // own, and every one of these categories already defaults to
            // `Retry::Never`.
            // The one override this layer makes, and the only variant here
            // that heals without anyone doing anything: the first request
            // finishing is what clears it, so the honest instruction is
            // "the same call, shortly" rather than `Category::Idempotency`'s
            // default `Retry::Never` (which is right for its sibling — a key
            // reused with a different body never becomes valid).
            Self::IdempotencyKeyInFlight { .. } => Retry::AfterBackoff,
            Self::UnknownRoute { .. }
            | Self::InvalidParam { .. }
            | Self::IdempotencyKeyReused { .. }
            | Self::NotFound { .. }
            | Self::Conflict { .. }
            | Self::Forbidden
            | Self::CheckoutNotConfigured(_)
            | Self::CheckoutSessionNotOpen { .. }
            | Self::Internal(_) => self.category().default_retry(),
        }
    }

    fn severity(&self) -> Severity {
        match self {
            Self::Db(e) => e.severity(),
            Self::Provider(e) => e.severity(),
            Self::Money(e) => e.severity(),
            Self::Currency(e) => e.severity(),
            Self::Ledger(e) => e.severity(),
            Self::Config(e) => e.severity(),
            Self::Auth(e) => e.severity(),
            // No overrides. A 404 and a bad parameter are `Info` because a
            // payment gateway serves thousands a day and none is worth
            // investigating; `Internal` pages.
            Self::UnknownRoute { .. }
            | Self::InvalidParam { .. }
            | Self::IdempotencyKeyReused { .. }
            | Self::IdempotencyKeyInFlight { .. }
            | Self::NotFound { .. }
            | Self::Conflict { .. }
            | Self::Forbidden
            | Self::CheckoutNotConfigured(_)
            | Self::CheckoutSessionNotOpen { .. }
            | Self::Internal(_) => self.category().default_severity(),
        }
    }

    fn public_message(&self) -> String {
        match self {
            Self::Db(e) => e.public_message(),
            Self::Provider(e) => e.public_message(),
            Self::Money(e) => e.public_message(),
            Self::Currency(e) => e.public_message(),
            Self::Ledger(e) => e.public_message(),
            Self::Config(e) => e.public_message(),
            Self::Auth(e) => e.public_message(),
            // Deliberately does *not* echo the method or path back. The
            // sentence is what this endpoint has always answered and what
            // `docs/api/README.md` documents; reflecting an
            // attacker-controlled URL into a response body buys nothing a
            // caller does not already know and is a reflection sink.
            Self::UnknownRoute { .. } => {
                "Unrecognized request URL. vpay implements a subset of the Stripe API; see docs/api."
                    .to_owned()
            }
            // The whole point of the variant: our own words about the
            // caller's own field — bounded, because this is the one public
            // message a call site writes freehand and the render path is the
            // last place an over-long one can be stopped.
            Self::InvalidParam { message, .. } => bounded_message(message),
            // Truncated a second time on the way out. The constructor
            // already did it, but a variant built by hand would otherwise
            // put a merchant's whole key in a response body, and the render
            // path is the last place that can still be prevented.
            Self::IdempotencyKeyReused { key_hint: hint } => format!(
                "The Idempotency-Key beginning {} was already used with a different request body.",
                key_hint(hint)
            ),
            // A fixed sentence, and deliberately not the key: unlike its
            // sibling above, nothing here needs to identify *which* key —
            // the caller sent it and has exactly one request in flight under
            // it. `key_hint` stays in the `Display` for the log. The
            // semicolon is load-bearing prose: "retry shortly" is the entire
            // instruction, and a merchant who reads `Category::Conflict`'s
            // "the object is in a state that does not allow this action"
            // (what this answered before it had its own variant) goes and
            // looks at their intent instead.
            Self::IdempotencyKeyInFlight { .. } => {
                "A request with this Idempotency-Key is still in progress; retry shortly."
                    .to_owned()
            }
            // Stripe's own sentence, and the id the caller asked for — the
            // one thing that makes a 404 actionable when a merchant is
            // holding two ids and does not know which one is stale. Bounded
            // like every other reflected value: the id comes from a URL path
            // segment, so it is caller-controlled text.
            Self::NotFound { resource, id } => bounded_message(&format!("No such {resource}: {id}")),
            // The whole point of the variant, exactly as `InvalidParam`:
            // our own words about the object's own state.
            Self::Conflict { message } => bounded_message(message),
            // Nothing about *why*: the category's sentence is all a client
            // can act on, and enumerating the scope it lacks would describe
            // the authorisation model to something that failed it.
            Self::Forbidden => Category::Forbidden.generic_message().to_owned(),
            // Its own sentence rather than `Category::Configuration`'s
            // generic "vpay is misconfigured for this operation. Contact
            // support." — which is true and useless here. This is not an
            // outage a merchant should open a ticket about; it is a
            // capability this deployment does not have, and naming the key
            // is what lets whoever they *do* contact fix it in one edit. The
            // key name is vpay's own configuration vocabulary, not a value
            // the caller sent, so echoing it reflects nothing.
            Self::CheckoutNotConfigured(reason) => (*reason).to_owned(),
            // The sentence lives on `ClosedSession`, beside the `code` the
            // same state picks. It names the session, which is not caller
            // text (see the variant), and goes through `bounded_message`
            // here like every other rendered value.
            Self::CheckoutSessionNotOpen { session_id, state } => {
                bounded_message(&state.message(session_id))
            }
            // Never the payload. `Internal(..)` is reached when an invariant
            // broke, and the text describing it is about our internals by
            // definition.
            Self::Internal(_) => Category::Internal.generic_message().to_owned(),
        }
    }
}

/// Converts one of axum's extractor rejections into an
/// [`ApiError::InvalidParam`] with a sentence of our own.
///
/// Without these, a handler taking `Form<T>` or `Json<T>` answers axum's
/// default rejection response: `400` with a **plain-text** body ("Failed to
/// deserialize form body: missing field `amount`") and no
/// `Content-Type: application/json`. An SDK that reads `error.message` finds
/// nothing at all, which breaks the promise `docs/api/README.md` makes that
/// *every* failure is the Stripe envelope. With them,
/// `async fn handler(Form(f): Form<T>) -> Result<_, ApiError>` is enough —
/// axum applies `From` to the rejection for us.
///
/// **The library's own text is deliberately dropped.** It names serde
/// internals and the field names of our own structs, it changes between axum
/// patch releases (pinning it in a test would pin someone else's string), and
/// for `Json` it can echo the caller's bytes back. The curated sentence says
/// what to fix without any of that. The cost is real and worth naming: the
/// rejection is consumed here, so serde's diagnosis does not reach the log
/// either. If an operator ever needs it, the fix is a `#[source]` on the
/// variant — not the library's words in a response body.
macro_rules! extractor_rejection {
    ($rejection:ty, $param:literal, $message:literal) => {
        impl From<$rejection> for ApiError {
            fn from(_rejection: $rejection) -> Self {
                Self::InvalidParam {
                    param: $param.to_owned(),
                    message: $message.to_owned(),
                }
            }
        }
    };
}

extractor_rejection!(
    FormRejection,
    "body",
    "The request body could not be read as a form. Send it as `application/x-www-form-urlencoded` with the fields this endpoint documents."
);
extractor_rejection!(
    JsonRejection,
    "body",
    "The request body could not be read as JSON. Send a JSON object with `Content-Type: application/json` and the fields this endpoint documents."
);
extractor_rejection!(
    PathRejection,
    "path",
    "A path segment of the request URL was missing or was not of the expected type."
);
extractor_rejection!(
    QueryRejection,
    "query",
    "The query string could not be read. Check the parameters this endpoint documents and their types."
);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // Logged before rendering, so an error still reaches the log if
        // serialising the body were ever to fail.
        self.log();

        let category = self.category();
        // Every `Category::http_status()` is a valid status code — pinned by
        // `vpay-core`'s own `every_category_has_a_status_in_the_4xx_or_5xx_range`
        // — so this conversion cannot fail. It is written as a fallback
        // rather than an `expect` because ADR-0007 denies panics on a
        // request path, and answering 500 is strictly better than killing
        // the connection over an unreachable branch.
        let status = StatusCode::from_u16(category.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        let body = error_envelope_with_param(
            category.stripe_type(),
            self.code(),
            &self.public_message(),
            self.param(),
        );

        // Asked of the same classification the status came from, so the
        // header cannot drift from the category the way a per-handler or
        // per-status rule would. See `STRIPE_SHOULD_RETRY_HEADER`.
        let should_retry = match self.retry() {
            Retry::AfterBackoff => RETRY_YES,
            Retry::Never | Retry::NewAttempt => RETRY_NO,
        };

        let mut response = (status, Json(body)).into_response();
        response
            .headers_mut()
            .insert(STRIPE_SHOULD_RETRY_HEADER, should_retry);
        response
    }
}

/// Compile-time proof that an `ApiError` can cross axum's boundaries.
///
/// A handler's return type must be `Send` to be awaited in a spawned task,
/// and `'static` to be boxed into a `Response`'s extensions or an error
/// layer. Wrapping a leaf that was not `Send + Sync` (a `Rc`, an
/// `Error` trait object without those bounds) would fail here, at the
/// composite, instead of at a distant handler with an error naming a
/// closure. Cheap to keep, and it is the assertion that would break first.
const fn assert_send_sync<T: Send + Sync + 'static>() {}
const _: () = assert_send_sync::<ApiError>();

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::extract::{Form, Path, Query};
    use axum::http::Request;
    use axum::routing::get;
    use axum::routing::post;
    use serde_json::Value;
    use tower::ServiceExt as _;
    use vpay_core::MoneyError;
    use vpay_core::money::UnknownCurrency;
    use vpay_db::DbError;
    use vpay_provider::ProviderError;

    use super::*;
    use crate::test_log::with_captured_log;

    /// A string that could only have come from inside a `sqlx::Error` — the
    /// stand-in for the host, credential or table name a real driver error
    /// carries.
    const LEAKY: &str = "host-secret-xyz";

    fn leaky_db_error() -> DbError {
        DbError::Connect(sqlx::Error::Configuration(LEAKY.into()))
    }

    async fn body_string(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("reading the response body succeeds");
        String::from_utf8(bytes.to_vec()).expect("the envelope is utf-8")
    }

    async fn body_json(response: Response) -> Value {
        serde_json::from_str(&body_string(response).await).expect("the body is valid JSON")
    }

    /// Walks a JSON envelope without `value["key"]` indexing, mirroring
    /// `resource_auth`'s own helper (the workspace warns on
    /// `clippy::indexing_slicing`).
    fn error_field<'a>(envelope: &'a Value, field: &str) -> Option<&'a str> {
        envelope.get("error")?.get(field)?.as_str()
    }

    /// One constructor per case, rather than a `Vec<ApiError>`, because
    /// `ApiError` is not `Clone` (a `sqlx::Error` is not) and each case is
    /// consumed twice: once directly and once through a real router.
    type Case = (fn() -> ApiError, u16, &'static str, &'static str);

    /// [`Case`]'s sibling for `the_retry_advisory_follows_the_classification_not_the_status`:
    /// constructor, expected status, expected `stripe-should-retry`. Same
    /// reason it is a `fn()` rather than a value — `ApiError` is not `Clone`.
    type RetryCase = (fn() -> ApiError, u16, &'static str);

    /// At least one variant per wrapped leaf, plus every variant this layer
    /// owns. The expectations are written from `docs/flows/errors.md`'s
    /// policy table and each leaf's own `Classify` impl — never read back
    /// from `ApiError`.
    fn cases() -> Vec<Case> {
        vec![
            // vpay-db: Storage → 503 / api_error, code overridden per variant.
            (
                || ApiError::Db(leaky_db_error()),
                503,
                "api_error",
                "database_unreachable",
            ),
            (
                || ApiError::Db(DbError::Query(sqlx::Error::RowNotFound)),
                503,
                "api_error",
                "database_query_failed",
            ),
            // vpay-provider: Rail → 502.
            (
                || ApiError::Provider(ProviderError::transport("connect timed out")),
                502,
                "api_error",
                "provider_unavailable",
            ),
            // ... and a rail *decision*, which keeps the failure taxonomy's
            // own code through the delegation rather than `invalid_state`.
            (
                || {
                    ApiError::Provider(ProviderError::Rejected {
                        code: vpay_core::FailureCode::InsufficientFunds,
                        message: "balance too low".into(),
                    })
                },
                409,
                "invalid_request_error",
                // One code for every decline, not the `FailureCode`'s own
                // string: `provider_unavailable` already means "502, being
                // retried" when `Transport` emits it. The specific taxonomy
                // code reaches the merchant in the message and on the charge.
                "charge_declined",
            ),
            (
                || ApiError::Provider(ProviderError::Unsupported),
                409,
                "invalid_request_error",
                "operation_unsupported_by_rail",
            ),
            // vpay-core money: the caller's fault ...
            (
                || ApiError::Money(MoneyError::Negative(-1)),
                400,
                "invalid_request_error",
                "amount_negative",
            ),
            // ... and ours, from the same enum: the delegation must not
            // flatten these two into one category.
            (
                || ApiError::Money(MoneyError::Overflow),
                500,
                "api_error",
                "internal_error",
            ),
            (
                || ApiError::Currency(UnknownCurrency("XYZ".into())),
                400,
                "invalid_request_error",
                "currency_unknown",
            ),
            // vpay-ledger: our own invariant, in the money path — 500 and it
            // pages. Nothing about the ledger reaches the merchant.
            (
                || {
                    ApiError::Ledger(LedgerError::Unbalanced {
                        debits: 5_000,
                        credits: 4_900,
                    })
                },
                500,
                "api_error",
                "ledger_unbalanced",
            ),
            // ... and the same enum delegating onward to `MoneyError`, which
            // is a *caller's* mistake: 400, not 500. A composite that
            // flattened `LedgerError` to one category would get this wrong.
            (
                || ApiError::Ledger(LedgerError::Money(MoneyError::Negative(-5))),
                400,
                "invalid_request_error",
                "amount_negative",
            ),
            // vpay-config: Configuration → 500, never retried.
            (
                || ApiError::Config(vpay_config::ConfigError::MissingPath),
                500,
                "api_error",
                "misconfigured",
            ),
            // The auth leaf, per variant.
            (
                || ApiError::Auth(AuthRejection::MissingHeader),
                401,
                "authentication_error",
                "missing_bearer_token",
            ),
            (
                || ApiError::Auth(AuthRejection::MalformedHeader),
                401,
                "authentication_error",
                "malformed_authorization_header",
            ),
            (
                || ApiError::Auth(AuthRejection::InvalidToken),
                401,
                "authentication_error",
                "invalid_token",
            ),
            // The one auth rejection that is *not* about the credential: a
            // JWKS this process could not fetch is our outage, and it has to
            // answer 503/`api_error` so an SDK backs off instead of
            // re-authenticating. A 401 here would send every client back to
            // the (database-backed) token endpoint during an outage.
            (
                || ApiError::Auth(AuthRejection::KeysUnavailable),
                503,
                "api_error",
                "service_unavailable",
            ),
            // This layer's own variants.
            (
                || ApiError::UnknownRoute {
                    method: "POST".into(),
                    path: "/v1/payment_intents".into(),
                },
                404,
                "invalid_request_error",
                "unknown_route",
            ),
            (
                || ApiError::invalid_param("amount", "amount must be a positive integer"),
                400,
                "invalid_request_error",
                "invalid_request",
            ),
            (
                || ApiError::idempotency_key_reused("idem_0123456789_tail"),
                400,
                "idempotency_error",
                "idempotency_key_in_use",
            ),
            // Same category and therefore the same status and `type` as the
            // row above; the `code` is the entire difference, which is why
            // both rows are here. If the two ever collapse to one code, a
            // client can no longer tell a merchant bug from a wait.
            (
                || ApiError::idempotency_key_in_flight("idem_0123456789_tail"),
                400,
                "idempotency_error",
                "idempotency_key_in_flight",
            ),
            (
                || ApiError::NotFound {
                    resource: "payment_intent",
                    id: "pi_0000000000000000000000000".into(),
                },
                404,
                "invalid_request_error",
                "resource_missing",
            ),
            (
                || ApiError::Conflict {
                    message: "This PaymentIntent is already processing.".into(),
                },
                409,
                "invalid_request_error",
                "invalid_state",
            ),
            (
                || ApiError::Forbidden,
                403,
                "invalid_request_error",
                "forbidden",
            ),
            // Two rows for one variant, because the `code` is the entire
            // difference between them and the whole reason the variant is not
            // `Conflict`. Same status and same `type` as the `Conflict` row
            // above, and a different code from it and from each other: if any
            // two of the three ever collapse, a merchant can no longer tell
            // "your payer abandoned this checkout" from "this intent is
            // already processing".
            (
                || ApiError::CheckoutSessionNotOpen {
                    session_id: "cs_00000000000000000000000001".into(),
                    state: ClosedSession::Expired,
                },
                409,
                "invalid_request_error",
                "checkout_session_expired",
            ),
            (
                || ApiError::CheckoutSessionNotOpen {
                    session_id: "cs_00000000000000000000000001".into(),
                    state: ClosedSession::Complete,
                },
                409,
                "invalid_request_error",
                "checkout_session_complete",
            ),
            (
                || ApiError::Internal("the ledger did not balance".into()),
                500,
                "api_error",
                "internal_error",
            ),
        ]
    }

    /// The two checkout-session refusals are 409s a merchant can act on, and
    /// each says something the other two 409s on the confirm path do not.
    ///
    /// ADR-0011's three derived facts are asserted through the
    /// classification, never chosen here: the category picks 409 and
    /// `invalid_request_error`, the category picks `Retry::Never`, and only
    /// the `code` is this variant's own.
    ///
    /// **Revert-proof.** Make the variant answer `Category::Conflict`'s
    /// default code and the two `assert_ne!`s against `invalid_state` fail;
    /// collapse the two states onto one code and the first `assert_ne!`
    /// fails.
    #[tokio::test]
    async fn a_closed_checkout_session_is_its_own_409_and_not_a_lifecycle_conflict() {
        const SESSION: &str = "cs_00000000000000000000000001";

        let expired = ApiError::CheckoutSessionNotOpen {
            session_id: SESSION.to_owned(),
            state: ClosedSession::Expired,
        };
        let complete = ApiError::CheckoutSessionNotOpen {
            session_id: SESSION.to_owned(),
            state: ClosedSession::Complete,
        };
        let lifecycle = ApiError::Conflict {
            message: "This PaymentIntent already has a charge.".to_owned(),
        };

        assert_ne!(
            expired.code(),
            complete.code(),
            "an abandoned checkout and a finished one need different handling"
        );
        for (label, error) in [("expired", &expired), ("complete", &complete)] {
            assert_eq!(error.category(), Category::Conflict, "{label}");
            assert_eq!(error.category().http_status(), 409, "{label}");
            assert_eq!(
                error.retry(),
                Retry::Never,
                "{label}: a closed session never reopens"
            );
            assert_ne!(
                error.code(),
                lifecycle.code(),
                "{label}: it must not be indistinguishable from `invalid_state`"
            );
        }

        let response = expired.into_response();
        assert_eq!(response.status().as_u16(), 409);
        let body = body_json(response).await;
        assert_eq!(error_field(&body, "type"), Some("invalid_request_error"));
        assert_eq!(error_field(&body, "code"), Some("checkout_session_expired"));
        // Pinned verbatim, not by `contains`. These sentences are built by
        // joining fragments across source lines, and a join that lost a space
        // or gained a run of them would still contain the session id and
        // still read fine in a diff — the exact string is the only assertion
        // that catches it.
        assert_eq!(
            error_field(&body, "message"),
            Some(
                "The checkout session cs_00000000000000000000000001 for this PaymentIntent has \
                 expired, so it can no longer be confirmed. Create a new checkout session for \
                 this intent to offer the payer another link."
            ),
            "{body:#}"
        );
        assert_eq!(
            body.pointer("/error/param"),
            None,
            "the session's state is not a request parameter, so nothing points at one: {body:#}"
        );

        let body = body_json(complete.into_response()).await;
        assert_eq!(
            error_field(&body, "code"),
            Some("checkout_session_complete")
        );
        assert_eq!(
            error_field(&body, "message"),
            Some(
                "The checkout session cs_00000000000000000000000001 for this PaymentIntent is \
                 already complete, so it cannot be confirmed again."
            ),
            "{body:#}"
        );
    }

    /// A 404 for someone else's object must be indistinguishable from a 404
    /// for an object that never existed, or the API answers "does this id
    /// exist under some other merchant?" — see `ApiError::NotFound`. This is
    /// the assertion behind the integration suite's
    /// `merchant_b_cannot_read_merchant_as_intent`; it lives here too because
    /// the property belongs to the *renderer*, and a future `Forbidden` arm
    /// added for "wrong tenant" would break it without touching a handler.
    #[tokio::test]
    async fn a_foreign_object_and_a_missing_object_are_byte_identical() {
        const ID: &str = "pi_zzzzzzzzzzzzzzzzzzzzzzzz";
        let foreign = ApiError::NotFound {
            resource: "payment_intent",
            id: ID.to_owned(),
        };
        let missing = ApiError::NotFound {
            resource: "payment_intent",
            id: ID.to_owned(),
        };
        assert_eq!(
            body_string(foreign.into_response()).await,
            body_string(missing.into_response()).await
        );

        // And it is *not* the shape a Forbidden would have had, which is the
        // mistake this variant exists to prevent.
        assert_ne!(
            ApiError::Forbidden.category().http_status(),
            ApiError::NotFound {
                resource: "payment_intent",
                id: ID.to_owned(),
            }
            .category()
            .http_status()
        );
    }

    /// The three variants Step 2 added carry text a *caller* controls (an id
    /// out of a URL path) or text a call site writes freehand. Both go
    /// through the same bound every other public message does.
    #[tokio::test]
    async fn the_step_2_variants_say_what_they_should_and_no_more() {
        let body = body_json(
            ApiError::NotFound {
                resource: "payment_intent",
                id: "pi_missing".to_owned(),
            }
            .into_response(),
        )
        .await;
        assert_eq!(
            error_field(&body, "message"),
            Some("No such payment_intent: pi_missing"),
            "the id is echoed, as Stripe does"
        );
        assert!(
            body.get("error").is_some_and(|e| e.get("param").is_none()),
            "a missing object is not a bad parameter: {body}"
        );

        // A megabyte of id in a path segment must not be a megabyte of body.
        let body = body_string(
            ApiError::NotFound {
                resource: "payment_intent",
                id: "z".repeat(1024 * 1024),
            }
            .into_response(),
        )
        .await;
        assert!(
            body.len() < 1_024,
            "the envelope must stay small: {}",
            body.len()
        );

        let body = body_json(
            ApiError::Conflict {
                message:
                    "A PaymentIntent may only be canceled while it is requires_payment_method."
                        .to_owned(),
            }
            .into_response(),
        )
        .await;
        assert_eq!(
            error_field(&body, "message"),
            Some("A PaymentIntent may only be canceled while it is requires_payment_method.")
        );

        // `Forbidden` says nothing about which scope was missing.
        let body = body_json(ApiError::Forbidden.into_response()).await;
        assert_eq!(
            error_field(&body, "message"),
            Some("This client is not permitted to perform that action.")
        );
    }

    #[test]
    fn every_variant_answers_with_the_classification_its_leaf_chose() {
        for (build, status, kind, code) in cases() {
            let error = build();
            let label = format!("{error:?}");
            assert_eq!(
                error.category().http_status(),
                status,
                "{label}: wrong status"
            );
            assert_eq!(
                error.category().stripe_type(),
                kind,
                "{label}: wrong stripe type"
            );
            assert_eq!(error.code(), code, "{label}: wrong code");
        }
    }

    #[tokio::test]
    async fn every_variant_renders_that_classification_over_a_real_router() {
        for (build, status, kind, code) in cases() {
            let label = format!("{:?}", build());
            let app = Router::new().route("/e", get(move || async move { build() }));
            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/e")
                        .body(Body::empty())
                        .expect("valid request"),
                )
                .await
                .expect("router does not fail to serve");

            assert_eq!(response.status().as_u16(), status, "{label}: wrong status");
            let body = body_json(response).await;
            assert_eq!(error_field(&body, "type"), Some(kind), "{label}");
            assert_eq!(error_field(&body, "code"), Some(code), "{label}");
            assert!(
                error_field(&body, "message").is_some_and(|m| !m.is_empty()),
                "{label}: every envelope carries a message"
            );
        }
    }

    #[tokio::test]
    async fn a_storage_errors_leaf_text_reaches_the_log_and_never_the_body() {
        let error = ApiError::Db(leaky_db_error());

        // The chain is where the driver's own words live: `DbError`'s
        // `Display` names Postgres, the `sqlx::Error` underneath names the
        // (here, deliberately recognisable) reason.
        assert!(
            error.source_chain().contains(LEAKY),
            "the source chain must carry the leaf's text: {}",
            error.source_chain()
        );
        assert!(
            !error.public_message().contains(LEAKY),
            "public message leaked the leaf's text: {}",
            error.public_message()
        );

        let (response, log) = with_captured_log(|| error.into_response());
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = body_string(response).await;
        assert!(
            !body.contains(LEAKY),
            "the response body leaked the leaf's text: {body}"
        );
        assert!(
            body.contains("vpay is temporarily unavailable"),
            "the merchant gets the category's generic sentence: {body}"
        );

        assert!(
            log.contains(LEAKY),
            "the operator's log must carry what the merchant's body does not: {log}"
        );
        assert!(
            log.contains("source_chain=error with configuration: host-secret-xyz"),
            "the log must carry the walked source chain, not only the Display: {log}"
        );
    }

    #[test]
    fn severity_decides_the_level_and_a_page_is_marked_for_alerting() {
        let (_, info) = with_captured_log(|| ApiError::Internal("x".into()).log());
        assert!(info.contains("ERROR"), "an Internal error logs at ERROR");
        assert!(
            info.contains("alert=true"),
            "a Page severity must be selectable by an alerting rule: {info}"
        );

        let (_, storage) = with_captured_log(|| ApiError::Db(leaky_db_error()).log());
        assert!(storage.contains("ERROR"), "a DbError logs at ERROR");
        assert!(
            !storage.contains("alert=true"),
            "an ordinary error must not page: {storage}"
        );

        let (_, rail) = with_captured_log(|| {
            ApiError::Provider(ProviderError::transport("timeout")).log();
        });
        assert!(rail.contains("WARN"), "a rail timeout warns: {rail}");

        let (_, caller) = with_captured_log(|| {
            ApiError::invalid_param("amount", "must be positive").log();
        });
        assert!(
            caller.contains("INFO"),
            "a caller's mistake informs: {caller}"
        );
    }

    /// The same decision, on the other channel: the log line's
    /// `alert = true` and the counter an alert rule fires on are one call,
    /// so they cannot disagree about what a page is.
    ///
    /// `Internal` pages and `Db` does not, which is exactly the pair the
    /// test above asserts on the log side — asserted here on the metrics
    /// side, in one recorder, so "both counters moved" and "only the error
    /// counter moved" are two lines of one document rather than two runs.
    #[test]
    fn a_page_severity_error_increments_the_alert_counter_and_a_storage_error_does_not() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            // `log()` and not `into_response()`: the counters belong to the
            // operator-facing half, and this pins them to the same function
            // that writes `alert = true`.
            let _ = with_captured_log(|| ApiError::Internal("x".into()).log());
            let _ = with_captured_log(|| ApiError::Db(leaky_db_error()).log());
        });
        let scrape = handle.render();

        assert!(
            scrape.contains(
                r#"vpay_error_events_total{category="Internal",code="internal_error",severity="Page"} 1"#
            ),
            "{scrape}"
        );
        assert!(
            scrape.contains(
                r#"vpay_alert_events_total{category="Internal",code="internal_error"} 1"#
            ),
            "an ApiError that logs `alert = true` must also reach vpay_alert_events_total, or \
             VpayPageableErrorEvents cannot fire on it: {scrape}"
        );
        assert!(
            scrape.contains(r#"severity="Error""#),
            "the DbError must still be counted, at its own severity: {scrape}"
        );
        assert_eq!(
            scrape
                .lines()
                .filter(|line| line.starts_with("vpay_alert_events_total{"))
                .count(),
            1,
            "only the Page-severity error may page: {scrape}"
        );
    }

    #[tokio::test]
    async fn param_is_in_the_envelope_exactly_when_the_variant_names_one() {
        let with_param = ApiError::invalid_param("amount", "amount must be a positive integer");
        assert_eq!(with_param.param(), Some("amount"));
        let body = body_json(with_param.into_response()).await;
        assert_eq!(error_field(&body, "param"), Some("amount"));
        assert_eq!(
            error_field(&body, "message"),
            Some("amount must be a positive integer")
        );

        for (build, ..) in cases() {
            let error = build();
            if error.param().is_some() {
                continue;
            }
            let label = format!("{error:?}");
            let body = body_json(error.into_response()).await;
            let error_object = body
                .get("error")
                .expect("every envelope has an error object");
            assert!(
                error_object.get("param").is_none(),
                "{label}: `param` must be absent, not null, when there is none"
            );
        }
    }

    #[tokio::test]
    async fn an_idempotency_key_is_never_echoed_past_its_hint() {
        const KEY: &str = "idem_0123_merchant_order_88_customer_email";
        let error = ApiError::idempotency_key_reused(KEY);

        let body = body_string(error.into_response()).await;
        assert!(
            body.contains("idem_012…"),
            "the hint identifies the key to its owner: {body}"
        );
        assert!(
            !body.contains("merchant_order_88"),
            "the body echoed past the hint: {body}"
        );
        // Belt and braces against a hand-built variant that skipped the
        // constructor: the render path truncates too.
        let body = body_string(
            ApiError::IdempotencyKeyReused {
                key_hint: KEY.to_owned(),
            }
            .into_response(),
        )
        .await;
        assert!(
            !body.contains("merchant_order_88"),
            "a hand-built variant must still be truncated on the way out: {body}"
        );
    }

    /// The two idempotency refusals are told apart by `code`, and the
    /// in-flight one says nothing about the key.
    ///
    /// **This is the assertion that fails if `IdempotencyKeyInFlight` is
    /// removed and the handler goes back to `ApiError::Conflict`**: the code
    /// becomes `invalid_state`, which is the same code a lifecycle conflict
    /// carries, and a client can no longer tell "wait" from "your intent
    /// moved on". `docs/flows/errors.md`'s policy table is what decides the
    /// status and the type; only the code is this variant's own.
    #[tokio::test]
    async fn a_key_still_in_flight_is_a_different_code_from_a_key_reused_and_from_a_conflict() {
        const KEY: &str = "idem_0123_merchant_order_88_customer_email";

        let in_flight = ApiError::idempotency_key_in_flight(KEY);
        assert_eq!(in_flight.code(), "idempotency_key_in_flight");
        assert_eq!(in_flight.category(), Category::Idempotency);
        // It heals on its own — the only variant this layer owns that does.
        assert_eq!(in_flight.retry(), Retry::AfterBackoff);
        assert_ne!(
            in_flight.code(),
            ApiError::idempotency_key_reused(KEY).code(),
            "a body mismatch and a request still running are different problems"
        );
        assert_ne!(
            in_flight.code(),
            ApiError::Conflict {
                message: "This PaymentIntent already has a charge.".into(),
            }
            .code(),
            "an in-flight key must not render as a lifecycle conflict"
        );

        let response = in_flight.into_response();
        assert_eq!(response.status(), Category::Idempotency.http_status());
        let body = body_json(response).await;
        assert_eq!(error_field(&body, "type"), Some("idempotency_error"));
        assert_eq!(
            error_field(&body, "code"),
            Some("idempotency_key_in_flight")
        );
        assert_eq!(
            error_field(&body, "message"),
            Some("A request with this Idempotency-Key is still in progress; retry shortly.")
        );
        // The key is not in the body at all — not even as a hint. It is in
        // the log line, which is where an operator correlates it.
        let rendered = serde_json::to_string(&body).expect("the envelope re-serialises");
        assert!(
            !rendered.contains("idem_0123"),
            "the in-flight message must not carry the key: {rendered}"
        );
        let (_, log) = with_captured_log(|| {
            ApiError::idempotency_key_in_flight(KEY).log();
        });
        assert!(
            log.contains("idem_012\u{2026}") || log.contains("idem_012"),
            "the operator half must carry the hint: {log}"
        );
        assert!(
            !log.contains("merchant_order_88"),
            "not even the log carries the whole key: {log}"
        );
    }

    // --- pinned bytes: what these two responses have always answered ---
    //
    // Captured from the implementation *before* `ApiError` existed and
    // asserted verbatim, so routing the fallback and `AuthRejection` through
    // the composite is provably a refactor and not a wire change. Any
    // difference — a re-worded sentence, a reordered key, a `param` that
    // should not be there — fails here.

    const PINNED_404: &str = r#"{"error":{"code":"unknown_route","message":"Unrecognized request URL. vpay implements a subset of the Stripe API; see docs/api.","type":"invalid_request_error"}}"#;

    const PINNED_MISSING_HEADER: &str = r#"{"error":{"code":"missing_bearer_token","message":"No Authorization header was provided. Send an OAuth2 access token as 'Authorization: Bearer <token>'.","type":"authentication_error"}}"#;

    const PINNED_MALFORMED_HEADER: &str = r#"{"error":{"code":"malformed_authorization_header","message":"The Authorization header was present but was not a well-formed 'Bearer <token>' value.","type":"authentication_error"}}"#;

    const PINNED_INVALID_TOKEN: &str = r#"{"error":{"code":"invalid_token","message":"The bearer token is invalid, expired, or was not issued for this endpoint.","type":"authentication_error"}}"#;

    /// The 404 path, driven through the real router.
    ///
    /// The URI is deliberately outside `/v1`: since the merchant
    /// authentication layer went in front of that nest, a `/v1/...` request
    /// with no bearer token is a 401 and never reaches the fallback. The
    /// *bytes* asserted below are unchanged — the envelope never echoed the
    /// path back (see `ApiError::public_message`) — so this still pins
    /// exactly what it pinned before.
    #[tokio::test]
    async fn the_404_fallback_is_byte_for_byte_what_it_was_before_api_error() {
        let response = crate::router(crate::test_fixtures::deps())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/not_a_vpay_route")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router does not fail to serve");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_string(response).await, PINNED_404);
    }

    #[tokio::test]
    async fn every_auth_rejection_is_byte_for_byte_what_it_was_before_api_error() {
        let expected = [
            (AuthRejection::MissingHeader, PINNED_MISSING_HEADER),
            (AuthRejection::MalformedHeader, PINNED_MALFORMED_HEADER),
            (AuthRejection::InvalidToken, PINNED_INVALID_TOKEN),
        ];

        for (rejection, pinned) in expected {
            let label = format!("{rejection:?}");
            // Through `AuthRejection`'s own `IntoResponse` — the path an
            // extractor rejection takes.
            let response = rejection.into_response();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{label}");
            assert_eq!(body_string(response).await, pinned, "{label}");
        }

        // And through the composite, which is what the above now delegates
        // to: identical bytes, or the delegation changed the wire.
        for (rejection, pinned) in [
            (AuthRejection::MissingHeader, PINNED_MISSING_HEADER),
            (AuthRejection::MalformedHeader, PINNED_MALFORMED_HEADER),
            (AuthRejection::InvalidToken, PINNED_INVALID_TOKEN),
        ] {
            let label = format!("{rejection:?}");
            let response = ApiError::from(rejection).into_response();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{label}");
            assert_eq!(body_string(response).await, pinned, "{label}");
        }
    }

    /// `KeysUnavailable` is deliberately **not** in the list above: it is the
    /// one [`AuthRejection`] that does not answer 401, so pinning it there
    /// would have meant weakening that test's status assertion for every
    /// variant. It gets the same both-paths treatment separately.
    ///
    /// The bytes are the `Category::Storage` policy row verbatim — 503,
    /// `api_error`, `service_unavailable`, and the sentence that tells a
    /// caller to retry rather than to re-authenticate.
    #[tokio::test]
    async fn a_jwks_outage_renders_as_503_through_both_paths() {
        const PINNED_KEYS_UNAVAILABLE: &str = r#"{"error":{"code":"service_unavailable","message":"vpay is temporarily unavailable. Retry after a short delay.","type":"api_error"}}"#;

        let response = AuthRejection::KeysUnavailable.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_string(response).await, PINNED_KEYS_UNAVAILABLE);

        let response = ApiError::from(AuthRejection::KeysUnavailable).into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_string(response).await, PINNED_KEYS_UNAVAILABLE);
    }

    /// Wrapping must not change the answer — ADR-0011's "composites do not
    /// re-classify", as an executable assertion, and the twin of
    /// `vpay_worker::error`'s test of the same name. It fails if any of the
    /// five `Classify` methods here stops delegating: forwarding
    /// `category()` alone and letting the trait defaults fill in the rest
    /// looks correct and silently discards every override a leaf made.
    ///
    /// One leaf per interesting override: `Rejected` overrides code, retry
    /// *and* severity; `Negative` overrides code and message; `UnknownCurrency`
    /// overrides both and bounds the message; `DbError::Connect` overrides
    /// the code; `LedgerError::Unbalanced` overrides code and severity;
    /// `AuthRejection::InvalidToken` is the one that must stay a 401 with
    /// nothing said about why.
    #[test]
    fn wrapping_a_leaf_preserves_every_classification_the_leaf_chose() {
        macro_rules! assert_delegates {
            ($wrapped:expr, $leaf:expr) => {{
                let wrapped: ApiError = $wrapped;
                let leaf = $leaf;
                let label = format!("{leaf:?}");
                assert_eq!(wrapped.category(), leaf.category(), "{label}: category");
                assert_eq!(wrapped.code(), leaf.code(), "{label}: code");
                assert_eq!(wrapped.retry(), leaf.retry(), "{label}: retry");
                assert_eq!(wrapped.severity(), leaf.severity(), "{label}: severity");
                assert_eq!(
                    wrapped.public_message(),
                    leaf.public_message(),
                    "{label}: public message"
                );
            }};
        }

        assert_delegates!(
            ApiError::Provider(ProviderError::Rejected {
                code: vpay_core::FailureCode::ProviderAccountBlocked,
                message: "partner account suspended".into(),
            }),
            ProviderError::Rejected {
                code: vpay_core::FailureCode::ProviderAccountBlocked,
                message: "partner account suspended".into(),
            }
        );
        assert_delegates!(
            ApiError::Money(MoneyError::Negative(-1)),
            MoneyError::Negative(-1)
        );
        assert_delegates!(
            ApiError::Currency(UnknownCurrency("XYZ".into())),
            UnknownCurrency("XYZ".into())
        );
        assert_delegates!(ApiError::Db(leaky_db_error()), leaky_db_error());
        assert_delegates!(
            ApiError::Ledger(LedgerError::Unbalanced {
                debits: 5_000,
                credits: 4_900,
            }),
            LedgerError::Unbalanced {
                debits: 5_000,
                credits: 4_900,
            }
        );
        assert_delegates!(
            ApiError::Auth(AuthRejection::InvalidToken),
            AuthRejection::InvalidToken
        );
        // The leaf that overrides its *sibling variants'* category rather
        // than its own category's defaults: a composite that mapped
        // `Self::Auth(_)` to `Category::Authentication` wholesale — the
        // obvious shortcut, and what this arm looked like before
        // `KeysUnavailable` existed — would answer 401 here and fail.
        assert_delegates!(
            ApiError::Auth(AuthRejection::KeysUnavailable),
            AuthRejection::KeysUnavailable
        );

        // The assertions above are only worth anything if at least one leaf
        // actually disagrees with its category's defaults — otherwise a
        // category-only delegation would pass them all.
        let declined = ProviderError::Rejected {
            code: vpay_core::FailureCode::ProviderAccountBlocked,
            message: "partner account suspended".into(),
        };
        assert_ne!(declined.code(), Category::Conflict.default_code());
        assert_ne!(declined.retry(), Category::Conflict.default_retry());
        assert_ne!(declined.severity(), Category::Conflict.default_severity());
    }

    // --- extractor rejections: axum's, rendered as ours ---

    #[derive(Debug, serde::Deserialize)]
    struct Payload {
        #[allow(
            dead_code,
            reason = "the field exists so deserialisation can fail without it"
        )]
        amount: i64,
    }

    /// The pattern a real handler uses to route an extractor rejection
    /// through the composite: take `Result<Form<T>, FormRejection>` and `?`
    /// it, which applies `From<FormRejection> for ApiError`. Without the
    /// conversion this handler would not compile; without the extractor
    /// being a *real* `Form<T>`, the test would prove nothing about axum.
    async fn form_handler(
        form: Result<Form<Payload>, FormRejection>,
    ) -> Result<&'static str, ApiError> {
        let Form(_payload) = form?;
        Ok("accepted")
    }

    async fn json_handler(
        json: Result<axum::Json<Payload>, JsonRejection>,
    ) -> Result<&'static str, ApiError> {
        let axum::Json(_payload) = json?;
        Ok("accepted")
    }

    async fn query_handler(
        query: Result<Query<Payload>, QueryRejection>,
    ) -> Result<&'static str, ApiError> {
        let Query(_payload) = query?;
        Ok("accepted")
    }

    async fn path_handler(
        path: Result<Path<u32>, PathRejection>,
    ) -> Result<&'static str, ApiError> {
        let Path(_id) = path?;
        Ok("accepted")
    }

    /// The failure this conversion exists to prevent: axum's own
    /// `FormRejection` response is `text/plain` with a serde sentence in it,
    /// so an SDK reading `error.message` from a 400 would find nothing.
    #[tokio::test]
    async fn a_form_rejection_is_answered_with_the_envelope_not_axums_plain_text() {
        let app = Router::new().route("/f", post(form_handler));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/f")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("not_the_field=1"))
                    .expect("valid request"),
            )
            .await
            .expect("router does not fail to serve");

        assert_eq!(response.status().as_u16(), 400);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/json"),
            "axum's own rejection would answer text/plain"
        );
        let body = body_json(response).await;
        assert_eq!(error_field(&body, "type"), Some("invalid_request_error"));
        assert_eq!(error_field(&body, "code"), Some("invalid_request"));
        assert_eq!(error_field(&body, "param"), Some("body"));
        let message = error_field(&body, "message").expect("every envelope carries a message");
        assert!(
            message.contains("form"),
            "the message must say what to fix: {message}"
        );
        assert!(
            !message.to_ascii_lowercase().contains("deserialize"),
            "axum's own text must not be echoed: {message}"
        );
    }

    /// `Form` is the one the reviewers asked for; the other three are the
    /// same conversion and would otherwise be untested. Each names the part
    /// of the request it came from, because that is what an SDK points at.
    #[tokio::test]
    async fn every_extractor_rejection_names_the_part_of_the_request_it_came_from() {
        struct Case {
            name: &'static str,
            app: Router,
            request: Request<Body>,
            param: &'static str,
        }

        let cases = vec![
            Case {
                name: "Json — no content-type at all",
                app: Router::new().route("/j", post(json_handler)),
                request: Request::builder()
                    .method("POST")
                    .uri("/j")
                    .body(Body::from("{}"))
                    .expect("valid request"),
                param: "body",
            },
            Case {
                name: "Query — the required field is missing",
                app: Router::new().route("/q", post(query_handler)),
                request: Request::builder()
                    .method("POST")
                    .uri("/q?something_else=1")
                    .body(Body::empty())
                    .expect("valid request"),
                param: "query",
            },
            Case {
                name: "Path — the segment is not a u32",
                app: Router::new().route("/p/{id}", post(path_handler)),
                request: Request::builder()
                    .method("POST")
                    .uri("/p/not-a-number")
                    .body(Body::empty())
                    .expect("valid request"),
                param: "path",
            },
        ];

        for case in cases {
            let response = case
                .app
                .oneshot(case.request)
                .await
                .expect("router does not fail to serve");
            assert_eq!(response.status().as_u16(), 400, "{}", case.name);
            let body = body_json(response).await;
            assert_eq!(
                error_field(&body, "type"),
                Some("invalid_request_error"),
                "{}",
                case.name
            );
            assert_eq!(
                error_field(&body, "param"),
                Some(case.param),
                "{}",
                case.name
            );
            assert!(
                error_field(&body, "message").is_some_and(|m| !m.is_empty()),
                "{}",
                case.name
            );
        }
    }

    // --- bounds: a caller cannot reflect a megabyte into the envelope ---

    /// `InvalidParam` is the one variant whose `param` and `message` are
    /// written at a call site, so it is the one a future handler could point
    /// at caller-supplied text. A megabyte in must not be a megabyte out.
    #[tokio::test]
    async fn a_megabyte_of_param_and_message_is_bounded_in_the_envelope() {
        let huge = "x".repeat(1024 * 1024);
        let error = ApiError::invalid_param(huge.clone(), huge);
        let body = body_string(error.into_response()).await;

        assert!(
            body.len() < 1_024,
            "the envelope must stay small, got {} bytes",
            body.len()
        );
        let body: Value = serde_json::from_str(&body).expect("still valid JSON");
        // 64 x's would be a *shaped* name, but a megabyte of them is not:
        // it exceeds PARAM_MAX_CHARS, so it falls back rather than being
        // truncated into a field name that does not exist.
        assert_eq!(error_field(&body, "param"), Some(FALLBACK_PARAM));
        let message = error_field(&body, "message").expect("message");
        assert_eq!(message.chars().count(), MESSAGE_MAX_CHARS + 1);
        assert!(message.ends_with('…'));
    }

    #[test]
    fn only_something_shaped_like_a_field_name_reaches_the_param_field() {
        for good in [
            "amount",
            "payment_method_types[0]",
            "metadata.order_id",
            "a",
            &"n".repeat(PARAM_MAX_CHARS),
        ] {
            assert_eq!(
                ApiError::invalid_param(good, "m").param(),
                Some(good),
                "{good} is a field name"
            );
        }
        for bad in [
            "",                               // no name at all
            "Amount",                         // our vocabulary is snake_case
            "amount; DROP TABLE charges",     // a sentence, not a name
            "https://example.test/?a=b",      // a reflected URL
            "{\"amount\":1}",                 // the caller's own body
            "amount\n\nX-Injected: 1",        // a header-splitting attempt
            &"n".repeat(PARAM_MAX_CHARS + 1), // one character too long
        ] {
            assert_eq!(
                ApiError::invalid_param(bad, "m").param(),
                Some(FALLBACK_PARAM),
                "{bad:?} is not a field name"
            );
        }
    }

    /// The bound lives on the render path, not in the constructor, so a
    /// variant built as a struct literal cannot skip it.
    #[tokio::test]
    async fn a_hand_built_invalid_param_is_bounded_too() {
        let error = ApiError::InvalidParam {
            param: "NOT A FIELD".to_owned(),
            message: "y".repeat(10_000),
        };
        let body = body_json(error.into_response()).await;
        assert_eq!(error_field(&body, "param"), Some(FALLBACK_PARAM));
        assert_eq!(
            error_field(&body, "message")
                .expect("message")
                .chars()
                .count(),
            MESSAGE_MAX_CHARS + 1
        );
    }

    /// The reason there is no `impl From<serde_json::Error> for ApiError`:
    /// the same type means "the caller sent bad JSON" on one path and "we
    /// built a body we cannot serialise" on the other, and only the second
    /// is a 500.
    #[tokio::test]
    async fn a_serialisation_failure_of_ours_is_internal_and_says_nothing() {
        let json_error =
            serde_json::from_str::<Payload>("{").expect_err("that is not a JSON object");
        let text = json_error.to_string();
        let error = ApiError::internal_serialization(json_error);

        assert_eq!(error.category(), Category::Internal);
        assert_eq!(error.severity(), Severity::Page);
        assert!(
            error.to_string().contains(&text),
            "the library's text stays in the operator's half: {error}"
        );

        let response = error.into_response();
        assert_eq!(response.status().as_u16(), 500);
        let body = body_string(response).await;
        assert!(
            !body.contains(&text),
            "the library's text must not reach the merchant: {body}"
        );
        assert!(body.contains("An internal error occurred"), "{body}");
    }

    #[test]
    fn the_source_chain_walks_past_the_first_level() {
        let error = ApiError::Db(leaky_db_error());
        // `Display` is the leaf's (the wrapping is `transparent`), and the
        // chain continues past it: `sqlx::Error::Configuration` is itself a
        // wrapper whose own `#[source]` is the boxed cause, so the walk
        // yields two more levels than `Display` shows. That third level is
        // precisely what a one-level log would throw away.
        assert!(
            error
                .to_string()
                .starts_with("failed to connect to Postgres")
        );
        assert_eq!(
            error.source_chain(),
            format!("error with configuration: {LEAKY}: {LEAKY}")
        );

        // A variant with no source has an empty chain rather than an
        // invented one.
        assert_eq!(ApiError::Internal("x".into()).source_chain(), "");
    }

    /// `stripe-should-retry` says what the *classification* says, which is
    /// not what the status code says.
    ///
    /// The expectations below are written from `docs/flows/errors.md`'s
    /// policy table and the design's §0 S2, never read back from
    /// `Classify::retry` — a table generated from the implementation would
    /// agree with any implementation. Two rows are the whole reason the
    /// header exists, and they are the two where status and advice disagree:
    ///
    /// - `400` / `true` for an in-flight key. stripe-node retries no 4xx, so
    ///   without the header it never retries the one refusal on this surface
    ///   that clears itself.
    /// - `409` / `false` for a lifecycle conflict. stripe-node retries every
    ///   409 unconditionally, so without the header it re-POSTs a permanent
    ///   refusal twice before surfacing it.
    ///
    /// The `500` row is the third: it is a 5xx, which stripe-node also
    /// retries unconditionally, and `Category::Internal` says `Retry::Never`
    /// because an invariant violation does not heal by being asked again.
    #[test]
    fn the_retry_advisory_follows_the_classification_not_the_status() {
        let cases: Vec<RetryCase> = vec![
            (
                || ApiError::idempotency_key_in_flight("idem_0123456789"),
                400,
                "true",
            ),
            (
                || ApiError::Conflict {
                    message: "This PaymentIntent already has a charge.".into(),
                },
                409,
                "false",
            ),
            (
                || ApiError::from(AuthRejection::KeysUnavailable),
                503,
                "true",
            ),
            (
                || ApiError::Internal("an invariant broke".into()),
                500,
                "false",
            ),
            (
                || ApiError::from(AuthRejection::MissingHeader),
                401,
                "false",
            ),
            (
                || ApiError::UnknownRoute {
                    method: "GET".to_owned(),
                    path: "/v1/nope".to_owned(),
                },
                404,
                "false",
            ),
            (
                || ApiError::invalid_param("amount", "`amount` must be a positive integer."),
                400,
                "false",
            ),
            (
                || ApiError::idempotency_key_reused("idem_0123456789"),
                400,
                "false",
            ),
            // A closed checkout session never reopens, so the honest advice
            // is "not this request again". `Category::Conflict`'s default,
            // arrived at through the classification rather than chosen here.
            (
                || ApiError::CheckoutSessionNotOpen {
                    session_id: "cs_00000000000000000000000001".into(),
                    state: ClosedSession::Expired,
                },
                409,
                "false",
            ),
        ];

        for (make, status, advice) in cases {
            let error = make();
            let label = format!("{error:?}");
            let response = error.into_response();
            assert_eq!(response.status().as_u16(), status, "{label}");
            assert_eq!(
                response
                    .headers()
                    .get("stripe-should-retry")
                    .map(|value| value.to_str().expect("the advisory is ascii")),
                Some(advice),
                "{label}"
            );
        }
    }

    /// The advisory reaches the wire through the real router too, not only
    /// through a directly-called `into_response`.
    ///
    /// A layer that rewrote or dropped response headers would leave the test
    /// above green and merchants without the header, so the 404 fallback —
    /// the one error the router produces with no handler involved — is
    /// checked end to end.
    #[tokio::test]
    async fn the_404_fallback_carries_the_retry_advisory() {
        let response = crate::router(crate::test_fixtures::deps())
            .oneshot(
                Request::builder()
                    .uri("/not_a_vpay_route")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router does not fail to serve");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response
                .headers()
                .get("stripe-should-retry")
                .map(|value| value.to_str().expect("the advisory is ascii")),
            Some("false"),
            "an unrouted path never becomes routed by being asked again"
        );
    }
}
