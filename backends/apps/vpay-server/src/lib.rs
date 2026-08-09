//! Wiring for the server binary.

use vpay_provider::ProviderAdapter;

/// Every adapter linked into this binary.
///
/// Note what is absent: there is no stub or fake entry. A stub rail is a
/// WireMock host in configuration, never a variant here.
/// See `docs/adr/0006-no-mocks-in-main-processes.md`.
#[must_use]
pub fn adapters() -> Vec<Box<dyn ProviderAdapter>> {
    vec![
        Box::new(vpay_adapter_mtn_momo::Adapter::new()),
        Box::new(vpay_adapter_orange_money::Adapter::new()),
    ]
}

#[must_use]
pub fn adapter_registry() -> Vec<&'static str> {
    adapters().iter().map(|a| a.code()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_mvp_rails_are_linked() {
        let codes = adapter_registry();
        assert!(codes.contains(&"mtn_momo"), "MTN missing: {codes:?}");
        assert!(codes.contains(&"orange_money"), "Orange missing: {codes:?}");
    }

    #[test]
    fn no_adapter_advertises_partial_without_full_refunds() {
        for a in adapters() {
            assert!(a.capabilities().is_coherent(), "{} incoherent", a.code());
        }
    }

    /// The rails have different flow shapes; that is the whole point of the
    /// port. If this ever collapses to one shape, the port stopped being tested.
    #[test]
    fn the_registry_covers_both_flow_shapes() {
        use vpay_core::ProviderFlow;
        let flows: Vec<_> = adapters().iter().map(|a| a.capabilities().flow).collect();
        assert!(flows.contains(&ProviderFlow::Push));
        assert!(flows.contains(&ProviderFlow::Redirect));
    }
}
