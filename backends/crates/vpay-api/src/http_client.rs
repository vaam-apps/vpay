//! The outbound HTTP client every server-side caller in this workspace
//! should use, and the reason one has to exist at all.
//!
//! # Why not `reqwest::Client::new()`
//!
//! The runtime image is `FROM scratch` ([ADR-0004]): no glibc, no shell, and
//! no OS certificate store. reqwest is pinned at 0.13 with
//! `rustls-no-provider` (see the long note on the pin in the root
//! `Cargo.toml`), and on that version reqwest no longer offers a
//! vendored-roots feature — it builds a `rustls_platform_verifier::Verifier`,
//! i.e. it reads the *platform* trust store. It does so **eagerly, inside
//! `ClientBuilder::build()`**, not lazily at connect time, and when the store
//! turns up empty the verifier returns
//! `General("No CA certificates were loaded from the system")`, which
//! `Client::new()` converts into a panic.
//!
//! That is not a hypothetical. `JwtValidator::new` used to call
//! `authkestra_resource::jwt::JwksCache::new`, which calls
//! `reqwest::Client::new()`, and `vpay-server` panicked at boot inside its own
//! image while passing every test on machines that happen to have `/etc/ssl`.
//! The JWKS URL it was about to fetch is plain `http://` over loopback — TLS
//! was never going to be negotiated — so the failure had nothing to do with
//! the request being made and everything to do with when the trust store is
//! read. `tests/cli.rs`'s
//! `a_server_with_no_os_trust_store_boots_and_still_validates_tokens`
//! reproduces the condition with `SSL_CERT_FILE`/`SSL_CERT_DIR` pointed at
//! paths that do not exist.
//!
//! [`client`] therefore hands reqwest a finished [`rustls::ClientConfig`]
//! built from Mozilla's vendored bundle. That takes reqwest's
//! `TlsBackend::BuiltRustls` branch, which consults neither the platform
//! verifier nor the process-wide `CryptoProvider` — so it also cannot hit the
//! *other* panic the `rustls-no-provider` pin exposes ("No rustls crypto
//! provider is configured"), independently of whether the binary installed a
//! default provider first.
//!
//! # The trade-off, stated plainly
//!
//! Vendored roots mean a deployment behind a TLS-intercepting proxy with a
//! private CA is not served by this client, and `SSL_CERT_FILE` will not
//! change that. That is the deliberate cost of being able to run in a
//! `scratch` image at all; the alternative is an image that carries a trust
//! store, which is a different ADR.
//!
//! # The twin in `sdks/rust`
//!
//! `sdks/rust/src/client.rs` has a near-identical `rustls_client_config`, and
//! it stays a separate copy on purpose: `vpay-sdk` is what a *merchant*
//! compiles into their own process, so it must not depend on a server crate —
//! making it depend on `vpay-api` would drag axum, sqlx and the whole OP into
//! a merchant's build. The two are expected to stay in step; if you change
//! the provider, the root source or the ALPN list here, change it there too.
//! The SDK's copy carries the extra constraint that a library inside someone
//! else's process may neither panic nor install a process-wide
//! `CryptoProvider` on that process's behalf.
//!
//! [ADR-0004]: ../../../docs/adr/0004-musl-mimalloc.md

use std::sync::Arc;

use vpay_core::error::{Category, Classify};

