//! The cross-cutting error classification every vpay error implements.
//!
//! [`Classify`] is the seam: one decision — the [`Category`] — and every
//! boundary's answer follows from it, so the HTTP envelope, the worker's
//! retry decision, the log severity and a binary's exit code are derived
//! rather than chosen at a call site.
//!
//! [ADR-0011](../../../../docs/adr/0011-error-modelling.md) is the decision
//! and `docs/flows/errors.md` the policy table. The three tiers, the five
//! questions a boundary has to answer and why this crate returns plain data
//! rather than framework types:
//! [docs/reference/vpay-core.md § error](../../../../docs/reference/vpay-core.md#error).
//!
//! ```
//! use vpay_core::{Category, Classify, Retry, Severity};
//!
//! #[derive(Debug, thiserror::Error)]
//! #[error("no signing key for merchant {0}")]
//! struct NoSigningKey(String);
//!
//! // The only required method.
//! impl Classify for NoSigningKey {
//!     fn category(&self) -> Category {
//!         Category::Configuration
//!     }
//! }
//!
//! let error = NoSigningKey("acct_1".to_owned());
//! assert_eq!(error.category().http_status(), 500);
//! assert_eq!(error.code(), "misconfigured");
//! assert_eq!(error.retry(), Retry::Never);
//! assert_eq!(error.severity(), Severity::Error);
//! assert_eq!(error.category().exit_code(), 78); // EX_CONFIG
//! // A merchant is told the category's generic sentence, never the Display.
//! assert_eq!(
//!     error.public_message(),
//!     "vpay is misconfigured for this operation. Contact support."
//! );
//! ```

use std::error::Error as StdError;

/// Whose problem an error is, and therefore how every boundary treats it.
///
/// Deliberately coarse: a category decides *policy*. The detail a merchant
/// needs lives in [`Classify::code`] and [`Classify::public_message`], the
/// detail an operator needs in the error's own `Display`/`source` chain.
/// Adding a variant is an ADR-level change — the `match`es below are
/// exhaustive on purpose, so every boundary is forced to decide.
///
/// ```
/// use vpay_core::{Category, Retry, Severity};
///
/// // A merchant's mistake: 4xx, never retried, logged at Info, and the
/// // caller is told what to fix.
/// assert_eq!(Category::InvalidRequest.http_status(), 400);
/// assert_eq!(Category::InvalidRequest.default_retry(), Retry::Never);
/// assert_eq!(Category::InvalidRequest.default_severity(), Severity::Info);
///
/// // A rail that would not answer: 502, retried by the poll ladder, and
/// // `EX_UNAVAILABLE` if a binary hits it at startup.
/// assert_eq!(Category::Rail.http_status(), 502);
/// assert_eq!(Category::Rail.default_retry(), Retry::AfterBackoff);
/// assert_eq!(Category::Rail.exit_code(), 69);
///
/// // Only an invariant violation pages, and it says nothing to the caller.
/// assert_eq!(Category::Internal.default_severity(), Severity::Page);
/// assert_eq!(
///     Category::Internal.generic_message(),
///     "An internal error occurred. Contact support with the request id."
/// );
///
/// // Stripe's `type` vocabulary is closed, so several categories share one.
/// assert_eq!(Category::NotFound.stripe_type(), "invalid_request_error");
/// assert_eq!(Category::Storage.stripe_type(), "api_error");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// The caller's input is wrong: a malformed field, an amount that is
    /// not an integer, a currency that does not exist. Fix the request.
    InvalidRequest,
    /// The caller is not who they claim, or presented no credential. Every
    /// authentication failure collapses here regardless of *which* check
    /// tripped — `vpay_api::resource_auth` explains why the response must
    /// not be an oracle.
    Authentication,
    /// The caller is authenticated but not allowed: a disabled client, a
    /// missing scope, a merchant touching another merchant's object.
    Forbidden,
    /// The object named in the request does not exist for this caller.
    NotFound,
    /// The request is well-formed but the object's state forbids it —
    /// cancelling an intent that is already `processing`, refunding on a
    /// rail whose capabilities say `supports_refunds: false`.
    Conflict,
    /// An `Idempotency-Key` was reused with a different request body.
    Idempotency,
    /// The caller is sending too fast. Retry after backoff.
    RateLimited,
    /// A payment rail could not be reached or answered incoherently. Not
    /// the payer's fault and not the merchant's; retried by the worker's
    /// poll ladder, never by a merchant re-submitting. A rail *rejecting*
    /// a charge is **not** this category — that is a [`FailureCode`] on the
    /// charge, a business outcome rather than a system error.
    ///
    /// [`FailureCode`]: crate::FailureCode
    Rail,
    /// Postgres could not be reached, a query failed, a migration broke.
    /// Retryable in the sense that the *next* request may succeed; the
    /// request that hit it must fail rather than guess.
    Storage,
    /// The deployment is misconfigured: an unresolved `${ENV}`, an `http`
    /// rail host in livemode, a client with no keys. An operator's problem,
    /// fixed by a deploy, never by retrying.
    Configuration,
    /// Honest stub: the code path exists but is not built yet
    /// (`ProviderError::NotImplemented`, tracked by `cargo xtask
    /// verify-status`). Never a success, never retryable.
    NotImplemented,
    /// A bug: an invariant this code guarantees was violated (a ledger
    /// that does not balance, a state transition the type system should
    /// have prevented). Page someone.
    Internal,
}

