//! The signed transport, from the merchant's side: what the SDK puts on the wire
//! and what it accepts back.
//!
//! It replaced `token_exchange.rs` and `op_conformance.rs` (the bearer-token
//! handshake and its conformance against `authkestra_op`'s verifier), which tested
//! a path that no longer exists. These cases run the SDK against **cratestack's
//! real `EnvelopeLayer`** (`support::front`), so "the SDK's request is valid" is
//! the layer's verdict and not a reading of the SDK's own code.
//!
//! Server-side counterparts, against the real `vpay-api` router and Postgres, are
//! `backends/tests/integration/tests/signed_transport.rs`.

// See `tests/support/mod.rs` for why this allow list mirrors
// `backends/apps/vpay-server/tests/cli.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use cratestack_client_rust::cose::{CoseSigner, Ed25519Signer};
use serde_json::json;
use vpay_sdk::refunds::UpdateRefundParams;
use vpay_sdk::{Client, Credentials, Error, RequestOptions, ServerKeys};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

mod support;
use support::front::{self, Seal};

fn event_json(id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "object": "event",
        "type": "payment_intent.succeeded",
        "created": 1_753_401_600,
        "livemode": false,
        "data": { "object": support::payment_intent_json("pi_1") },
    })
}

async fn upstream_with_event() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "url": "/v1/events",
            "has_more": false,
            "data": [event_json("evt_1")],
        })))
        .mount(&server)
        .await;
    server
}

fn header_value(request: &Request, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
}

#[tokio::test]
async fn a_signed_read_is_opened_by_the_real_layer_and_its_sealed_answer_decodes() {
    let server = upstream_with_event().await;
    let front = front::start(&server.uri(), Seal::Current).await;

    let page = front
        .client()
        .events()
        .list(Default::default())
        .await
        .unwrap();
    assert_eq!(page.data[0].id, "evt_1");
}

#[tokio::test]
async fn the_upstream_receives_the_plain_form_request_and_no_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/refunds/re_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "re_1", "object": "refund", "payment_intent": "pi_1", "amount": 1000,
            "currency": "xaf", "status": "pending", "reason": null, "failure_code": null,
            "fee": null, "metadata": { "k": "v" }, "created": 1_753_401_600,
            "livemode": false,
        })))
        .mount(&server)
        .await;
    let front = front::start(&server.uri(), Seal::Current).await;

    let _ = front
        .client()
        .refunds()
        .update(
            "re_1",
            UpdateRefundParams {
                metadata: [("k".to_owned(), "v".to_owned())].into_iter().collect(),
            },
            RequestOptions::new().with_idempotency_key("idem-123"),
        )
        .await;

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method.as_str(), "POST");
    assert_eq!(
        header_value(request, "content-type").as_deref(),
        Some("application/x-www-form-urlencoded")
    );
    // The key the signature binds is the key the handler sees, as sent.
    assert_eq!(
        header_value(request, "idempotency-key").as_deref(),
        Some("idem-123")
    );
    assert_eq!(String::from_utf8_lossy(&request.body), "metadata[k]=v");
    // There is no token and no Authorization header of any kind.
    assert!(request.headers.get("authorization").is_none());
}

#[tokio::test]
async fn the_user_agent_names_this_sdk_and_its_version() {
    let server = upstream_with_event().await;
    let front = front::start(&server.uri(), Seal::Current).await;
    front
        .client()
        .events()
        .list(Default::default())
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        header_value(&requests[0], "user-agent").as_deref(),
        Some(format!("vpay-sdk-rust/{}", env!("CARGO_PKG_VERSION")).as_str())
    );
}

