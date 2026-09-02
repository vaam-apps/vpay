//! The provider port: the single interface every payment rail implements.
//!
//! The core decides what a payment *means*; an adapter decides how to say it on
//! the wire. If `if provider == "mtn_momo"` appears anywhere outside an adapter
//! crate, the port is wrong — fix the port, not the caller.
//!
//! See `docs/adr/0002-provider-port.md` and `docs/flows/provider-port.md`.

use std::collections::BTreeMap;
use std::fmt::Debug;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vpay_core::{FailureCode, Money, ProviderFlow};

/// Static declaration of what a rail can do.
///
/// The core reads these instead of special-casing a provider code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub flow: ProviderFlow,
    pub supports_refunds: bool,
    pub supports_partial_refunds: bool,
    pub delivers_callbacks: bool,
    pub requires_ip_allowlist: bool,
}

impl Capabilities {
    /// Invariant mirrored by a CHECK constraint in the schema: see
    /// `partial_refunds_imply_refunds` in
    /// `backends/migrations/0002_create-providers.sql`, proven to fire by
    /// `partial_refunds_without_refunds_is_rejected_by_the_database` in
    /// `backends/tests/integration/tests/postgres_smoke.rs`. This Rust check
    /// runs independently — belt and braces, not a substitute for the DB
    /// constraint or vice versa.
    #[must_use]
    pub const fn is_coherent(&self) -> bool {
        !self.supports_partial_refunds || self.supports_refunds
    }
}

/// Rail-supplied key material the core must persist to query status later —
/// Orange's `pay_token`, for instance. Opaque to the core.
pub type RefExtra = BTreeMap<String, String>;

/// What a charge looks like to an adapter. Deliberately minimal.
#[derive(Debug, Clone)]
pub struct ChargeRef {
    /// The reference *we* generated, durable before any network call.
    pub reference_id: Uuid,
    pub amount: Money,
    /// Payer instrument. `None` on redirect rails, where the payer
    /// authenticates with the rail and we may never learn who they are.
    pub payer_ref: Option<String>,
    /// Rail key material captured from a previous `submit`, if any.
    pub ref_extra: RefExtra,
}

#[derive(Debug, Clone)]
pub struct Submitted {
    /// Key material the core must commit. On a redirect rail this MUST be
    /// committed before `redirect_url` is handed to anyone.
    pub ref_extra: RefExtra,
    /// Present iff the rail's flow is [`ProviderFlow::Redirect`].
    pub redirect_url: Option<String>,
}

/// The authoritative status read. Never derived from a callback body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChargeStatus {
    Pending,
    Succeeded {
        provider_txn_id: Option<String>,
    },
    Failed {
        code: FailureCode,
        raw: String,
    },
    /// The rail has no record. Never on its own grounds to fail a charge —
    /// see `docs/flows/crash-safety.md`.
    NotFound,
}

/// Identifiers extracted from a callback. Deliberately *not* a status: the core
/// will not trust anything read off an unauthenticated request.
#[derive(Debug, Clone)]
pub struct CallbackRef {
    pub reference_id: Uuid,
    /// May repair a charge whose `ref_extra` write was lost.
    pub ref_extra: RefExtra,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("transport error talking to the rail: {0}")]
    Transport(String),
    #[error("rail rejected the request: {code} — {message}")]
    Rejected { code: FailureCode, message: String },
    #[error("could not parse the rail's response: {0}")]
    Malformed(String),
    #[error("adapter configuration is invalid: {0}")]
    Config(String),
    /// Returned by an operation this rail does not support.
    ///
    /// Two facts, and they decide different columns. The *request* cannot
    /// proceed and never will on this rail, so the category is
    /// [`vpay_core::Category::Conflict`] — 409, and a merchant reading the
    /// envelope learns the truth. But the core is supposed to branch on
    /// [`Capabilities`] before it ever calls (ADR-0002), so reaching this
    /// arm at all means *our* check was skipped: the severity is therefore
    /// overridden to [`vpay_core::Severity::Error`] rather than the
    /// `Conflict` default of `Info`, so it shows up in a log an operator
    /// reads instead of being counted alongside merchants' typos.
    #[error("operation not supported by this rail")]
    Unsupported,
    /// Not yet built. This is NOT a mock: it never pretends to succeed.
    /// Every occurrence must appear in `docs/status.md`.
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),
}