/// Who may retry, and how. Derived from the category by default; a leaf
/// error overrides it only when it knows better (e.g. a rail timeout on a
/// push rail after the payer may already have acted — see
/// `docs/flows/crash-safety.md` — must not be retried blindly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Retry {
    /// Retrying the same operation cannot succeed until something else
    /// changes (a fix, a deploy, a different request).
    Never,
    /// The same operation, unchanged, may succeed after a delay. This is
    /// the worker's poll-ladder case and a merchant's "try again shortly".
    AfterBackoff,
    /// The operation must not be repeated as-is; the caller has to start
    /// over with a *new* attempt (a new `PaymentIntent`, per
    /// `docs/flows/payment-lifecycle.md`: retry means a new intent).
    NewAttempt,
}

/// How loudly a boundary should log the error. `tracing` levels are the
/// obvious mapping, but this crate does not depend on `tracing`, so the
/// translation happens at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    /// Expected in normal operation: a merchant's typo, an expired token.
    /// Counted, not investigated.
    Info,
    /// Something degraded but self-healing: a rail timeout, a rate limit.
    Warn,
    /// Something that needs a human eventually: a DB failure, a config
    /// error at runtime.
    Error,
    /// Something that needs a human *now*: an invariant violation in the
    /// money path. Maps to an alert, not just a log line.
    Page,
}

impl Severity {
    /// The string this severity is spelled with on a metric label — the
    /// `Debug` spelling, for the reason [`Category::as_metric_label`] gives.
    ///
    /// `Error` and `Page` are two *labels* on one `tracing` level: the level
    /// cannot tell them apart, which is why `vpay_alert_events_total` exists
    /// as a separate counter rather than as a level filter.
    ///
    /// ```
    /// use vpay_core::Severity;
    ///
    /// assert_eq!(Severity::Page.as_metric_label(), "Page");
    /// assert_eq!(format!("{:?}", Severity::Page), Severity::Page.as_metric_label());
    /// ```
    #[must_use]
    pub const fn as_metric_label(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Warn => "Warn",
            Self::Error => "Error",
            Self::Page => "Page",
        }
    }
}

impl Category {
    /// The HTTP status a boundary answers with. Stripe's own mapping where
    /// Stripe has one (`docs/api/README.md`: the error envelope is
    /// Stripe-shaped), and the RFC 9110 status closest in meaning where it
    /// does not.
    #[must_use]
    pub const fn http_status(self) -> u16 {
        match self {
            Self::InvalidRequest | Self::Idempotency => 400,
            Self::Authentication => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::Conflict => 409,
            Self::RateLimited => 429,
            Self::Internal | Self::Configuration => 500,
            Self::NotImplemented => 501,
            Self::Rail => 502,
            Self::Storage => 503,
        }
    }

