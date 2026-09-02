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
use std::sync::LazyLock;
use std::time::Duration;

use serde_json::json;
use vpay_sdk::{Client, Credentials, Error};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod support;

static KEY: LazyLock<support::TestKey> = LazyLock::new(|| support::generate_key(None));

async fn fixture() -> (MockServer, Client) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::token_response("tok_1", 300)),
        )
        .mount(&server)
        .await;
    let client = Client::builder(server.uri())
        .credentials(Credentials::rsa_pem("merchant_acme", &KEY.pem).unwrap())
        .build()
        .unwrap();
    (server, client)
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
    let (server, client) = fixture().await;
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
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "error": { "type": "invalid_request_error", "message": "Unrecognized request URL." }
        })))
        .mount(&server)
        .await;

    match client.balance().retrieve().await.unwrap_err() {
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
async fn a_proxy_html_502_maps_to_an_unexpected_response_with_the_status_and_body() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .respond_with(
            ResponseTemplate::new(502)
                .set_body_string("<html><body><h1>502 Bad Gateway</h1></body></html>")
                .insert_header("content-type", "text/html"),
        )
        .mount(&server)
        .await;

    match client.balance().retrieve().await.unwrap_err() {
        Error::UnexpectedResponse {
            status,
            body_prefix,
        } => {
            assert_eq!(status, 502);
            assert!(
                body_prefix.contains("502 Bad Gateway"),
                "the prefix must be diagnosable, got {body_prefix:?}"
            );
        }
        other => panic!("expected an unexpected-response error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_oversized_error_body_is_truncated_to_a_bounded_prefix() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .respond_with(ResponseTemplate::new(500).set_body_string("x".repeat(10_000)))
        .mount(&server)
        .await;

    match client.balance().retrieve().await.unwrap_err() {
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
    let (server, client) = fixture().await;
    // "€" is 3 bytes: 166 whole characters fill 498 bytes and the 167th
    // straddles the 500-byte cut.
    let body = "€".repeat(1_000);
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .respond_with(ResponseTemplate::new(500).set_body_string(body))
        .mount(&server)
        .await;

    match client.balance().retrieve().await.unwrap_err() {
        Error::UnexpectedResponse { body_prefix, .. } => {
            assert!(
                body_prefix.len() <= 500,
                "the byte bound must hold for multibyte bodies, got {}",
                body_prefix.len()
            );
            assert_eq!(body_prefix.len(), 498, "cut on the character boundary");
            assert_eq!(body_prefix.chars().count(), 166);
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
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "object": "balance" })))
        .mount(&server)
        .await;

    match client.balance().retrieve().await.unwrap_err() {
        Error::UnexpectedResponse { status, .. } => assert_eq!(status, 200),
        other => panic!("expected an unexpected-response error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_refused_connection_to_the_token_endpoint_is_a_transport_error() {
    let client = Client::builder(format!("http://127.0.0.1:{}", closed_port()))
        .credentials(Credentials::rsa_pem("merchant_acme", &KEY.pem).unwrap())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();

    match client.balance().retrieve().await.unwrap_err() {
        Error::Transport(_) => {}
        other => panic!("expected a transport error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_refused_connection_to_a_resource_route_is_a_transport_error() {
    // Token endpoint live, resource base dead: proves the transport mapping
    // is not something only the token path does.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::token_response("tok_1", 300)),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::builder(format!("http://127.0.0.1:{}", closed_port()))
        .credentials(Credentials::rsa_pem("merchant_acme", &KEY.pem).unwrap())
        .token_endpoint(format!("{}/v1/oauth/token", server.uri()))
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();

    match client.balance().retrieve().await.unwrap_err() {
        Error::Transport(_) => {}
        other => panic!("expected a transport error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_token_endpoint_returning_html_is_an_unexpected_response_not_a_token_error() {
    // The documented failure for "token endpoint path differs from the SDK
    // default": a 404 that is not an OAuth2 error object must not be dressed
    // up as one, or the merchant would go looking for a credential problem.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(ResponseTemplate::new(404).set_body_string("<html>not here</html>"))
        .mount(&server)
        .await;

    let client = Client::builder(server.uri())
        .credentials(Credentials::rsa_pem("merchant_acme", &KEY.pem).unwrap())
        .build()
        .unwrap();

    match client.balance().retrieve().await.unwrap_err() {
        Error::UnexpectedResponse { status, .. } => assert_eq!(status, 404),
        other => panic!("expected an unexpected-response error, got {other:?}"),
    }
}