/// Why an outbound HTTP client could not be constructed.
///
/// Construction-time only: this is not reachable from a request path, which
/// is why [`crate::ApiError`] does not `#[from]` it (ADR-0011 asks a
/// composite to cover the leaves its layer can *meet*, and serving a request
/// never builds a client). It surfaces at boot, through `anyhow`, in
/// `backends/apps/*`.
#[derive(Debug, thiserror::Error)]
pub enum HttpClientError {
    /// The vendored-roots rustls configuration did not assemble.
    ///
    /// Every input to it is fixed at compile time — the `ring` provider, the
    /// protocol versions the `rustls` pin enables, and a root store baked
    /// into the binary — so nothing an operator supplies can cause this. It
    /// is a bug in this module or a broken `rustls` pin.
    #[error("assembling the vendored-roots rustls configuration")]
    Tls(#[source] rustls::Error),

    /// reqwest refused to build a client around that configuration.
    #[error("building the outbound HTTP client")]
    Client(#[source] reqwest::Error),
}

impl Classify for HttpClientError {
    /// Both variants page, and deliberately so.
    ///
    /// [`Category::Configuration`] would be the wrong answer even for the
    /// reqwest half: there is no configuration file, flag or environment
    /// variable an operator could change to fix either one, because this
    /// function takes no inputs. A failure here means an invariant this crate
    /// guarantees — "the fixed, vendored TLS stack always assembles" — did
    /// not hold, which is [`Category::Internal`] by ADR-0011's definition,
    /// and exit `1` rather than `78` is the honest signal to a supervisor.
    fn category(&self) -> Category {
        Category::Internal
    }
}

/// Mozilla's CA bundle, compiled into the binary, as a rustls root store.
///
/// `webpki_roots` rather than `rustls_native_certs`: see the module doc. It
/// is already in the resolved graph (`sqlx`'s `tls-rustls-ring` feature
/// vendors the same bundle for Postgres TLS), so trusting it here does not
/// widen the dependency surface — it makes the two agree.
fn vendored_root_store() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

/// An outbound `reqwest::Client` whose trust anchors are compiled in and
/// which never reads the OS certificate store — the only kind that survives
/// the `FROM scratch` runtime image. See the module doc for the panic this
/// exists to prevent.
///
/// # Errors
///
/// [`HttpClientError::Tls`] if the vendored rustls configuration does not
/// assemble, [`HttpClientError::Client`] if reqwest refuses it. Neither is
/// expected to be reachable; they are `Result` rather than a panic because
/// this crate denies `unwrap`/`expect`/`panic` in shipping code (ADR-0007),
/// and because "impossible" is a claim a payment system should not make with
/// a panic.
///
/// # Deliberately not configurable
///
/// No timeout, no header, no proxy setting: this returns the same client
/// `reqwest::Client::new()` would have, minus the trust-store dependency, so
/// swapping it in changes exactly one thing. A future rail adapter that wants
/// its own timeouts should grow a `builder()` sibling here that returns the
/// preconfigured `reqwest::ClientBuilder` rather than re-deriving the TLS
/// configuration at its own call site — one such function is not written yet
/// because nothing calls it, and an untested one would be a claim this crate
/// has not earned.
pub fn client() -> Result<reqwest::Client, HttpClientError> {
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(HttpClientError::Tls)?
    .with_root_certificates(vendored_root_store())
    .with_no_client_auth();
    // Set explicitly because reqwest only fills ALPN in on the branch *not*
    // taken here: hand it a built `ClientConfig` and whatever is in
    // `alpn_protocols` is what goes on the wire, so omitting these two names
    // would silently mean a TLS connection never negotiates HTTP/2.
    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    reqwest::Client::builder()
        // The bare config, not `Some(config)`: `tls_backend_preconfigured`
        // wraps its argument in an `Option` itself before downcasting, so an
        // already-`Option` argument silently becomes `UnknownPreconfigured`
        // and `build()` fails with a bare "builder error".
        .tls_backend_preconfigured(tls)
        .build()
        .map_err(HttpClientError::Client)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: building a client must not depend on a process-wide
    /// `CryptoProvider` having been installed, and must not read the platform
    /// trust store. This unit test can only prove the first half (the second
    /// needs a process whose environment says the store is empty, which is
    /// `tests/cli.rs`'s job) — but the first half is the one that would
    /// otherwise panic rather than return an `Err`.
    #[test]
    fn a_client_builds_without_a_process_wide_crypto_provider() {
        assert!(client().is_ok(), "the vendored-roots client must build");
    }

    /// A `RootCertStore` with nothing in it would still let `client()` return
    /// `Ok` — and would then reject every TLS peer on earth at connect time,
    /// far from here. Asserting the bundle is non-empty pins the vendoring
    /// itself rather than the plumbing around it.
    #[test]
    fn the_vendored_bundle_is_not_empty() {
        assert!(
            !vendored_root_store().is_empty(),
            "webpki-roots must supply trust anchors"
        );
    }
}
