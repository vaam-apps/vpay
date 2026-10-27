//! Error mapping, per `docs/flows/merchant-auth.md`'s "Errors" section: the
//! Stripe-shaped envelope becomes one typed error carrying all four fields, a
//! body that is *not* that envelope becomes a distinct "unexpected response"
//! carrying a bounded prefix, and a failure that never produced an HTTP
//! response at all is a third, distinct error.
//!
//! Keeping the three apart matters more than it looks: a merchant retrying on
//! a transport failure is correct, and retrying on a `400` is a bug. If the
//! SDK collapsed them, no caller could tell.

// See `tests/support/mod.rs` for why this allow list mirrors
// `backends/apps/vpay-server/tests/cli.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::net::TcpListener;
use std::time::Duration;

use serde_json::json;
use vpay_sdk::{Client, Error};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod support;
use support::front;

/// An upstream, the signed front in front of it, and a client pointed at the
/// front. The upstream's answers are sealed on the way back, so a test that
/// mounts a body is testing what the SDK does with a body vpay vouched for.
async fn fixture() -> (MockServer, front::Front, Client) {
    let server = MockServer::start().await;
    let front = front::start(&server.uri(), front::Seal::Current).await;
    let client = front.client();
    (server, front, client)
}

/// A harmless, real, read-only call: `GET /v1/events`. (These tests used
/// `GET /v1/balance` until VP-3; that route is mounted nowhere, so a signed
/// client has no operation to bind it to and refuses it before sending.)
async fn probe(client: &Client) -> Result<vpay_sdk::List<vpay_sdk::Event>, Error> {
    client.events().list(Default::default()).await
}

/// A TCP port with nothing listening on it: bound, its address read, then
/// released. Racy in principle (something else could claim it), but nothing
/// else in this test binary binds ports, and the alternative — a hard-coded
/// port — is racy against the whole machine.
fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binding an ephemeral port succeeds");
    let port = listener
        .local_addr()
        .expect("a bound listener has an address")
        .port();
    drop(listener);
    port
}