    /// The Stripe-shaped `error.type` for the envelope. Stripe's vocabulary
    /// is closed (`api_error`, `authentication_error`, `idempotency_error`,
    /// `invalid_request_error`, `rate_limit_error`), so several categories
    /// share a type and are told apart by [`Classify::code`] and the status.
    #[must_use]
    pub const fn stripe_type(self) -> &'static str {
        match self {
            Self::InvalidRequest | Self::Forbidden | Self::NotFound | Self::Conflict => {
                "invalid_request_error"
            }
            Self::Authentication => "authentication_error",
            Self::Idempotency => "idempotency_error",
            Self::RateLimited => "rate_limit_error",
            Self::Rail
            | Self::Storage
            | Self::Configuration
            | Self::NotImplemented
            | Self::Internal => "api_error",
        }
    }

    /// The `error.code` used when a leaf error does not name a more
    /// specific one. Stripe's own codes where they exist
    /// (`resource_missing`, `rate_limit`, `idempotency_key_in_use`).
    #[must_use]
    pub const fn default_code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Authentication => "invalid_token",
            Self::Forbidden => "forbidden",
            Self::NotFound => "resource_missing",
            Self::Conflict => "invalid_state",
            Self::Idempotency => "idempotency_key_in_use",
            Self::RateLimited => "rate_limit",
            Self::Rail => "provider_unavailable",
            Self::Storage => "service_unavailable",
            Self::Configuration => "misconfigured",
            Self::NotImplemented => "not_implemented",
            Self::Internal => "internal_error",
        }
    }

    /// The default retry policy. Only [`Category::Rail`],
    /// [`Category::Storage`] and [`Category::RateLimited`] are retryable
    /// as-is; everything else needs a changed input or a fix.
    #[must_use]
    pub const fn default_retry(self) -> Retry {
        match self {
            Self::Rail | Self::Storage | Self::RateLimited => Retry::AfterBackoff,
            Self::InvalidRequest
            | Self::Authentication
            | Self::Forbidden
            | Self::NotFound
            | Self::Conflict
            | Self::Idempotency
            | Self::Configuration
            | Self::NotImplemented
            | Self::Internal => Retry::Never,
        }
    }

    /// The default log severity. Caller errors are `Info`: they are the
    /// merchant's to fix and a payment gateway sees thousands a day.
    #[must_use]
    pub const fn default_severity(self) -> Severity {
        match self {
            Self::InvalidRequest
            | Self::Authentication
            | Self::Forbidden
            | Self::NotFound
            | Self::Conflict
            | Self::Idempotency => Severity::Info,
            Self::RateLimited | Self::Rail => Severity::Warn,
            Self::Storage | Self::Configuration | Self::NotImplemented => Severity::Error,
            Self::Internal => Severity::Page,
        }
    }

    /// The string this category is spelled with on a metric label — the
    /// `Debug` spelling, so an alert's label and the JSON log line that
    /// produced it are joinable by eye. See
    /// [docs/reference/vpay-core.md § metric labels are the Debug spelling](../../../../docs/reference/vpay-core.md#metric-labels-are-the-debug-spelling).
    ///
    /// ```
    /// use vpay_core::Category;
    ///
    /// for category in Category::ALL {
    ///     assert_eq!(format!("{category:?}"), category.as_metric_label());
    /// }
    /// ```
    #[must_use]
    pub const fn as_metric_label(self) -> &'static str {
        match self {
            Self::InvalidRequest => "InvalidRequest",
            Self::Authentication => "Authentication",
            Self::Forbidden => "Forbidden",
            Self::NotFound => "NotFound",
            Self::Conflict => "Conflict",
            Self::Idempotency => "Idempotency",
            Self::RateLimited => "RateLimited",
            Self::Rail => "Rail",
            Self::Storage => "Storage",
            Self::Configuration => "Configuration",
            Self::NotImplemented => "NotImplemented",
            Self::Internal => "Internal",
        }
    }

    /// A message safe to show any caller. Names nothing internal — no
    /// table, no host, no library error text. Leaf errors in the caller's
    /// own categories (`InvalidRequest`, `Conflict`, ...) usually override
    /// [`Classify::public_message`] with the specific field or state; the
    /// system categories never should.
    #[must_use]
    pub const fn generic_message(self) -> &'static str {
        match self {
            Self::InvalidRequest => "The request was malformed or a parameter was invalid.",
            Self::Authentication => {
                "The bearer token is invalid, expired, or was not issued for this endpoint."
            }
            Self::Forbidden => "This client is not permitted to perform that action.",
            Self::NotFound => "No such object exists for this client.",
            Self::Conflict => "The object is in a state that does not allow this action.",
            Self::Idempotency => {
                "This Idempotency-Key was already used with a different request body."
            }
            Self::RateLimited => "Too many requests. Retry after a short delay.",
            Self::Rail => {
                "The payment rail is temporarily unavailable. The charge will be retried."
            }
            Self::Storage => "vpay is temporarily unavailable. Retry after a short delay.",
            Self::Configuration => "vpay is misconfigured for this operation. Contact support.",
            Self::NotImplemented => "This operation is not implemented yet.",
            Self::Internal => "An internal error occurred. Contact support with the request id.",
        }
    }

    /// Process exit code for a binary that fails at startup with this
    /// category — `sysexits.h` where it has a fitting code, so a supervisor
    /// or a human can tell "fix the YAML" (78) from "Postgres is down" (69)
    /// without parsing logs. Everything else is a plain `1`.
    #[must_use]
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Configuration => 78,                      // EX_CONFIG
            Self::Storage | Self::Rail => 69,               // EX_UNAVAILABLE
            Self::InvalidRequest | Self::Idempotency => 64, // EX_USAGE
            Self::Authentication | Self::Forbidden => 77,   // EX_NOPERM
            Self::NotFound
            | Self::Conflict
            | Self::RateLimited
            | Self::NotImplemented
            | Self::Internal => 1,
        }
    }

    /// Every category, for exhaustive tests and documentation generators.
    pub const ALL: [Category; 12] = [
        Self::InvalidRequest,
        Self::Authentication,
        Self::Forbidden,
        Self::NotFound,
        Self::Conflict,
        Self::Idempotency,
        Self::RateLimited,
        Self::Rail,
        Self::Storage,
        Self::Configuration,
        Self::NotImplemented,
        Self::Internal,
    ];
}

