//! Wiring for the server binary.

use vpay_provider::ProviderAdapter;

/// Every adapter linked into this binary, each holding a clone of the
/// process's one outbound HTTP client.
///
/// `http` is a parameter rather than something built here because there must
/// be exactly one client per process: `reqwest::Client` owns a connection
/// pool and clones share it, and building a second one would double the
/// pools while giving the second no chance of being the vendored-roots
/// client the `FROM scratch` image requires
/// (`vpay_provider::http::client_with_timeouts`). It also keeps this
/// function infallible — client construction can fail, and `main` is where
/// that failure belongs.
///
/// Note what is absent: there is no stub or fake entry. A stub rail is a
/// WireMock host in configuration, never a variant here.
/// See `docs/adr/0006-no-mocks-in-main-processes.md`.
#[must_use]
pub fn adapters(http: reqwest::Client) -> Vec<Box<dyn ProviderAdapter>> {
    vec![
        Box::new(vpay_adapter_mtn_momo::Adapter::new(http.clone())),
        Box::new(vpay_adapter_orange_money::Adapter::new(http.clone())),
        // Admitted under ADR-0031 (rule 3 amended): its status read takes a
        // key Notch Pay calls public, and boot warns about it for every
        // deployment that configures the rail.
        Box::new(vpay_adapter_notchpay::Adapter::new(http)),
    ]
}

/// The rail codes this binary links, for the boot log line.
///
/// Codes rather than adapters (it was `adapter_registry() -> Vec<&str>`,
/// built by constructing every adapter): a log line must not need an HTTP
/// client, and the previous shape would have forced `main` to build a
/// throwaway TLS stack — or to log after the client existed — purely to
/// print three strings. The two lists are kept honest by
/// `the_codes_match_the_adapters_that_are_linked` below.
#[must_use]
pub const fn adapter_codes() -> [&'static str; 3] {
    ["mtn_momo", "orange_money", "notchpay"]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same client `main` builds, so what is under test is the real
    /// constructor rather than something assembled only for tests.
    fn http() -> reqwest::Client {
        vpay_provider::http::client().expect("the vendored-roots client builds")
    }

    #[test]
    fn both_mvp_rails_are_linked() {
        let codes = adapter_codes();
        assert!(codes.contains(&"mtn_momo"), "MTN missing: {codes:?}");
        assert!(codes.contains(&"orange_money"), "Orange missing: {codes:?}");
        assert!(codes.contains(&"notchpay"), "Notch Pay missing: {codes:?}");
    }

    /// ADR-0031: exactly the rail admitted on the amended rule 3 declares an
    /// exposure, so the boot warning fires for it and for nothing else. A
    /// future rail on the second limb fails here until this list names it,
    /// which is the review ADR-0031 asks for.
    #[test]
    fn only_notchpay_declares_an_accepted_exposure() {
        let declaring: Vec<&str> = adapters(http())
            .iter()
            .filter(|adapter| adapter.accepted_exposure().is_some())
            .map(|adapter| adapter.code())
            .collect();
        assert_eq!(declaring, ["notchpay"]);
    }

    /// The log line and the wiring must not drift: `adapter_codes` is a
    /// hand-written list precisely so it needs no client, and a hand-written
    /// list is one someone can forget to update.
    #[test]
    fn the_codes_match_the_adapters_that_are_linked() {
        let linked: Vec<&str> = adapters(http()).iter().map(|a| a.code()).collect();
        assert_eq!(linked, adapter_codes().to_vec());
    }

    #[test]
    fn no_adapter_advertises_partial_without_full_refunds() {
        for a in adapters(http()) {
            assert!(a.capabilities().is_coherent(), "{} incoherent", a.code());
        }
    }

    /// The rails have different flow shapes; that is the whole point of the
    /// port. If this ever collapses to one shape, the port stopped being tested.
    #[test]
    fn the_registry_covers_both_flow_shapes() {
        use vpay_core::ProviderFlow;
        let flows: Vec<_> = adapters(http())
            .iter()
            .map(|a| a.capabilities().flow)
            .collect();
        assert!(flows.contains(&ProviderFlow::Push));
        assert!(flows.contains(&ProviderFlow::Redirect));
    }
}
