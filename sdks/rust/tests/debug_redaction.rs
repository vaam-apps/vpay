//! The private key, the live access token, a `PaymentIntent`'s
//! `client_secret` and a `CheckoutSession`'s must never reach a `{:?}`.
//!
//! This file is named by the `Debug` implementations of
//! [`vpay_sdk::Credentials`], `Client`, `PaymentIntent` and
//! `CheckoutSession` themselves: they
//! are hand-written rather than derived *because* of these tests, and
//! replacing any of them with `#[derive(Debug)]` must fail here. A
//! merchant's private key, bearer token, or payer credential in a log line
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

use vpay_sdk::{CheckoutSession, Client, Credentials, PaymentIntent};
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
fn a_payment_intents_debug_output_never_contains_its_client_secret() {
    let secret = "pi_1_secret_super_secret_value";
    let rendered = format!(
        "{:?}",
        serde_json::from_str::<PaymentIntent>(&format!(
            r#"{{"id":"pi_1","object":"payment_intent","amount":5000,"currency":"xaf",
                "status":"requires_payment_method","payment_method_types":["mtn_momo"],
                "next_action":null,"last_payment_error":null,"metadata":{{}},
                "description":null,"created":1,"livemode":false,
                "client_secret":"{secret}"}}"#
        ))
        .unwrap()
    );
    assert!(
        !rendered.contains(secret),
        "Debug output must not contain the client_secret"
    );
    assert!(rendered.contains("[redacted]") || rendered.contains("chars redacted"));
    // The rest of the object is still useful for debugging.
    assert!(rendered.contains("pi_1"));
}

#[test]
fn a_checkout_sessions_debug_output_never_contains_its_client_secret_or_its_url_fragment() {
    // Two credentials in one object, not one. D6 puts the session's
    // `client_secret` in the hosted page's URL fragment, so a `Debug` that
    // redacted only the named field would print the same bytes through
    // `url` — which is exactly what the first version of the Node SDK's
    // equivalent did, and what the test there now catches too.
    let secret = "cs_1_secret_super_secret_value";
    let session: CheckoutSession = serde_json::from_str(&format!(
        r#"{{"id":"cs_1","object":"checkout.session","livemode":false,
            "payment_intent":"pi_1","ui_mode":"hosted","status":"open",
            "payment_status":"unpaid",
            "success_url":"https://shop.example/ok?sid={{CHECKOUT_SESSION_ID}}",
            "cancel_url":"https://shop.example/cancel","return_url":null,
            "url":"https://checkout.example/c/cs_1#{secret}",
            "expires_at":86401,"created":1,
            "client_secret":"{secret}"}}"#
    ))
    .unwrap();

    let rendered = format!("{session:?}");
    assert!(
        !rendered.contains(secret),
        "Debug output must not contain the client_secret: {rendered}"
    );
    assert!(rendered.contains("chars redacted"), "{rendered}");
    // A redaction, not a blackout: which page the session points at is the
    // whole diagnostic value of `url`, and it survives.
    assert!(
        rendered.contains("https://checkout.example/c/cs_1#["),
        "{rendered}"
    );
    assert!(rendered.contains("cs_1"));
    assert!(rendered.contains("pi_1"));
    // The merchant's own forwarding URLs are not credentials and are not
    // touched.
    assert!(rendered.contains("{CHECKOUT_SESSION_ID}"), "{rendered}");
}

#[test]
fn a_checkout_session_without_a_secret_or_a_fragment_renders_no_redaction_marker() {
    // The list shape: no `client_secret` key at all, and an embedded
    // session has no `url`. Nothing to redact, and nothing pretending
    // there was.
    let session: CheckoutSession = serde_json::from_str(
        r#"{"id":"cs_2","object":"checkout.session","livemode":false,
            "payment_intent":"pi_2","ui_mode":"embedded","status":"open",
            "payment_status":"unpaid","success_url":null,"cancel_url":null,
            "return_url":"https://shop.example/order/42","url":null,
            "expires_at":86401,"created":1}"#,
    )
    .unwrap();

    assert_eq!(session.client_secret, None);
    let rendered = format!("{session:?}");
    assert!(!rendered.contains("chars redacted"), "{rendered}");
    assert!(rendered.contains("client_secret: None"), "{rendered}");
    assert!(rendered.contains("url: None"), "{rendered}");
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