/// The classification seam. Implemented by every error enum in
/// `backends/crates` (`cargo xtask verify-errors` fails the build if one is
/// missing), and by nothing outside them — the SDKs model the *wire*, not
/// the system, and the binaries consume this rather than implement it.
///
/// Only [`Classify::category`] is required. The defaults derive everything
/// else from the category; an override is a deliberate statement that the
/// default is wrong for that variant, and should say why in a comment.
///
/// ```
/// use vpay_core::{Category, Classify};
///
/// #[derive(Debug, thiserror::Error)]
/// #[error("amount must not be negative, got {0}")]
/// struct Negative(i64);
///
/// impl Classify for Negative {
///     fn category(&self) -> Category {
///         Category::InvalidRequest
///     }
///
///     // Overridden: the caller's own value *is* the useful information,
///     // and echoing it leaks nothing internal.
///     fn code(&self) -> &'static str {
///         "amount_negative"
///     }
///
///     fn public_message(&self) -> String {
///         self.to_string()
///     }
/// }
///
/// let error = Negative(-1);
/// assert_eq!(error.code(), "amount_negative");
/// assert_eq!(error.public_message(), "amount must not be negative, got -1");
/// // Everything not overridden still comes from the category.
/// assert_eq!(error.severity(), Category::InvalidRequest.default_severity());
/// ```
pub trait Classify: StdError {
    /// Whose problem this is. Decides status, retry, severity and exit code
    /// unless overridden below.
    fn category(&self) -> Category;

    /// The Stripe-shaped `error.code`: a stable, snake_case identifier a
    /// merchant can branch on (`docs/flows/failures.md` is the model: a
    /// closed vocabulary that does not grow when a rail is added). Defaults
    /// to the category's own code.
    fn code(&self) -> &'static str {
        self.category().default_code()
    }

    /// Whether and how the failed operation may be retried.
    fn retry(&self) -> Retry {
        self.category().default_retry()
    }

    /// How loudly to log it.
    fn severity(&self) -> Severity {
        self.category().default_severity()
    }

    /// The message a caller may see. **Must not** leak anything from the
    /// `source` chain — a library's error text names hosts, tables, key
    /// ids and file paths. The default is the category's generic sentence;
    /// override it only for caller-category errors where the specific
    /// field or state *is* the useful information.
    fn public_message(&self) -> String {
        self.category().generic_message().to_owned()
    }
}