#[tokio::test]
async fn a_400_error_envelope_maps_to_an_api_error_carrying_all_four_fields() {
    let (server, _front, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/payment_intents"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "type": "invalid_request_error",
                "code": "parameter_invalid_integer",
                "message": "amount must be an integer in minor units",
                "param": "amount",
            }
        })))
        .mount(&server)
        .await;

    let err = client
        .payment_intents()
        .create(Default::default(), Default::default())
        .await
        .unwrap_err();

    match err {
        Error::Api {
            status,
            kind,
            code,
            message,
            param,
        } => {
            assert_eq!(status, 400);
            assert_eq!(kind, "invalid_request_error");
            assert_eq!(code.as_deref(), Some("parameter_invalid_integer"));
            assert_eq!(message, "amount must be an integer in minor units");
            assert_eq!(param.as_deref(), Some("amount"));
        }
        other => panic!("expected an API error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_envelope_without_the_optional_fields_still_maps_to_an_api_error() {
    let (server, _front, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "error": { "type": "invalid_request_error", "message": "Unrecognized request URL." }
        })))
        .mount(&server)
        .await;

    match probe(&client).await.unwrap_err() {
        Error::Api {
            status,
            code,
            param,
            ..
        } => {
            assert_eq!(status, 404);
            assert!(code.is_none());
            assert!(param.is_none());
        }
        other => panic!("expected an API error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_proxy_html_502_is_an_unsigned_response_and_its_body_is_never_read() {
    // Something in front of vpay answered, not vpay: no COSE message, whatever
    // the status. The SDK says so and reads nothing from the body — a body
    // nobody vouched for must not become an error message a merchant trusts.
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .respond_with(
            ResponseTemplate::new(502)
                .set_body_string("<html><body><h1>502 Bad Gateway</h1></body></html>")
                .insert_header("content-type", "text/html"),
        )
        .mount(&upstream)
        .await;
    let client = Client::builder(upstream.uri())
        .credentials(front::merchant_credentials())
        .server_keys(front::server_keys())
        .build()
        .unwrap();

    match probe(&client).await.unwrap_err() {
        Error::Unsigned { status } => assert_eq!(status, 502),
        other => panic!("expected an unsigned-response error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_oversized_error_body_is_truncated_to_a_bounded_prefix() {
    let (server, _front, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .respond_with(ResponseTemplate::new(500).set_body_raw(
            format!("\"{}\"", "x".repeat(10_000)).into_bytes(),
            "application/json",
        ))
        .mount(&server)
        .await;

    match probe(&client).await.unwrap_err() {
        Error::UnexpectedResponse { body_prefix, .. } => {
            assert_eq!(
                body_prefix.len(),
                500,
                "an unbounded upstream must not size this error value"
            );
        }
        other => panic!("expected an unexpected-response error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_oversized_multibyte_error_body_is_cut_on_a_character_boundary() {
    // The bound is on bytes, and `bounded_prefix` slices bytes — so a body of
    // multi-byte text is exactly where a naive cut goes wrong. Two things are
    // asserted: the bound still holds (an unbounded upstream cannot size this
    // error value), and the character straddling the cut is *dropped* rather
    // than rendered as U+FFFD — matching `boundedBodyPrefix` in
    // `sdks/nodejs/src/errors.ts`, which decodes in streaming mode and never
    // flushes. A replacement character here would be indistinguishable from
    // one the upstream actually sent.
    let (server, _front, client) = fixture().await;
    // "€" is 3 bytes: the quote and 166 whole characters fill 499 bytes and
    // the 167th straddles the 500-byte cut.
    // A JSON string literal, because the envelope layer seals only what is
    // valid for the type it was declared as: the opening quote is byte 0.
    let body = format!("\"{}\"", "€".repeat(1_000));
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .respond_with(
            ResponseTemplate::new(500).set_body_raw(body.into_bytes(), "application/json"),
        )
        .mount(&server)
        .await;

    match probe(&client).await.unwrap_err() {
        Error::UnexpectedResponse { body_prefix, .. } => {
            assert!(
                body_prefix.len() <= 500,
                "the byte bound must hold for multibyte bodies, got {}",
                body_prefix.len()
            );
            assert_eq!(body_prefix.len(), 499, "cut on the character boundary");
            assert_eq!(body_prefix.chars().count(), 167);
            assert!(
                !body_prefix.contains('\u{FFFD}'),
                "a straddling character must be dropped, not replaced: {body_prefix:?}"
            );
        }
        other => panic!("expected an unexpected-response error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_success_body_that_is_not_the_expected_object_is_an_unexpected_response() {
    let (server, _front, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "object": "event" })))
        .mount(&server)
        .await;

    match probe(&client).await.unwrap_err() {
        Error::UnexpectedResponse { status, .. } => assert_eq!(status, 200),
        other => panic!("expected an unexpected-response error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_refused_connection_is_a_transport_error() {
    let client = Client::builder(format!("http://127.0.0.1:{}", closed_port()))
        .credentials(front::merchant_credentials())
        .server_keys(front::server_keys())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();

    match probe(&client).await.unwrap_err() {
        Error::Transport(_) => {}
        other => panic!("expected a transport error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_bare_401_from_the_envelope_layer_is_unsigned_not_an_api_error() {
    // The layer's refusal for an unknown key, a bad signature, a stale clock
    // and a replay is one unsigned 401 (cratestack ADR 0006 §10). It must not be
    // dressed up as a Stripe-shaped `Error::Api`: nothing in it is vouched for.
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "code": "UNAUTHORIZED", "message": "unauthenticated", "details": null
        })))
        .mount(&upstream)
        .await;
    let client = Client::builder(upstream.uri())
        .credentials(front::merchant_credentials())
        .server_keys(front::server_keys())
        .build()
        .unwrap();

    match probe(&client).await.unwrap_err() {
        Error::Unsigned { status } => assert_eq!(status, 401),
        other => panic!("expected an unsigned-response error, got {other:?}"),
    }
}
