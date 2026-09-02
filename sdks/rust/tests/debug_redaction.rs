//! The private key and the live access token must never reach a `{:?}`.
//!
//! This file is named by the `Debug` implementations of
//! [`vpay_sdk::Credentials`] and `Client` themselves: they are hand-written
//! rather than derived *because* of these tests, and replacing either with
//! `#[derive(Debug)]` must fail here. A merchant's private key in a log line
//! is not a formatting nit — it is the whole credential, and structured
//! loggers print `{:?}` of whatever they are handed.

// See `tests/support/mod.rs` for why this allow list mirrors
// `backends/apps/vpay-server/tests/cli.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::LazyLock;

use vpay_sdk::{Client, Credentials};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod support;

static KEY: LazyLock<support::TestKey> = LazyLock::new(|| support::generate_key(Some("key-a")));

/// Distinctive substrings of a PKCS#1 PEM. Checking for the base64 body as
/// well as the header means a `Debug` that printed only the key *bytes*
/// (without the armour) would still be caught.
fn assert_no_key_material(rendered: &str) {
    assert!(!rendered.contains("BEGIN"), "PEM armour leaked: {rendered}");
    assert!(
        !rendered.contains("PRIVATE KEY"),
        "PEM label leaked: {rendered}"
    );
    for line in KEY.pem.lines().filter(|l| !l.starts_with("-----")) {
        // Any full base64 line of the key is enough to matter.
        assert!(
            !rendered.contains(line),
            "key material leaked into: {rendered}"
        );
    }
}

#[test]
fn credentials_debug_shows_the_client_id_and_kid_but_no_key_material() {
    let credentials = Credentials::rsa_pem("merchant_acme", &KEY.pem)
        .unwrap()
        .with_kid("key-a");

    for rendered in [format!("{credentials:?}"), format!("{credentials:#?}")] {
        assert_no_key_material(&rendered);
        // Still useful: the fields an operator actually needs are present.
        assert!(rendered.contains("merchant_acme"));
        assert!(rendered.contains("key-a"));
    }
}

#[test]
fn client_and_builder_debug_carry_no_key_material() {
    let builder = Client::builder("https://api.vpay.test")
        .credentials(Credentials::rsa_pem("merchant_acme", &KEY.pem).unwrap());
    assert_no_key_material(&format!("{builder:?}"));

    let client = builder.build().unwrap();
    assert_no_key_material(&format!("{client:?}"));
    assert_no_key_material(&format!("{client:#?}"));
}

#[tokio::test]
async fn a_cached_access_token_never_appears_in_the_clients_debug_output() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(support::token_response("tok_super_secret_value", 300)),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "balance", "available": [], "pending": [],
        })))
        .mount(&server)
        .await;

    let client = Client::builder(server.uri())
        .credentials(Credentials::rsa_pem("merchant_acme", &KEY.pem).unwrap())
        .build()
        .unwrap();
    // Populate the cache first — a `Debug` written before any token exists
    // would pass trivially.
    client.balance().retrieve().await.unwrap();

    let rendered = format!("{client:?}");
    assert!(
        !rendered.contains("tok_super_secret_value"),
        "the cached bearer token leaked: {rendered}"
    );
    assert!(rendered.contains("[redacted]"));
    assert_no_key_material(&rendered);
}

#[test]
fn a_rejected_private_key_does_not_echo_the_key_into_the_error() {
    // The error path is the easiest place for a credential to escape: an
    // error string built from the input is a log line by another name.
    let err = Credentials::rsa_pem(
        "merchant_acme",
        "-----BEGIN PRIVATE KEY-----\nnot-a-key\n-----END PRIVATE KEY-----\n",
    )
    .expect_err("a malformed PEM is refused");
    let rendered = format!("{err} / {err:?}");
    assert!(
        !rendered.contains("not-a-key"),
        "the supplied key material was echoed: {rendered}"
    );
}