/// Every link *below* `error`, joined with `": "`.
///
/// The half of an error that names the concrete cause and the half a
/// `Display` alone throws away. `vpay_api::ApiError::log` and
/// `vpay_worker`'s job settlement both emit it beside the `Display`, so an
/// operator reading either sees the same two lines and `jobs.last_error`
/// keeps the leaf.
///
/// Empty when `error` has no source. It walks `Error::source`, so a leaf that
/// flattens its cause into its own message contributes nothing here — which
/// is the point: this function is what makes `#[source]` worth carrying.
///
/// ```
/// use vpay_core::error::source_chain;
///
/// #[derive(Debug, thiserror::Error)]
/// #[error("the rail refused the connection")]
/// struct Transport;
///
/// #[derive(Debug, thiserror::Error)]
/// #[error("submit failed")]
/// struct Submit(#[source] Transport);
///
/// // `Display` names only the top of the chain…
/// assert_eq!(Submit(Transport).to_string(), "submit failed");
/// // …and this is the rest of it.
/// assert_eq!(
///     source_chain(&Submit(Transport)),
///     "the rail refused the connection"
/// );
/// // A leaf with nothing under it contributes nothing.
/// assert_eq!(source_chain(&Transport), "");
/// ```
#[must_use]
pub fn source_chain(error: &dyn StdError) -> String {
    let mut parts = Vec::new();
    let mut current = error.source();
    while let Some(link) = current {
        parts.push(link.to_string());
        current = link.source();
    }
    parts.join(": ")
}

/// `error` and every link below it, joined with `": "` — the whole failure as
/// one line for an operator.
///
/// The rendering a durable `last_error`-shaped column wants. [`source_chain`]
/// alone throws away the top of the chain and `Display` alone throws away the
/// bottom, and a caller that writes only one of them has recorded a failure
/// that names either the operation or the cause but never both.
///
/// Two write sites use it today and they are the reason it is here rather than
/// inlined at each: `jobs.last_error` (`vpay_worker::run_loop`) and
/// `webhook_deliveries.response_excerpt` for a delivery that got no answer
/// (`vpay_worker::webhooks`). Neither truncates here — both columns carry a
/// `char_length` CHECK and `vpay_db` bounds the value against it at the write,
/// which is the only layer that knows the ceiling.
///
/// ```
/// use vpay_core::error::display_with_chain;
///
/// #[derive(Debug, thiserror::Error)]
/// #[error("connection refused")]
/// struct Refused;
///
/// #[derive(Debug, thiserror::Error)]
/// #[error("the request to the rail failed")]
/// struct Transport(#[source] Refused);
///
/// // Both halves, in that order.
/// assert_eq!(
///     display_with_chain(&Transport(Refused)),
///     "the request to the rail failed: connection refused"
/// );
/// // A leaf with nothing under it renders exactly as its `Display`, with no
/// // trailing separator.
/// assert_eq!(display_with_chain(&Refused), "connection refused");
/// ```
#[must_use]
pub fn display_with_chain(error: &dyn StdError) -> String {
    let chain = source_chain(error);
    if chain.is_empty() {
        error.to_string()
    } else {
        format!("{error}: {chain}")
    }
}

