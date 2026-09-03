//! The canonical failure taxonomy.
//!
//! A closed vocabulary owned by the core: every adapter maps its rail's error
//! strings into it, merchants integrate against this list once, and it does
//! not grow when a rail is added. `docs/flows/failures.md` is the flow;
//! [docs/reference/vpay-core.md § failure](../../../../docs/reference/vpay-core.md#failure)
//! says why the policy lives on the code rather than at a call site.

use serde::{Deserialize, Serialize};

/// Why a charge failed, in vpay's own words rather than a rail's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    /// The payer's balance was too low.
    InsufficientFunds,
    /// The payer never answered the prompt.
    PayerTimeout,
    /// The payer answered, and refused.
    PayerDeclined,
    /// The payer's identifier is not an account this rail knows.
    InvalidPayer,
    /// The payer is over a rail-imposed limit.
    PayerLimitReached,
    /// The payer's own account is blocked.
    PayerAccountBlocked,
    /// The *merchant's* collection account is not one this rail accepts.
    InvalidPayee,
    /// The merchant's collection account is blocked.
    PayeeAccountBlocked,
    /// *Your* partner account is blocked. Page yourself.
    ProviderAccountBlocked,
    /// The rail could not answer.
    ProviderUnavailable,
    /// Unmapped. Always accompanied by the raw reason. A rising rate of this
    /// means an adapter's mapping table has drifted behind the rail — alert
    /// on it, do not tolerate it.
    ProviderError,
}

impl FailureCode {
    /// Every code, so a caller can iterate the vocabulary without holding a
    /// copy of it.
    pub const ALL: [Self; 11] = [
        Self::InsufficientFunds,
        Self::PayerTimeout,
        Self::PayerDeclined,
        Self::InvalidPayer,
        Self::PayerLimitReached,
        Self::PayerAccountBlocked,
        Self::InvalidPayee,
        Self::PayeeAccountBlocked,
        Self::ProviderAccountBlocked,
        Self::ProviderUnavailable,
        Self::ProviderError,
    ];

    /// Whether the payer could plausibly succeed on a fresh PaymentIntent.
    ///
    /// ```
    /// use vpay_core::FailureCode;
    ///
    /// assert!(FailureCode::InsufficientFunds.payer_actionable());
    /// // An unmapped rail error tells the payer nothing they can act on.
    /// assert!(!FailureCode::ProviderError.payer_actionable());
    /// ```
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
    ///
    /// Never true at the same time as [`FailureCode::payer_actionable`]: a
    /// failure belongs to at most one of them, and a code that is neither
    /// (`provider_account_blocked`) is the operator's.
    ///
    /// ```
    /// use vpay_core::FailureCode;
    ///
    /// assert!(FailureCode::InvalidPayee.merchant_actionable());
    /// for code in FailureCode::ALL {
    ///     assert!(!(code.payer_actionable() && code.merchant_actionable()));
    /// }
    /// // A blocked partner account is nobody else's problem.
    /// let ours = FailureCode::ProviderAccountBlocked;
    /// assert!(!ours.payer_actionable() && !ours.merchant_actionable());
    /// ```
    #[must_use]
    pub const fn merchant_actionable(self) -> bool {
        matches!(self, Self::InvalidPayee | Self::PayeeAccountBlocked)
    }

    /// The wire spelling, which is also this code's `Display`.
    ///
    /// ```
    /// use vpay_core::FailureCode;
    ///
    /// assert_eq!(FailureCode::PayerTimeout.as_str(), "payer_timeout");
    /// assert_eq!(FailureCode::PayerTimeout.to_string(), "payer_timeout");
    /// ```
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
