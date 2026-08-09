//! Test-only helpers: containers and rail stubs.
//!
//! # This crate must never be a runtime dependency
//!
//! A stub rail is a *deployment* (a WireMock host in `provider_hosts`), not a
//! bean wired into the server. Nothing here may be reachable from
//! `vpay-server` or `vpay-worker-bin`. `cargo xtask verify-no-mocks` enforces
//! it in CI. See `docs/adr/0006-no-mocks-in-main-processes.md`.

/// Marker asserting the intended usage of this crate, for documentation tests.
#[must_use]
pub const fn is_test_only() -> bool {
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn marker_holds() {
        assert!(super::is_test_only());
    }
}