/// Finds the first [`Classify`] implementor of type `T` anywhere in an error
/// chain — the tool a boundary uses to classify an `anyhow::Error` (whose
/// `chain()` yields exactly this iterator) without depending on `anyhow`
/// here.
///
/// Typed rather than dynamic on purpose: `dyn Error` cannot be downcast to
/// `dyn Classify`, so a binary names the leaf types it knows how to classify
/// in order of specificity and falls back to [`Category::Internal`] for
/// anything else — which pages, the honest outcome for an unclassified
/// startup failure in a payment binary.
///
/// ```
/// use std::error::Error as StdError;
///
/// use vpay_core::{Category, Classify, error::find_in_chain};
///
/// #[derive(Debug, thiserror::Error)]
/// #[error("no such file: vpay.yaml")]
/// struct MissingConfig;
///
/// impl Classify for MissingConfig {
///     fn category(&self) -> Category {
///         Category::Configuration
///     }
/// }
///
/// #[derive(Debug, thiserror::Error)]
/// #[error("while loading configuration")]
/// struct WhileLoading(#[source] MissingConfig);
///
/// // What `anyhow::Error::chain()` yields: the error, then its sources.
/// let outer = WhileLoading(MissingConfig);
/// let mut chain: Vec<&(dyn StdError + 'static)> = vec![&outer];
/// let mut current: &(dyn StdError + 'static) = &outer;
/// while let Some(source) = current.source() {
///     chain.push(source);
///     current = source;
/// }
///
/// let found = find_in_chain::<MissingConfig>(chain.iter().copied());
/// assert_eq!(found.map(Classify::category), Some(Category::Configuration));
/// // …so the binary exits 78 (EX_CONFIG) rather than a bare 1.
/// assert_eq!(found.map(|e| e.category().exit_code()), Some(78));
/// ```
#[must_use]
pub fn find_in_chain<'a, T: Classify + 'static>(
    chain: impl IntoIterator<Item = &'a (dyn StdError + 'static)>,
) -> Option<&'a T> {
    chain.into_iter().find_map(|e| e.downcast_ref::<T>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_has_a_status_in_the_4xx_or_5xx_range() {
        for c in Category::ALL {
            let s = c.http_status();
            assert!((400..600).contains(&s), "{c:?} maps to {s}");
        }
    }

    #[test]
    fn caller_categories_are_4xx_and_system_categories_are_5xx() {
        let caller = [
            Category::InvalidRequest,
            Category::Authentication,
            Category::Forbidden,
            Category::NotFound,
            Category::Conflict,
            Category::Idempotency,
            Category::RateLimited,
        ];
        for c in Category::ALL {
            let is_caller = caller.contains(&c);
            assert_eq!(
                c.http_status() < 500,
                is_caller,
                "{c:?}: a caller's error is 4xx and a system error is 5xx, never the other way"
            );
        }
    }

    #[test]
    fn stripe_types_are_from_stripes_closed_vocabulary() {
        const STRIPE: [&str; 5] = [
            "api_error",
            "authentication_error",
            "idempotency_error",
            "invalid_request_error",
            "rate_limit_error",
        ];
        for c in Category::ALL {
            assert!(
                STRIPE.contains(&c.stripe_type()),
                "{c:?} → {}",
                c.stripe_type()
            );
        }
    }

    #[test]
    fn only_transient_categories_default_to_retry_after_backoff() {
        for c in Category::ALL {
            let transient = matches!(
                c,
                Category::Rail | Category::Storage | Category::RateLimited
            );
            assert_eq!(c.default_retry() == Retry::AfterBackoff, transient, "{c:?}");
        }
    }

    #[test]
    fn only_internal_pages_and_caller_errors_never_exceed_info() {
        assert_eq!(Category::Internal.default_severity(), Severity::Page);
        for c in Category::ALL {
            if c.http_status() < 429 {
                assert_eq!(c.default_severity(), Severity::Info, "{c:?}");
            }
            if c != Category::Internal {
                assert!(c.default_severity() < Severity::Page, "{c:?} must not page");
            }
        }
    }

    #[test]
    fn generic_messages_name_nothing_internal() {
        for c in Category::ALL {
            let m = c.generic_message();
            for forbidden in [
                "postgres",
                "sqlx",
                "table",
                "http://",
                "authkestra",
                "panic",
            ] {
                assert!(
                    !m.to_ascii_lowercase().contains(forbidden),
                    "{c:?}'s public message leaks `{forbidden}`: {m}"
                );
            }
            assert!(
                m.ends_with('.'),
                "{c:?}: a public message is a sentence: {m}"
            );
        }
    }

    #[test]
    fn exit_codes_follow_sysexits_where_one_fits() {
        assert_eq!(Category::Configuration.exit_code(), 78);
        assert_eq!(Category::Storage.exit_code(), 69);
        assert_eq!(Category::Internal.exit_code(), 1);
        for c in Category::ALL {
            assert!((1..=78).contains(&c.exit_code()), "{c:?}");
        }
    }

    /// The policy table of `docs/flows/errors.md`, transcribed by hand.
    ///
    /// Every other test in this module asserts an *invariant* ("caller
    /// categories are 4xx", "only `Internal` pages") — which is worth more
    /// per line, but which a wrong-but-self-consistent table would satisfy.
    /// This one is the literal row-for-row transcription, so the document
    /// and the code fail together: if a column here disagrees with the
    /// document, one of the two is wrong and someone has to decide which.
    ///
    /// Columns are exactly the document's, in its order, with one addition:
    /// the document's table has no message column, so `generic_message` is
    /// pinned from the implementation instead. That is still worth pinning
    /// — these sentences cross the wire to merchants — but it is a
    /// regression test rather than a transcription, and a deliberate
    /// re-wording updates it.
    /// One row of the document's table. A struct rather than a tuple so the
    /// columns are named where they are written, and so a reordering of two
    /// same-typed columns cannot go unnoticed.
    struct PolicyRow {
        category: Category,
        http: u16,
        stripe_type: &'static str,
        default_code: &'static str,
        retry: Retry,
        severity: Severity,
        exit: i32,
        message: &'static str,
    }

    const POLICY_TABLE: [PolicyRow; 12] = [
        PolicyRow {
            category: Category::InvalidRequest,
            http: 400,
            stripe_type: "invalid_request_error",
            default_code: "invalid_request",
            retry: Retry::Never,
            severity: Severity::Info,
            exit: 64,
            message: "The request was malformed or a parameter was invalid.",
        },
        PolicyRow {
            category: Category::Authentication,
            http: 401,
            stripe_type: "authentication_error",
            default_code: "invalid_token",
            retry: Retry::Never,
            severity: Severity::Info,
            exit: 77,
            message: "The bearer token is invalid, expired, or was not issued for this endpoint.",
        },
        PolicyRow {
            category: Category::Forbidden,
            http: 403,
            stripe_type: "invalid_request_error",
            default_code: "forbidden",
            retry: Retry::Never,
            severity: Severity::Info,
            exit: 77,
            message: "This client is not permitted to perform that action.",
        },
        PolicyRow {
            category: Category::NotFound,
            http: 404,
            stripe_type: "invalid_request_error",
            default_code: "resource_missing",
            retry: Retry::Never,
            severity: Severity::Info,
            exit: 1,
            message: "No such object exists for this client.",
        },
        PolicyRow {
            category: Category::Conflict,
            http: 409,
            stripe_type: "invalid_request_error",
            default_code: "invalid_state",
            retry: Retry::Never,
            severity: Severity::Info,
            exit: 1,
            message: "The object is in a state that does not allow this action.",
        },
        PolicyRow {
            category: Category::Idempotency,
            http: 400,
            stripe_type: "idempotency_error",
            default_code: "idempotency_key_in_use",
            retry: Retry::Never,
            severity: Severity::Info,
            exit: 64,
            message: "This Idempotency-Key was already used with a different request body.",
        },
        PolicyRow {
            category: Category::RateLimited,
            http: 429,
            stripe_type: "rate_limit_error",
            default_code: "rate_limit",
            retry: Retry::AfterBackoff,
            severity: Severity::Warn,
            exit: 1,
            message: "Too many requests. Retry after a short delay.",
        },
        PolicyRow {
            category: Category::Rail,
            http: 502,
            stripe_type: "api_error",
            default_code: "provider_unavailable",
            retry: Retry::AfterBackoff,
            severity: Severity::Warn,
            exit: 69,
            message: "The payment rail is temporarily unavailable. The charge will be retried.",
        },
        PolicyRow {
            category: Category::Storage,
            http: 503,
            stripe_type: "api_error",
            default_code: "service_unavailable",
            retry: Retry::AfterBackoff,
            severity: Severity::Error,
            exit: 69,
            message: "vpay is temporarily unavailable. Retry after a short delay.",
        },
        PolicyRow {
            category: Category::Configuration,
            http: 500,
            stripe_type: "api_error",
            default_code: "misconfigured",
            retry: Retry::Never,
            severity: Severity::Error,
            exit: 78,
            message: "vpay is misconfigured for this operation. Contact support.",
        },
        PolicyRow {
            category: Category::NotImplemented,
            http: 501,
            stripe_type: "api_error",
            default_code: "not_implemented",
            retry: Retry::Never,
            severity: Severity::Error,
            exit: 1,
            message: "This operation is not implemented yet.",
        },
        PolicyRow {
            category: Category::Internal,
            http: 500,
            stripe_type: "api_error",
            default_code: "internal_error",
            retry: Retry::Never,
            severity: Severity::Page,
            exit: 1,
            message: "An internal error occurred. Contact support with the request id.",
        },
    ];

    #[test]
    fn every_category_matches_the_policy_table_in_docs_flows_errors_md() {
        for row in POLICY_TABLE {
            let c = row.category;
            assert_eq!(c.http_status(), row.http, "{c:?}: HTTP");
            assert_eq!(c.stripe_type(), row.stripe_type, "{c:?}: stripe type");
            assert_eq!(c.default_code(), row.default_code, "{c:?}: default code");
            assert_eq!(c.default_retry(), row.retry, "{c:?}: retry");
            assert_eq!(c.default_severity(), row.severity, "{c:?}: severity");
            assert_eq!(c.exit_code(), row.exit, "{c:?}: exit code");
            assert_eq!(c.generic_message(), row.message, "{c:?}: message");
        }
    }

    /// A row per category is only a full transcription if the table has a
    /// row for *every* category, and `ALL` is only usable as "every
    /// category" if it actually is.
    #[test]
    fn the_policy_table_covers_every_category_exactly_once() {
        assert_eq!(POLICY_TABLE.len(), Category::ALL.len());
        for c in Category::ALL {
            assert_eq!(
                POLICY_TABLE.iter().filter(|row| row.category == c).count(),
                1,
                "{c:?} is not in the transcribed table exactly once"
            );
        }
    }

    /// A dense index over [`Category`], deliberately without a wildcard arm.
    ///
    /// This is the half of the completeness check the compiler enforces: a
    /// thirteenth variant fails to compile *here*, before any test runs. The
    /// test below is the other half — it catches a thirteenth variant that
    /// was given an index but left out of [`Category::ALL`], which nothing
    /// else in this crate would notice, since every other test iterates
    /// `ALL` and would simply never see it.
    const fn index(c: Category) -> usize {
        match c {
            Category::InvalidRequest => 0,
            Category::Authentication => 1,
            Category::Forbidden => 2,
            Category::NotFound => 3,
            Category::Conflict => 4,
            Category::Idempotency => 5,
            Category::RateLimited => 6,
            Category::Rail => 7,
            Category::Storage => 8,
            Category::Configuration => 9,
            Category::NotImplemented => 10,
            Category::Internal => 11,
        }
    }

    /// The highest index [`index`] can return, plus one. Bumped in the same
    /// edit that adds a variant to `index`, and the test fails loudly if the
    /// two disagree.
    const CATEGORY_COUNT: usize = 12;

    #[test]
    fn all_contains_every_category_exactly_once() {
        let mut hits = [0usize; CATEGORY_COUNT];
        for c in Category::ALL {
            let i = index(c);
            assert!(i < CATEGORY_COUNT, "{c:?} indexes past the count");
            match hits.get_mut(i) {
                Some(slot) => *slot += 1,
                None => unreachable!(),
            }
        }
        for (i, hits) in hits.iter().enumerate() {
            assert_eq!(
                *hits, 1,
                "index {i} appears {hits} time(s) in Category::ALL; every category must appear exactly once"
            );
        }
        assert_eq!(Category::ALL.len(), CATEGORY_COUNT);
    }

    #[test]
    fn codes_are_snake_case_identifiers() {
        for c in Category::ALL {
            let code = c.default_code();
            assert!(
                code.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'),
                "{c:?} → {code}"
            );
        }
    }

    /// The decisive test for `vpay_error_events_total`'s `category` and
    /// `severity` labels: the metric spells them exactly as the JSON log
    /// line does.
    ///
    /// The log lines write `category = ?category` / `severity = ?severity`,
    /// so `Debug` *is* the wire format an operator greps for. Renaming a
    /// variant without touching `as_metric_label` fails here rather than
    /// producing a dashboard whose labels no log line contains.
    #[test]
    fn the_metric_label_is_the_debug_spelling() {
        for c in Category::ALL {
            assert_eq!(format!("{c:?}"), c.as_metric_label());
        }
        for s in [
            Severity::Info,
            Severity::Warn,
            Severity::Error,
            Severity::Page,
        ] {
            assert_eq!(format!("{s:?}"), s.as_metric_label());
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("leaf: {0}")]
    struct Leaf(&'static str);

    impl Classify for Leaf {
        fn category(&self) -> Category {
            Category::Configuration
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("wrapper")]
    struct Wrapper(#[source] Leaf);

    #[test]
    fn defaults_derive_everything_from_the_category() {
        let e = Leaf("x");
        assert_eq!(e.code(), "misconfigured");
        assert_eq!(e.retry(), Retry::Never);
        assert_eq!(e.severity(), Severity::Error);
        assert_eq!(
            e.public_message(),
            Category::Configuration.generic_message()
        );
    }

    #[test]
    fn find_in_chain_walks_sources_and_downcasts_by_type() {
        let outer = Wrapper(Leaf("inner"));
        let chain: Vec<&(dyn StdError + 'static)> = {
            let mut v: Vec<&(dyn StdError + 'static)> = vec![&outer];
            let mut cur: &(dyn StdError + 'static) = &outer;
            while let Some(s) = cur.source() {
                v.push(s);
                cur = s;
            }
            v
        };
        let found = find_in_chain::<Leaf>(chain.iter().copied()).expect("leaf is in the chain");
        assert_eq!(found.0, "inner");
        // The wrapper itself does not implement `Classify`, so a boundary
        // asking for the wrapper's type cannot even express the question —
        // `find_in_chain` is typed to `Classify` implementors on purpose.
    }
}