impl vpay_core::Classify for ProviderError {
    fn category(&self) -> vpay_core::Category {
        use vpay_core::Category;
        match self {
            // The rail could not be reached or spoke gibberish. The worker's
            // poll ladder retries these (docs/flows/reconciler.md); nobody
            // else should, and a merchant re-submitting would risk a double
            // charge on a push rail (docs/flows/crash-safety.md).
            Self::Transport(_) | Self::Malformed(_) => Category::Rail,
            // A rail *decision*, not a rail *failure*: the charge is
            // declined, the intent goes back to requires_payment_method with
            // `last_payment_error`, and the merchant starts a new intent.
            // Classified as Conflict (the charge's state now forbids
            // retrying it) with `Retry::NewAttempt` below.
            Self::Rejected { .. } => Category::Conflict,
            Self::Config(_) => Category::Configuration,
            // The request cannot proceed on this rail, so 409 — whose *bug*
            // it is (ours) is carried by the severity override below, not by
            // the category. See the variant's own doc comment.
            Self::Unsupported => Category::Conflict,
            Self::NotImplemented(_) => Category::NotImplemented,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Transport(_) => "provider_unavailable",
            Self::Malformed(_) => "provider_error",
            // A constant, *not* the `FailureCode`'s own string. The two
            // vocabularies overlap: `FailureCode::ProviderUnavailable`
            // renders `provider_unavailable`, which is also
            // `Category::Rail`'s default code — the one `Transport` emits
            // with a 502 and "we are retrying this". A merchant branching on
            // the envelope's `code` would then see the same token for "the
            // rail is down, we will retry" (502) and "your charge was
            // declined, start a new intent" (409). One code per outcome:
            // `charge_declined` says the outcome, and the specific
            // `FailureCode` reaches the merchant through the charge's own
            // `failure_code` field (docs/flows/failures.md) and through the
            // public message below.
            Self::Rejected { .. } => "charge_declined",
            Self::Config(_) => "misconfigured",
            Self::Unsupported => "operation_unsupported_by_rail",
            Self::NotImplemented(_) => "not_implemented",
        }
    }

    fn retry(&self) -> vpay_core::Retry {
        match self {
            // "Retry means a new PaymentIntent" (docs/flows/payment-lifecycle.md).
            Self::Rejected { .. } => vpay_core::Retry::NewAttempt,
            // Exhaustive rather than a wildcard: a new variant must state
            // its retry policy here, not inherit one silently from whatever
            // category it happened to pick.
            Self::Transport(_)
            | Self::Malformed(_)
            | Self::Config(_)
            | Self::Unsupported
            | Self::NotImplemented(_) => self.category().default_retry(),
        }
    }

    fn severity(&self) -> vpay_core::Severity {
        use vpay_core::Severity;
        match self {
            // A decline is a business outcome, and `Category::Conflict`
            // would log every one of them at `Info`. Most of them should be:
            // an insufficient balance is the payer's problem and a gateway
            // sees thousands a day. But the taxonomy already carries its own
            // policy (docs/flows/failures.md), and two of its codes are not
            // about the payer at all, so the severity follows the
            // `FailureCode` rather than the category.
            Self::Rejected { code, .. } => match code {
                // "**Your** partner account is blocked … Page yourself" —
                // docs/flows/failures.md. Every charge on this rail is
                // failing and no payer can fix it.
                vpay_core::FailureCode::ProviderAccountBlocked => Severity::Page,
                // The rail is down, or the adapter's mapping table has
                // drifted behind it ("`provider_error` is an alert, not a
                // resting place" — same doc). Degraded and self-healing, or
                // degraded and needing a mapping fix: `Warn` either way.
                vpay_core::FailureCode::ProviderUnavailable
                | vpay_core::FailureCode::ProviderError => Severity::Warn,
                // Everything else is about this payer or this merchant's
                // configuration, and is the caller's to act on.
                vpay_core::FailureCode::InsufficientFunds
                | vpay_core::FailureCode::PayerTimeout
                | vpay_core::FailureCode::PayerDeclined
                | vpay_core::FailureCode::InvalidPayer
                | vpay_core::FailureCode::PayerLimitReached
                | vpay_core::FailureCode::PayerAccountBlocked
                | vpay_core::FailureCode::InvalidPayee
                | vpay_core::FailureCode::PayeeAccountBlocked => Severity::Info,
            },
            // Overrides `Conflict`'s `Info`: the 409 is honest about the
            // request, but reaching this arm means the core did not check
            // `Capabilities` first, and that is ours to fix. See the
            // variant's doc comment.
            Self::Unsupported => Severity::Error,
            Self::Transport(_) | Self::Malformed(_) | Self::Config(_) | Self::NotImplemented(_) => {
                self.category().default_severity()
            }
        }
    }

