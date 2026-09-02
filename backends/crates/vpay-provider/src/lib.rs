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
    /// Returned by an operation this rail does not support. Honest by design —
    /// the core checks [`Capabilities`] first, so reaching this is a bug.
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
            // Capabilities said no and the core asked anyway — the core
            // should have branched on the capability value first
            // (docs/adr/0002-provider-port.md), so reaching this is our bug,
            // but the *request* is what cannot proceed.
            Self::Unsupported => Category::Conflict,
            Self::NotImplemented(_) => Category::NotImplemented,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Transport(_) => "provider_unavailable",
            Self::Malformed(_) => "provider_error",
            // The merchant-facing signal for a declined charge is the
            // failure taxonomy itself (docs/flows/failures.md): the same
            // string the charge's `failure_code` column carries, so a
            // merchant sees one vocabulary whether they read the intent or
            // catch the error.
            Self::Rejected { code, .. } => code.as_str(),
            Self::Config(_) => "misconfigured",
            Self::Unsupported => "operation_unsupported_by_rail",
            Self::NotImplemented(_) => "not_implemented",
        }
    }

    fn retry(&self) -> vpay_core::Retry {
        match self {
            // "Retry means a new PaymentIntent" (docs/flows/payment-lifecycle.md).
            Self::Rejected { .. } => vpay_core::Retry::NewAttempt,
            _ => self.category().default_retry(),
        }
    }

    fn public_message(&self) -> String {
        match self {
            // The taxonomy's meaning is public by design; the rail's raw
            // reason string is not — it is logged via Display, never sent.
            Self::Rejected { code, .. } => format!("The payment was declined: {code}."),
            Self::Unsupported => "This rail does not support the requested operation.".to_owned(),
            _ => self.category().generic_message().to_owned(),
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
    use super::*;

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
