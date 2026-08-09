//! PaymentIntent and charge state, and the flow shape that selects between
//! them. See `docs/flows/payment-lifecycle.md`.

use serde::{Deserialize, Serialize};

/// How a payer authorises on a given rail.
///
/// The core branches on this *value*. It must never branch on a provider code —
/// see `docs/adr/0002-provider-port.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFlow {
    /// A prompt reaches the payer's handset (MTN MoMo). The payer can act
    /// before we learn whether submission succeeded, so the reference must be
    /// durable *before* submitting.
    Push,
    /// The payer is redirected to the rail's hosted page (Orange Money). They
    /// cannot act until we hand them a URL, so the rail's token must be durable
    /// *before* redirecting.
    Redirect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    RequiresPaymentMethod,
    /// Redirect rails only; carries `next_action.redirect_to_url`.
    RequiresAction,
    Processing,
    Succeeded,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeState {
    Submitting,
    Submitted,
    Pending,
    /// Past the 24h horizon with no terminal answer. Still polled, but a human
    /// has been alerted. Never a silent failure.
    Unresolved,
    Succeeded,
    Failed,
}

impl ChargeState {
    /// Whether the reconciler should keep polling this charge.
    #[must_use]
    pub const fn is_live(self) -> bool {
        matches!(
            self,
            Self::Submitting | Self::Submitted | Self::Pending | Self::Unresolved
        )
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

impl ProviderFlow {
    /// The PaymentIntent status a successful `confirm` produces on this rail.
    #[must_use]
    pub const fn status_after_confirm(self) -> IntentStatus {
        match self {
            Self::Push => IntentStatus::Processing,
            Self::Redirect => IntentStatus::RequiresAction,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_is_still_polled() {
        assert!(ChargeState::Unresolved.is_live());
        assert!(!ChargeState::Unresolved.is_terminal());
    }

    #[test]
    fn flow_selects_the_post_confirm_status() {
        assert_eq!(
            ProviderFlow::Push.status_after_confirm(),
            IntentStatus::Processing
        );
        assert_eq!(
            ProviderFlow::Redirect.status_after_confirm(),
            IntentStatus::RequiresAction
        );
    }

    #[test]
    fn every_state_is_live_or_terminal_exclusively() {
        for s in [
            ChargeState::Submitting,
            ChargeState::Submitted,
            ChargeState::Pending,
            ChargeState::Unresolved,
            ChargeState::Succeeded,
            ChargeState::Failed,
        ] {
            assert_ne!(s.is_live(), s.is_terminal(), "{s:?} is neither or both");
        }
    }
}