    fn public_message(&self) -> String {
        match self {
            // The taxonomy's meaning is public by design; the rail's raw
            // reason string is not — it is logged via Display, never sent.
            // The `FailureCode` moves here now that the envelope's `code` is
            // a constant, so a merchant still learns *why* without the two
            // vocabularies colliding on one field.
            Self::Rejected { code, .. } => format!("The payment was declined ({code})."),
            Self::Unsupported => "This rail does not support the requested operation.".to_owned(),
            Self::Transport(_) | Self::Malformed(_) | Self::Config(_) | Self::NotImplemented(_) => {
                self.category().generic_message().to_owned()
            }
        }
    }
}

/// Opaque per-merchant, per-rail configuration handed to the adapter.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub base_url: String,
    pub callback_url: String,
    pub currency: vpay_core::Currency,
    /// Non-secret, adapter-defined.
    pub settings: BTreeMap<String, String>,
    /// Decrypted immediately before use, adapter-defined.
    pub credentials: BTreeMap<String, String>,
}

/// Every rail implements exactly this.
///
/// Note `query_status` takes the whole [`ChargeRef`], not just an id: some
/// rails need the amount and their own token to answer.
pub trait ProviderAdapter: Debug + Send + Sync {
    /// Stable code, equal to the `payment_method_types` value.
    fn code(&self) -> &'static str;

    fn capabilities(&self) -> Capabilities;

