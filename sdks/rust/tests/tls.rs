//! The TLS stack this SDK ships, and the one hazard it exists to avoid.
//!
//! `reqwest` is pinned with `rustls-no-provider` (the only feature set that
//! keeps the banned `aws-lc-rs` provider out of the graph — see the root
//! `Cargo.toml`), and under that feature **reqwest's own client builder
//! panics** if no process-wide `CryptoProvider` has been installed:
//!
//! ```text
//! No rustls crypto provider is configured. When using the
//! `rustls-no-provider` feature you must install a crypto provider before
//! building a Client.
//! ```
//!
//! A merchant SDK is a library inside somebody else's process. It cannot
//! require that process to have called `install_default()`, and it must not
//! call it on that process's behalf — installing a provider is a decision
//! that belongs to the application, and whoever installs first wins. So
//! `ClientBuilder::build` hands reqwest an already-built
//! `rustls::ClientConfig` instead, taking a code path that never consults the
//! process default.
//!
//! **This file contains no `install_default()` call, and asserts that none has
//! happened.** `cargo nextest` runs each test in its own process, so the
//! assertion below is about a genuinely fresh process, not one another test
//! has already prepared.
//!
//! What is **not** proven here: that a TLS handshake against a real vpay
//! succeeds. Nothing in this repository serves TLS (`wiremock` is plaintext
//! HTTP), and no test reaches the public internet, so certificate
//! verification against the vendored roots is exercised nowhere. The last
//! test below proves only that the TLS stack is constructed and reached — a
//! handshake is attempted and fails against a plaintext listener — not that
//! it would succeed against a real one.

// See `tests/support/mod.rs` for why this allow list mirrors
// `backends/apps/vpay-server/tests/cli.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::LazyLock;
use std::time::Duration;

use vpay_sdk::{Client, Credentials, Error};
use wiremock::MockServer;

mod support;

static KEY: LazyLock<support::TestKey> = LazyLock::new(|| support::generate_key(None));

fn client_for(base_url: String) -> Result<Client, vpay_sdk::ConfigError> {
    Client::builder(base_url)
        .credentials(Credentials::rsa_pem("merchant_acme", &KEY.pem).unwrap())
        .timeout(Duration::from_secs(5))
        .build()
}

#[test]
fn a_client_builds_in_a_process_that_never_installed_a_crypto_provider() {
    // The precondition, asserted rather than assumed: if some other code in
    // this process had installed a default provider, this test would pass for
    // a reason that says nothing about a merchant's process.
    assert!(
        rustls::crypto::CryptoProvider::get_default().is_none(),
        "this test is only meaningful in a process with no default provider"
    );

    client_for("https://api.vpay.example".to_string())
        .expect("building a client must not depend on a process-wide provider");

    // And the SDK did not quietly install one either — that decision stays
    // with the application.
    assert!(
        rustls::crypto::CryptoProvider::get_default().is_none(),
        "the SDK must not install a process-wide CryptoProvider behind its caller's back"
    );
}

#[tokio::test]
async fn an_https_url_actually_attempts_a_tls_handshake() {
    // Points the SDK at a plaintext HTTP listener over `https://`. A TLS
    // stack that was never wired up would fail differently — at build time,
    // or with an "unknown scheme"/"TLS backend not enabled" error, or by
    // succeeding in plaintext. Reaching a transport failure *during the
    // handshake* is what shows rustls is in the request path.
    //
    // It does not show that certificate verification works: that needs a real
    // certificate chain, and nothing here serves one.
    let server = MockServer::start().await;
    let https = server.uri().replace("http://", "https://");

    let client = client_for(https).expect("the client builds");
    match client.balance().retrieve().await.unwrap_err() {
        Error::Transport(message) => {
            assert!(
                !message.is_empty(),
                "the transport failure should carry the underlying cause"
            );
        }
        other => panic!("expected a transport error from the failed handshake, got {other:?}"),
    }
}
