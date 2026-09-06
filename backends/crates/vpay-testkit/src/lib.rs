//! Test-only helpers: containers and rail stubs.
//!
//! # This crate must never be a runtime dependency
//!
//! A stub rail is a *deployment* (a WireMock URL under `providers[].host` in
//! the YAML — there is no `provider_hosts` table and there never has been;
//! this line said otherwise until 2026-09-06), not a bean wired into the
//! server. Nothing here may be reachable from
//! `vpay-server` or `vpay-worker-bin`. `cargo xtask verify-no-mocks` enforces
//! it in CI. See `docs/adr/0006-no-mocks-in-main-processes.md`.
//!
//! A real container is not a test double: [`containers`] starts the same
//! `postgres:16-alpine` image `compose.yml` runs. It lives here because it is
//! shared by suites in four different packages, not because it substitutes
//! for anything.

pub mod containers;

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
