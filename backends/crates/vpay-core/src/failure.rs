//! The canonical failure taxonomy.
//!
//! This vocabulary is closed and owned by the core. Every adapter maps its
//! rail's error strings into it; merchants integrate against this list once and
//! it does not grow when a rail is added. See `docs/flows/failures.md`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    InsufficientFunds,
    PayerTimeout,
    PayerDeclined,
    InvalidPayer,
    PayerLimitReached,
    PayerAccountBlocked,
    InvalidPayee,
    PayeeAccountBlocked,
    /// *Your* partner account is blocked. Page yourself.
    ProviderAccountBlocked,
    ProviderUnavailable,
    /// Unmapped. Always accompanied by the raw reason. A rising rate of this
    /// means an adapter's mapping table has drifted behind the rail — alert on
    /// it, do not tolerate it.
    ProviderError,
}

impl FailureCode {
    /// Whether the payer could plausibly succeed on a fresh PaymentIntent.
    #[must_use]
    pub const fn payer_actionable(self) -> bool {
        matches!(
            self,
            Self::InsufficientFunds
                | Self::PayerTimeout
                | Self::PayerDeclined
                | Self::PayerLimitReached
        )
    }

    /// Whether the merchant's own configuration is what needs fixing.
    #[must_use]
    pub const fn merchant_actionable(self) -> bool {
        matches!(self, Self::InvalidPayee | Self::PayeeAccountBlocked)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InsufficientFunds => "insufficient_funds",
            Self::PayerTimeout => "payer_timeout",
            Self::PayerDeclined => "payer_declined",
            Self::InvalidPayer => "invalid_payer",
            Self::PayerLimitReached => "payer_limit_reached",
            Self::PayerAccountBlocked => "payer_account_blocked",
            Self::InvalidPayee => "invalid_payee",
            Self::PayeeAccountBlocked => "payee_account_blocked",
            Self::ProviderAccountBlocked => "provider_account_blocked",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::ProviderError => "provider_error",
        }
    }
}

impl std::fmt::Display for FailureCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_error_is_never_payer_actionable() {
        assert!(!FailureCode::ProviderError.payer_actionable());
    }

    #[test]
    fn display_matches_the_wire_string() {
        assert_eq!(FailureCode::PayerTimeout.to_string(), "payer_timeout");
    }

    #[test]
    fn a_blocked_partner_account_is_nobody_elses_problem() {
        let c = FailureCode::ProviderAccountBlocked;
        assert!(!c.payer_actionable());
        assert!(!c.merchant_actionable());
    }
}