    /// Idempotent on `charge.reference_id`. A duplicate submission MUST be
    /// reported as [`Submitted`], never as an error — that is what makes
    /// same-reference retry safe.
    fn submit(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<Submitted, ProviderError>;

    /// Must remain callable indefinitely, long after any prompt expired.
    fn query_status(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<ChargeStatus, ProviderError>;

    /// Extract identifiers only. Returning a status here is a design error.
    fn parse_callback(&self, body: &[u8]) -> Result<CallbackRef, ProviderError>;

    /// Only called when [`Capabilities::supports_refunds`] is true.
    fn refund(
        &self,
        _charge: &ChargeRef,
        _amount: Money,
        _config: &ProviderConfig,
    ) -> Result<Submitted, ProviderError> {
        Err(ProviderError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use vpay_core::{Category, Classify as _, Retry, Severity};

    use super::*;

    /// Every code in the taxonomy, so the severity table below cannot pass
    /// by only exercising the rows someone remembered. `docs/flows/failures.md`
    /// is the list; adding a code there means adding it here.
    const EVERY_FAILURE_CODE: [FailureCode; 11] = [
        FailureCode::InsufficientFunds,
        FailureCode::PayerTimeout,
        FailureCode::PayerDeclined,
        FailureCode::InvalidPayer,
        FailureCode::PayerLimitReached,
        FailureCode::PayerAccountBlocked,
        FailureCode::InvalidPayee,
        FailureCode::PayeeAccountBlocked,
        FailureCode::ProviderAccountBlocked,
        FailureCode::ProviderUnavailable,
        FailureCode::ProviderError,
    ];

    fn rejected(code: FailureCode) -> ProviderError {
        ProviderError::Rejected {
            code,
            message: "the rail's own words".to_owned(),
        }
    }

    /// The collision this constant exists to prevent: `FailureCode`'s
    /// strings and `Category`'s default codes are two vocabularies that
    /// overlap, and the envelope's `code` field can only carry one meaning.
    #[test]
    fn a_decline_has_one_code_of_its_own_and_never_borrows_the_rails() {
        for code in EVERY_FAILURE_CODE {
            assert_eq!(rejected(code).code(), "charge_declined", "{code}");
        }
        // Same `code` string, wildly different meaning: 502 "we are
        // retrying" vs. 409 "start a new intent". They must not collide.
        assert_eq!(
            ProviderError::Transport("timeout".to_owned()).code(),
            "provider_unavailable"
        );
        assert_eq!(
            ProviderError::Malformed("not json".to_owned()).code(),
            "provider_error"
        );
        assert_eq!(
            FailureCode::ProviderUnavailable.as_str(),
            "provider_unavailable",
            "if this ever stops overlapping, the constant above is still right but this test's premise changed"
        );
    }

    #[test]
    fn a_decline_names_the_failure_code_in_the_message_a_merchant_sees() {
        assert_eq!(
            rejected(FailureCode::InsufficientFunds).public_message(),
            "The payment was declined (insufficient_funds)."
        );
        // The rail's own reason string stays in `Display` (an operator's
        // half) and never crosses into the merchant's.
        let e = rejected(FailureCode::PayerDeclined);
        assert!(e.to_string().contains("the rail's own words"));
        assert!(!e.public_message().contains("the rail's own words"));
    }

    /// `Conflict` defaults to `Info`, which is right for a payer's declined
    /// charge and wrong for a blocked partner account. The taxonomy already
    /// carries that policy (`docs/flows/failures.md`); this asserts the
    /// severity follows it rather than the category.
    #[test]
    fn a_declines_severity_follows_the_failure_codes_own_policy() {
        assert_eq!(Category::Conflict.default_severity(), Severity::Info);

        // "Page yourself" — every charge on the rail is failing.
        assert_eq!(
            rejected(FailureCode::ProviderAccountBlocked).severity(),
            Severity::Page
        );
        // Degraded, or an adapter mapping that has drifted: warn.
        assert_eq!(
            rejected(FailureCode::ProviderUnavailable).severity(),
            Severity::Warn
        );
        assert_eq!(
            rejected(FailureCode::ProviderError).severity(),
            Severity::Warn
        );
        // Everything else is the payer's or the merchant's, and a gateway
        // sees thousands a day.
        for code in EVERY_FAILURE_CODE {
            let expected = match code {
                FailureCode::ProviderAccountBlocked => Severity::Page,
                FailureCode::ProviderUnavailable | FailureCode::ProviderError => Severity::Warn,
                _ => Severity::Info,
            };
            assert_eq!(rejected(code).severity(), expected, "{code}");
        }
    }

    #[test]
    fn a_decline_is_a_conflict_the_caller_must_start_over_from() {
        for code in EVERY_FAILURE_CODE {
            let e = rejected(code);
            assert_eq!(e.category(), Category::Conflict, "{code}");
            assert_eq!(e.retry(), Retry::NewAttempt, "{code}");
        }
    }

    /// 409 for the merchant, `Error` for us: the request genuinely cannot
    /// proceed, *and* the core skipped the capability check it is supposed
    /// to branch on (ADR-0002). Logging it at `Conflict`'s default `Info`
    /// would bury our own bug among merchants' typos.
    #[test]
    fn an_unsupported_operation_answers_409_but_is_logged_as_our_bug() {
        let e = ProviderError::Unsupported;
        assert_eq!(e.category(), Category::Conflict);
        assert_eq!(e.category().http_status(), 409);
        assert_eq!(Category::Conflict.default_severity(), Severity::Info);
        assert_eq!(e.severity(), Severity::Error);
        assert_eq!(e.retry(), Retry::Never);
    }

    #[test]
    fn partial_refunds_imply_refunds() {
        let bad = Capabilities {
            flow: ProviderFlow::Push,
            supports_refunds: false,
            supports_partial_refunds: true,
            delivers_callbacks: true,
            requires_ip_allowlist: false,
        };
        assert!(!bad.is_coherent());
    }
}
