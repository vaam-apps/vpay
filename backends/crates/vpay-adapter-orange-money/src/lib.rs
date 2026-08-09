//! Orange Money Cameroun adapter.
//!
//! STATUS: capabilities are real and enforced; wire calls are NOT implemented.
//! Every unimplemented method returns `ProviderError::NotImplemented` — it never
//! fabricates a success. See `docs/status.md` and `docs/flows/adapter-orange-money.md`.

use vpay_core::{Money, ProviderFlow};
use vpay_provider::{
    CallbackRef, Capabilities, ChargeRef, ChargeStatus, ProviderAdapter, ProviderConfig,
    ProviderError, Submitted,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct Adapter;

impl Adapter {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ProviderAdapter for Adapter {
    fn code(&self) -> &'static str {
        "orange_money"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            flow: ProviderFlow::Redirect,
            supports_refunds: false,
            supports_partial_refunds: false,
            delivers_callbacks: true,
            requires_ip_allowlist: false,
        }
    }

    fn submit(&self, _c: &ChargeRef, _cfg: &ProviderConfig) -> Result<Submitted, ProviderError> {
        Err(ProviderError::NotImplemented("orange_money::submit"))
    }

    fn query_status(
        &self,
        _c: &ChargeRef,
        _cfg: &ProviderConfig,
    ) -> Result<ChargeStatus, ProviderError> {
        Err(ProviderError::NotImplemented("orange_money::query_status"))
    }

    fn parse_callback(&self, _body: &[u8]) -> Result<CallbackRef, ProviderError> {
        Err(ProviderError::NotImplemented(
            "orange_money::parse_callback",
        ))
    }

    fn refund(
        &self,
        _c: &ChargeRef,
        _amount: Money,
        _cfg: &ProviderConfig,
    ) -> Result<Submitted, ProviderError> {
        Err(ProviderError::NotImplemented("orange_money::refund"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_are_coherent() {
        assert!(Adapter::new().capabilities().is_coherent());
    }

    #[test]
    fn code_matches_the_payment_method_type() {
        assert_eq!(Adapter::new().code(), "orange_money");
    }

    /// The honesty test: an unbuilt method must error, never fake a success.
    #[test]
    fn unimplemented_methods_do_not_pretend() {
        let a = Adapter::new();
        assert!(matches!(
            a.parse_callback(b"{}"),
            Err(ProviderError::NotImplemented(_))
        ));
    }
}