#[tokio::test]
async fn every_call_spends_a_fresh_cti_so_repeating_a_call_is_never_a_replay() {
    // The layer records `(kid, cti)` and refuses a repeat. Two identical calls
    // from the SDK both succeed, which they can only do if each is sealed anew.
    let server = upstream_with_event().await;
    let front = front::start(&server.uri(), Seal::Current).await;
    let client = front.client();
    for _ in 0..3 {
        client.events().list(Default::default()).await.unwrap();
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

#[tokio::test]
async fn an_answer_sealed_by_the_next_pinned_key_verifies() {
    let server = upstream_with_event().await;
    let front = front::start(&server.uri(), Seal::Next).await;

    front
        .client()
        .events()
        .list(Default::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn an_answer_sealed_by_a_key_that_was_not_pinned_is_refused_as_unverified() {
    let server = upstream_with_event().await;
    let front = front::start(&server.uri(), Seal::Unpinned).await;

    match front
        .client()
        .events()
        .list(Default::default())
        .await
        .unwrap_err()
    {
        Error::Unverified => {}
        other => panic!("expected Unverified, got {other:?}"),
    }
}

#[tokio::test]
async fn a_client_that_pinned_only_the_current_key_refuses_an_answer_under_next() {
    let server = upstream_with_event().await;
    let front = front::start(&server.uri(), Seal::Next).await;
    let current_only = Client::builder(&front.base_url)
        .credentials(front::merchant_credentials())
        .server_keys(ServerKeys::current_only(front::server_keys_current()))
        .build()
        .unwrap();

    match current_only
        .events()
        .list(Default::default())
        .await
        .unwrap_err()
    {
        Error::Unverified => {}
        other => panic!("expected Unverified, got {other:?}"),
    }
}

#[tokio::test]
async fn a_key_the_server_has_not_registered_gets_the_bare_unsigned_401_and_is_never_retried() {
    let server = upstream_with_event().await;
    let front = front::start(&server.uri(), Seal::Current).await;
    let stranger = Client::builder(&front.base_url)
        .credentials(Credentials::ed25519_seed("stranger", &[9; 32]).unwrap())
        .server_keys(front::server_keys())
        .build()
        .unwrap();

    match stranger
        .events()
        .list(Default::default())
        .await
        .unwrap_err()
    {
        Error::Unsigned { status } => assert_eq!(status, 401),
        other => panic!("expected Unsigned(401), got {other:?}"),
    }
    // Nothing reached the handler, and the SDK did not try again.
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_request_addressed_to_another_audience_is_refused() {
    let server = upstream_with_event().await;
    let front = front::start(&server.uri(), Seal::Current).await;
    let misaddressed = Client::builder(&front.base_url)
        .credentials(front::merchant_credentials())
        .server_keys(front::server_keys())
        .audience("https://some-other-deployment.example")
        .build()
        .unwrap();

    match misaddressed
        .events()
        .list(Default::default())
        .await
        .unwrap_err()
    {
        Error::Unsigned { status } => assert_eq!(status, 401),
        other => panic!("expected Unsigned(401), got {other:?}"),
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_key_held_outside_the_process_seals_the_same_calls() {
    // A KMS-held key: the SDK hands the callback the exact bytes to sign and
    // takes back a 64-byte Ed25519 signature.
    let server = upstream_with_event().await;
    let front = front::start(&server.uri(), Seal::Current).await;
    let signer = Arc::new(Ed25519Signer::from_seed(&front::merchant_seed()));
    let public = front::merchant_credentials().public_key();
    let credentials = Credentials::ed25519_external("kms", &public, move |tbs| {
        let signer = Arc::clone(&signer);
        async move { signer.sign(&tbs).await }
    })
    .unwrap();
    let client = Client::builder(&front.base_url)
        .credentials(credentials)
        .server_keys(front::server_keys())
        .build()
        .unwrap();

    client.events().list(Default::default()).await.unwrap();
}

#[tokio::test]
async fn an_external_signer_whose_signature_does_not_verify_is_refused_before_sending() {
    let server = upstream_with_event().await;
    let front = front::start(&server.uri(), Seal::Current).await;
    let public = front::merchant_credentials().public_key();
    // Signs with the wrong key: refused here, not left to fail at vpay as the
    // coarse 401.
    let wrong = Arc::new(Ed25519Signer::from_seed(&[4; 32]));
    let credentials = Credentials::ed25519_external("kms", &public, move |tbs| {
        let wrong = Arc::clone(&wrong);
        async move { wrong.sign(&tbs).await }
    })
    .unwrap();
    let client = Client::builder(&front.base_url)
        .credentials(credentials)
        .server_keys(front::server_keys())
        .build()
        .unwrap();

    match client.events().list(Default::default()).await.unwrap_err() {
        Error::Seal(_) => {}
        other => panic!("expected Seal, got {other:?}"),
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// A redirect is an unsigned answer, never a hop. The request is a sealed body
/// that is valid once for whoever receives it within the skew, so following a
/// `307` would hand the COSE message, its `Idempotency-Key` and its unspent
/// `cti` to another origin, which could replay it to vpay. The second origin
/// below must receive nothing at all.
#[tokio::test]
async fn a_redirect_to_another_origin_is_refused_and_that_origin_receives_nothing() {
    let elsewhere = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&elsewhere)
        .await;

    let redirecting = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", format!("{}/v1/events", elsewhere.uri())),
        )
        .mount(&redirecting)
        .await;

    let client = Client::builder(redirecting.uri())
        .credentials(front::merchant_credentials())
        .server_keys(front::server_keys())
        .build()
        .unwrap();
    match client.events().list(Default::default()).await.unwrap_err() {
        Error::Unsigned { status: 307 } => {}
        other => panic!("expected Unsigned {{ status: 307 }}, got {other:?}"),
    }

    assert_eq!(
        redirecting.received_requests().await.unwrap().len(),
        1,
        "the original origin is asked once, never retried"
    );
    assert!(
        elsewhere.received_requests().await.unwrap().is_empty(),
        "the sealed request must not have been re-sent to the redirect target"
    );
}

/// An answer larger than [`vpay_sdk::MAX_RESPONSE_BYTES`] is a typed error and
/// is not read further, whether it declares its length or not.
#[tokio::test]
async fn an_oversized_answer_is_a_typed_error_declared_or_chunked() {
    // Declared: `wiremock` sets `Content-Length`, so nothing is read at all.
    let declared = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![
            0_u8;
            vpay_sdk::MAX_RESPONSE_BYTES
                + 1
        ]))
        .mount(&declared)
        .await;
    let client = Client::builder(declared.uri())
        .credentials(front::merchant_credentials())
        .server_keys(front::server_keys())
        .build()
        .unwrap();
    match client.events().list(Default::default()).await.unwrap_err() {
        Error::ResponseTooLarge { limit } => assert_eq!(limit, vpay_sdk::MAX_RESPONSE_BYTES),
        other => panic!("expected ResponseTooLarge, got {other:?}"),
    }

    // Chunked, with no length to refuse on: the count is what stops it.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut scratch = [0_u8; 8192];
        let _ = socket.read(&mut scratch).await;
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..(vpay_sdk::MAX_RESPONSE_BYTES / chunk.len() + 2) {
            let framed = [
                format!("{:x}\r\n", chunk.len()).into_bytes(),
                chunk.clone(),
                b"\r\n".to_vec(),
            ]
            .concat();
            if socket.write_all(&framed).await.is_err() {
                return;
            }
        }
        let _ = socket.write_all(b"0\r\n\r\n").await;
    });
    let client = Client::builder(format!("http://{address}"))
        .credentials(front::merchant_credentials())
        .server_keys(front::server_keys())
        .build()
        .unwrap();
    match client.events().list(Default::default()).await.unwrap_err() {
        Error::ResponseTooLarge { .. } => {}
        other => panic!("expected ResponseTooLarge, got {other:?}"),
    }
    server.abort();
}
