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
//! id, and this module does not invent one. `tower-http`'s
//! `SetRequestIdLayer`/`PropagateRequestIdLayer` (already a workspace
//! dependency, with the `request-id` feature enabled in the root
//! `Cargo.toml`) is the mechanism, and `tower_http::trace::TraceLayer` puts
//! the id on the span that encloses the handler — so every event this module
//! emits inherits it automatically once those layers are mounted in
//! `router()`. Generating a second id here would produce an id that appears
//! in the log and in no response header, which is worse than none.
//! `Category::Internal`'s generic message already promises the merchant a
//! request id ("Contact support with the request id"); honouring that
//! promise is the job of whoever mounts the layer, and it is not mounted
//! today.
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

use std::error::Error as StdError;

use axum::Json;
use axum::extract::rejection::{FormRejection, JsonRejection, PathRejection, QueryRejection};
use axum::http::StatusCode;
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
        let mut parts = Vec::new();
        let mut current = StdError::source(self);
        while let Some(error) = current {
            parts.push(error.to_string());
            current = error.source();
        }
        parts.join(": ")
    }

    /// Emits the operator-facing half of this error, at the level its
    /// [`Severity`] maps to.
    ///
    /// `tracing` has four levels and [`Severity`] has four values, but
    /// `Error` and `Page` both map to `ERROR` — so a `Page` additionally
    /// carries `alert = true`, which is what an alerting rule selects on. A
    /// level alone could not express the difference, and losing it would
    /// mean either paging on every `DbError` or never paging at all.
    fn log(&self) {
        let category = self.category();
        let chain = self.source_chain();
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
            Self::IdempotencyKeyReused { .. } => Category::Idempotency,
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
            Self::UnknownRoute { .. }
            | Self::InvalidParam { .. }
            | Self::IdempotencyKeyReused { .. }
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

        (status, Json(body)).into_response()
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
    use std::io;
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::body::Body;
    use axum::extract::{Form, Path, Query};
    use axum::http::Request;
    use axum::routing::get;
    use axum::routing::post;
    use serde_json::Value;
    use tower::ServiceExt as _;
    use tracing_subscriber::fmt::MakeWriter;
    use vpay_core::MoneyError;
    use vpay_core::money::UnknownCurrency;
    use vpay_db::DbError;
    use vpay_provider::ProviderError;

    use super::*;

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
                || ApiError::Provider(ProviderError::Transport("connect timed out".into())),
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
            (
                || ApiError::Internal("the ledger did not balance".into()),
                500,
                "api_error",
                "internal_error",
            ),
        ]
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

    /// Captures `tracing` output for the duration of one closure, so a test
    /// can assert on what an operator would see. Scoped with `with_default`
    /// (thread-local) rather than a global subscriber, so it holds whether
    /// the suite runs process-per-test under nextest or threaded under
    /// `cargo test`.
    #[derive(Clone, Default)]
    struct CapturedLog(Arc<Mutex<Vec<u8>>>);

    impl io::Write for CapturedLog {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            match self.0.lock() {
                Ok(mut sink) => sink.write(buf),
                Err(poisoned) => poisoned.into_inner().write(buf),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CapturedLog {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn with_captured_log<T>(f: impl FnOnce() -> T) -> (T, String) {
        let sink = CapturedLog::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(sink.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let out = tracing::subscriber::with_default(subscriber, f);
        let captured = sink.0.lock().map_or_else(
            |poisoned| String::from_utf8_lossy(&poisoned.into_inner()).into_owned(),
            |bytes| String::from_utf8_lossy(&bytes).into_owned(),
        );
        (out, captured)
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
            ApiError::Provider(ProviderError::Transport("timeout".into())).log();
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

    #[tokio::test]
    async fn the_404_fallback_is_byte_for_byte_what_it_was_before_api_error() {
        let pool = vpay_db::PgPool::connect_lazy("postgres://vpay:vpay@localhost:5432/vpay")
            .expect("connect_lazy performs no I/O and only fails on a malformed URL");
        let response = crate::router(pool)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/payment_intents")
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
}
