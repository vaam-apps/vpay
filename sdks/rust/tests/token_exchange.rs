//! The token half of the wire contract, asserted against the bytes on the
//! wire (`wiremock`): the exact form fields of the `client_credentials` +
//! `private_key_jwt` request, the `Authorization: Bearer` header it produces,
//! and the caching, single-flight and re-auth rules of
//! `docs/flows/merchant-auth.md` §3-4.
//!
//! `wiremock` is a real local HTTP server, not a mocked `reqwest`: the SDK's
//! own transport, header and encoding code runs unchanged. What this cannot
//! prove is that vpay's token endpoint behaves like this stub — no vpay
//! serves one (`docs/status.md`).

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

use serde_json::json;
use vpay_sdk::payment_intents::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, PaymentMethodType,
};
use vpay_sdk::{Client, Credentials, Error, RequestOptions};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

mod support;

const CLIENT_ID: &str = "merchant_acme";
const CLIENT_ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

static KEY: LazyLock<support::TestKey> = LazyLock::new(|| support::generate_key(Some("key-a")));

fn client_for(server: &MockServer) -> Client {
    Client::builder(server.uri())
        .credentials(
            Credentials::rsa_pem(CLIENT_ID, &KEY.pem)
                .unwrap()
                .with_kid("key-a"),
        )
        .build()
        .unwrap()
}

async fn mount_token_endpoint(server: &MockServer, expires_in: u64, expected_calls: u64) {
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::token_response("tok_1", expires_in)),
        )
        .expect(expected_calls)
        .mount(server)
        .await;
}

async fn mount_balance(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "balance",
            "available": [{ "amount": 5000, "currency": "xaf" }],
            "pending": [],
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn the_token_request_carries_exactly_the_documented_form_fields() {
    let server = MockServer::start().await;
    mount_token_endpoint(&server, 300, 1).await;
    mount_balance(&server).await;

    client_for(&server).balance().retrieve().await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let token_request = requests
        .iter()
        .find(|r| r.url.path() == "/v1/oauth/token")
        .expect("a token request was made");

    assert_eq!(
        token_request
            .headers
            .get("content-type")
            .map(|v| v.to_str().unwrap()),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        token_request
            .headers
            .get("accept")
            .map(|v| v.to_str().unwrap()),
        Some("application/json")
    );

    let pairs = support::form_pairs(&token_request.body);
    let names: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
    // Order and membership both: `docs/flows/merchant-auth.md`'s token-request
    // table, and `scope` omitted entirely because none was configured.
    assert_eq!(
        names,
        [
            "grant_type",
            "client_id",
            "client_assertion_type",
            "client_assertion",
            "audience"
        ]
    );
    assert_eq!(
        support::form_field(&pairs, "grant_type"),
        Some("client_credentials")
    );
    assert_eq!(support::form_field(&pairs, "client_id"), Some(CLIENT_ID));
    assert_eq!(
        support::form_field(&pairs, "client_assertion_type"),
        // The URN is percent-encoded on the wire; comparing the encoded form
        // is the point (see `support::form_pairs`).
        Some("urn%3Aietf%3Aparams%3Aoauth%3Aclient-assertion-type%3Ajwt-bearer")
    );
    assert_eq!(support::form_field(&pairs, "audience"), Some("vpay%3Av1"));

    // A three-segment compact JWS, not an empty placeholder.
    let assertion = support::form_field(&pairs, "client_assertion").expect("assertion is present");
    assert_eq!(assertion.split('.').count(), 3);

    // "No `client_secret`, ever": the OP rejects a request presenting more
    // than one client-authentication method.
    assert!(support::form_field(&pairs, "client_secret").is_none());

    // Sanity: the constant this SDK sends decodes to the RFC 7523 URN.
    assert_eq!(
        percent_decode(assertion_type_of(&pairs)),
        CLIENT_ASSERTION_TYPE
    );
}

fn assertion_type_of(pairs: &[(String, String)]) -> &str {
    support::form_field(pairs, "client_assertion_type").expect("client_assertion_type is present")
}

/// Minimal percent-decoder for the one assertion above — `%XX` only, which is
/// all this encoder ever emits (no `+`-for-space form).
fn percent_decode(input: &str) -> String {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes.get(i) {
            Some(b'%') => {
                let hex = input.get(i + 1..i + 3).expect("a %XX escape is complete");
                out.push(u8::from_str_radix(hex, 16).expect("a %XX escape is hex"));
                i += 3;
            }
            Some(b) => {
                out.push(*b);
                i += 1;
            }
            None => break,
        }
    }
    String::from_utf8(out).expect("decoded bytes are UTF-8")
}

#[tokio::test]
async fn the_access_token_is_presented_as_a_bearer_header_on_the_next_resource_call() {
    let server = MockServer::start().await;
    mount_token_endpoint(&server, 300, 1).await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .and(header("authorization", "Bearer tok_1"))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "balance",
            "available": [],
            "pending": [],
        })))
        .expect(1)
        .mount(&server)
        .await;

    client_for(&server).balance().retrieve().await.unwrap();
}

#[tokio::test]
async fn the_user_agent_names_this_sdk_and_its_version() {
    let server = MockServer::start().await;
    mount_token_endpoint(&server, 300, 1).await;
    mount_balance(&server).await;

    client_for(&server).balance().retrieve().await.unwrap();

    let expected = format!("vpay-sdk-rust/{}", env!("CARGO_PKG_VERSION"));
    for request in server.received_requests().await.unwrap() {
        assert_eq!(
            request
                .headers
                .get("user-agent")
                .map(|v| v.to_str().unwrap()),
            Some(expected.as_str()),
            "every request, token exchange included, names the SDK"
        );
    }
}

#[tokio::test]
async fn a_cached_token_is_reused_across_calls() {
    let server = MockServer::start().await;
    // `.expect(1)`: a second token request here is the failure. Verified when
    // the `MockServer` is dropped at the end of this test.
    mount_token_endpoint(&server, 300, 1).await;
    mount_balance(&server).await;

    let client = client_for(&server);
    for _ in 0..3 {
        client.balance().retrieve().await.unwrap();
    }
}

#[tokio::test]
async fn an_expired_token_is_refreshed() {
    let server = MockServer::start().await;
    // `expires_in: 1` ⇒ margin = min(30, 1/2) = 0 ⇒ usable for 1s.
    mount_token_endpoint(&server, 1, 2).await;
    mount_balance(&server).await;

    let client = client_for(&server);
    client.balance().retrieve().await.unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    client.balance().retrieve().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_calls_share_one_token_request() {
    let server = MockServer::start().await;
    // The single-flight property: eight callers racing from a cold cache must
    // spend one assertion `jti`, not eight (`docs/flows/merchant-auth.md` §3).
    // A 200ms delay on the token response widens the window they must
    // actually contend in — without it the first call could plausibly finish
    // before the others start, and this test would pass without proving
    // anything.
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(support::token_response("tok_1", 300))
                .set_delay(Duration::from_millis(200)),
        )
        .expect(1)
        .mount(&server)
        .await;
    mount_balance(&server).await;

    let client = client_for(&server);
    // `.collect()` before awaiting, deliberately: `Iterator::map` is lazy, so
    // consuming it inside the `for` below would spawn each task only as the
    // previous one was awaited. The eight calls would then run strictly one
    // after another, the second onwards would find a warm cache, and this
    // test would pass with the single-flight lock removed entirely — which is
    // exactly what it happened to do before this line existed.
    let calls: Vec<_> = (0..8)
        .map(|_| {
            let client = client.clone();
            tokio::spawn(async move { client.balance().retrieve().await })
        })
        .collect();
    for handle in calls {
        handle.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn a_401_from_a_resource_route_triggers_exactly_one_reauth_and_retry() {
    let server = MockServer::start().await;
    // Two token requests: the first cold, the second after the 401
    // invalidated the cache.
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::token_response("tok_stale", 300)),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::token_response("tok_fresh", 300)),
        )
        .expect(1)
        .mount(&server)
        .await;

    // The stale token is refused; the fresh one is accepted. Matching on the
    // header, not on call order, is what proves the retry used a *new* token
    // rather than replaying the old one.
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .and(header("authorization", "Bearer tok_stale"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "type": "invalid_request_error", "code": "invalid_token", "message": "expired" }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .and(header("authorization", "Bearer tok_fresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "balance",
            "available": [],
            "pending": [],
        })))
        .expect(1)
        .mount(&server)
        .await;

    let balance = client_for(&server).balance().retrieve().await.unwrap();
    assert_eq!(balance.object, "balance");
}

/// Mounts a token endpoint that answers `tok_stale` once and then
/// `tok_fresh`, asserting exactly one of each — the shape a single re-auth
/// produces.
async fn mount_staleness_then_freshness(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::token_response("tok_stale", 300)),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::token_response("tok_fresh", 300)),
        )
        .expect(1)
        .mount(server)
        .await;
}

/// A `POST` route that refuses `tok_stale` with a `401` and accepts
/// `tok_fresh`, exactly once each.
async fn mount_post_that_401s_once(server: &MockServer, route: &str) {
    Mock::given(method("POST"))
        .and(path(route.to_string()))
        .and(header("authorization", "Bearer tok_stale"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "type": "invalid_request_error", "code": "invalid_token", "message": "expired" }
        })))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path(route.to_string()))
        .and(header("authorization", "Bearer tok_fresh"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .expect(1)
        .mount(server)
        .await;
}

/// The two recorded requests to `route`, in wire order — the original and the
/// post-re-auth retry.
async fn the_two_attempts(server: &MockServer, route: &str) -> (Request, Request) {
    let mut attempts: Vec<Request> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == route)
        .collect();
    assert_eq!(
        attempts.len(),
        2,
        "expected the original and exactly one retry to {route}"
    );
    let second = attempts.remove(1);
    let first = attempts.remove(0);
    (first, second)
}

fn idempotency_key(request: &Request) -> String {
    request
        .headers
        .get("idempotency-key")
        .expect("every POST carries an Idempotency-Key")
        .to_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_reauthed_post_replays_the_callers_own_idempotency_key_and_body() {
    // The whole point of the `Idempotency-Key` header is that a retry of the
    // same logical operation is not a second operation. A re-auth that minted
    // a *new* key would turn one create into two charges the first time a
    // token expired mid-call — the single failure this header exists to
    // prevent, reached through the SDK's own retry rather than the network's.
    let server = MockServer::start().await;
    mount_staleness_then_freshness(&server).await;
    mount_post_that_401s_once(&server, "/v1/payment_intents").await;

    client_for(&server)
        .payment_intents()
        .create(
            CreatePaymentIntentParams {
                amount: 5000,
                currency: "xaf".to_string(),
                payment_method_types: vec![PaymentMethodType::MtnMomo],
                ..Default::default()
            },
            RequestOptions::new().with_idempotency_key("order-1234-create"),
        )
        .await
        .unwrap();

    let (first, second) = the_two_attempts(&server, "/v1/payment_intents").await;
    assert_eq!(idempotency_key(&first), "order-1234-create");
    assert_eq!(idempotency_key(&second), "order-1234-create");
    assert_eq!(first.body, second.body);
    assert_eq!(
        String::from_utf8(second.body.clone()).unwrap(),
        "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo"
    );
    // And the retry did use the new token — otherwise the assertion above
    // would be satisfied by two identical, identically-failing calls.
    assert_eq!(
        second
            .headers
            .get("authorization")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer tok_fresh")
    );
}

#[tokio::test]
async fn a_reauthed_post_replays_the_generated_idempotency_key_too() {
    // The generated case is the one that can silently regress: nothing in the
    // caller's code names the key, so a second UUIDv4 on the retry looks like
    // nothing at all from the outside.
    let server = MockServer::start().await;
    mount_staleness_then_freshness(&server).await;
    mount_post_that_401s_once(&server, "/v1/payment_intents/pi_1/cancel").await;

    client_for(&server)
        .payment_intents()
        .cancel("pi_1", RequestOptions::new())
        .await
        .unwrap();

    let (first, second) = the_two_attempts(&server, "/v1/payment_intents/pi_1/cancel").await;
    let key = idempotency_key(&first);
    assert_eq!(
        uuid::Uuid::parse_str(&key)
            .expect("the generated key is a UUID")
            .get_version_num(),
        4
    );
    assert_eq!(
        idempotency_key(&second),
        key,
        "the retry must replay the key generated for the first attempt"
    );
    assert_eq!(first.body, second.body);
}

#[tokio::test]
async fn a_reauthed_confirm_replays_its_nested_body_byte_for_byte() {
    // A nested body is where a rebuild-on-retry would be most likely to drift
    // — and a confirm is the call that actually moves money.
    let server = MockServer::start().await;
    mount_staleness_then_freshness(&server).await;
    mount_post_that_401s_once(&server, "/v1/payment_intents/pi_1/confirm").await;

    client_for(&server)
        .payment_intents()
        .confirm(
            "pi_1",
            ConfirmPaymentIntentParams::mtn_momo("237670000000"),
            RequestOptions::new().with_idempotency_key("order-1234-confirm"),
        )
        .await
        .unwrap();

    let (first, second) = the_two_attempts(&server, "/v1/payment_intents/pi_1/confirm").await;
    assert_eq!(idempotency_key(&first), "order-1234-confirm");
    assert_eq!(idempotency_key(&second), "order-1234-confirm");
    assert_eq!(
        String::from_utf8(first.body.clone()).unwrap(),
        "payment_method_data[type]=mtn_momo&payment_method_data[mtn_momo][msisdn]=237670000000"
    );
    assert_eq!(first.body, second.body);
}

#[tokio::test]
async fn a_second_consecutive_401_is_returned_to_the_caller() {
    let server = MockServer::start().await;
    mount_token_endpoint(&server, 300, 2).await;
    // Always 401: the SDK must retry once and then stop, not loop.
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "type": "invalid_request_error", "code": "invalid_token", "message": "nope" }
        })))
        .expect(2)
        .mount(&server)
        .await;

    let err = client_for(&server).balance().retrieve().await.unwrap_err();
    match err {
        Error::Api {
            status,
            code,
            message,
            ..
        } => {
            assert_eq!(status, 401);
            assert_eq!(code.as_deref(), Some("invalid_token"));
            assert_eq!(message, "nope");
        }
        other => panic!("expected an API error carrying the 401, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_concurrent_401_does_not_discard_the_token_the_first_one_just_fetched() {
    // Two callers holding the same token are refused a moment apart. The
    // first re-authenticates and stores a fresh token; the second's `401`
    // then arrives carrying the *stale* token. Discarding the cache
    // unconditionally at that point throws away a token that is valid and was
    // never refused — spending another assertion `jti` for nothing, and doing
    // it again for every further caller that was mid-flight.
    //
    // The ordering is forced, not raced: the second route's `401` is delayed
    // past the point where the first caller's refresh has completed. Exactly
    // two token requests is the assertion — a third means the cache was
    // cleared behind the first caller's back — and it is enforced by mounting
    // exactly two token responses (`.expect(1)` each) with no fallback, so a
    // third request gets no match at all.
    let server = MockServer::start().await;
    mount_staleness_then_freshness(&server).await;

    // Route A: refused immediately, so its re-auth is the one that runs first.
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .and(header("authorization", "Bearer tok_stale"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "type": "invalid_request_error", "code": "invalid_token", "message": "expired" }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .and(header("authorization", "Bearer tok_fresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "balance",
            "available": [],
            "pending": [],
        })))
        .expect(1)
        .mount(&server)
        .await;

    // Route B: its `401` is held back 700ms — comfortably past route A's
    // whole re-auth — so B invalidates *after* `tok_fresh` is in the cache.
    Mock::given(method("GET"))
        .and(path("/v1/payment_intents/pi_1"))
        .and(header("authorization", "Bearer tok_stale"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({
                    "error": { "type": "invalid_request_error", "code": "invalid_token", "message": "expired" }
                }))
                .set_delay(Duration::from_millis(700)),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/payment_intents/pi_1"))
        .and(header("authorization", "Bearer tok_fresh"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server);
    let a = {
        let client = client.clone();
        tokio::spawn(async move { client.balance().retrieve().await })
    };
    let b = {
        let client = client.clone();
        tokio::spawn(async move { client.payment_intents().retrieve("pi_1").await })
    };

    a.await
        .unwrap()
        .expect("the first caller re-authenticates and succeeds");
    b.await
        .unwrap()
        .expect("the second caller reuses the token the first one fetched");
}

#[tokio::test]
async fn a_token_endpoint_rejection_surfaces_as_a_token_error_and_is_never_retried() {
    let server = MockServer::start().await;
    // `.expect(1)`: the token endpoint's own failure is not a re-auth
    // trigger (`docs/flows/merchant-auth.md` §4).
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": "invalid_client",
            "error_description": "assertion signature did not verify",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let err = client_for(&server).balance().retrieve().await.unwrap_err();
    match err {
        Error::TokenEndpoint { error, description } => {
            assert_eq!(error, "invalid_client");
            assert_eq!(
                description.as_deref(),
                Some("assertion signature did not verify")
            );
        }
        other => panic!("expected a token-endpoint error, got {other:?}"),
    }

    // And no resource call was attempted without a token.
    let requests = server.received_requests().await.unwrap();
    assert!(requests.iter().all(|r| r.url.path() == "/v1/oauth/token"));
}

#[tokio::test]
async fn a_configured_scope_is_sent_and_an_unconfigured_one_is_omitted() {
    let server = MockServer::start().await;
    mount_token_endpoint(&server, 300, 1).await;
    mount_balance(&server).await;

    let client = Client::builder(server.uri())
        .credentials(Credentials::rsa_pem(CLIENT_ID, &KEY.pem).unwrap())
        .scope("payments:write")
        .build()
        .unwrap();
    client.balance().retrieve().await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let token_request = requests
        .iter()
        .find(|r| r.url.path() == "/v1/oauth/token")
        .expect("a token request was made");
    let pairs = support::form_pairs(&token_request.body);
    assert_eq!(
        support::form_field(&pairs, "scope"),
        Some("payments%3Awrite")
    );
    // `scope` comes last, after `audience` — the omission case is covered by
    // the field-order assertion in the first test in this file.
    assert_eq!(pairs.last().map(|(k, _)| k.as_str()), Some("scope"));
}

#[tokio::test]
async fn the_assertion_audience_follows_an_overridden_token_endpoint() {
    // The assertion's `aud` is the token endpoint, not the issuer and not the
    // resource base — so overriding the endpoint must move the `aud` with it,
    // or every assertion would be minted for a URL nobody serves.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/custom/oauth2/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::token_response("tok_1", 300)),
        )
        .expect(1)
        .mount(&server)
        .await;
    mount_balance(&server).await;

    let token_endpoint = format!("{}/custom/oauth2/token", server.uri());
    let client = Client::builder(server.uri())
        .credentials(Credentials::rsa_pem(CLIENT_ID, &KEY.pem).unwrap())
        .token_endpoint(&token_endpoint)
        .build()
        .unwrap();
    client.balance().retrieve().await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let token_request = requests
        .iter()
        .find(|r| r.url.path() == "/custom/oauth2/token")
        .expect("the overridden token endpoint was called");
    let pairs = support::form_pairs(&token_request.body);
    let assertion = support::form_field(&pairs, "client_assertion").unwrap();
    let payload = decode_jwt_payload(&percent_decode(assertion));
    assert_eq!(
        payload.get("aud").and_then(|v| v.as_str()),
        Some(token_endpoint.as_str())
    );
}

#[tokio::test]
async fn an_explicit_assertion_audience_is_signed_without_moving_the_request() {
    // The defect this option fixes: the URL reachable from the merchant's own
    // server and the string the OP calls itself are two different facts. The
    // request must still go to the reachable address; only `aud` moves.
    let server = MockServer::start().await;
    mount_token_endpoint(&server, 300, 1).await;
    mount_balance(&server).await;

    let public_token_endpoint = "http://localhost:8080/v1/oauth/token";
    let client = Client::builder(server.uri())
        .credentials(Credentials::rsa_pem(CLIENT_ID, &KEY.pem).unwrap())
        .assertion_audience(public_token_endpoint)
        .build()
        .unwrap();
    client.balance().retrieve().await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let token_request = requests
        .iter()
        .find(|r| r.url.path() == "/v1/oauth/token")
        .expect("the reachable token endpoint was called, not the audience");
    // The request went to the mock server's own address — nothing was sent to
    // `public_token_endpoint`, which nothing in this test serves.
    assert!(server.uri().starts_with("http://127.0.0.1:"));

    let pairs = support::form_pairs(&token_request.body);
    let assertion = support::form_field(&pairs, "client_assertion").unwrap();
    let payload = decode_jwt_payload(&percent_decode(assertion));
    assert_eq!(
        payload.get("aud").and_then(|v| v.as_str()),
        Some(public_token_endpoint)
    );
    // Still not the `audience` form field, which stays `vpay:v1` (percent-
    // encoded on the wire — `form_pairs` hands back raw bytes on purpose).
    assert_eq!(
        percent_decode(support::form_field(&pairs, "audience").unwrap()),
        "vpay:v1"
    );
}

#[tokio::test]
async fn an_unset_assertion_audience_signs_the_url_the_request_went_to() {
    // The default, pinned: unchanged for every merchant whose server reaches
    // vpay at the URL vpay publishes as its own.
    let server = MockServer::start().await;
    mount_token_endpoint(&server, 300, 1).await;
    mount_balance(&server).await;

    client_for(&server).balance().retrieve().await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let token_request = requests
        .iter()
        .find(|r| r.url.path() == "/v1/oauth/token")
        .expect("the token endpoint was called");
    let pairs = support::form_pairs(&token_request.body);
    let assertion = support::form_field(&pairs, "client_assertion").unwrap();
    let payload = decode_jwt_payload(&percent_decode(assertion));
    assert_eq!(
        payload.get("aud").and_then(|v| v.as_str()),
        Some(format!("{}/v1/oauth/token", server.uri()).as_str())
    );
}

fn decode_jwt_payload(jwt: &str) -> serde_json::Value {
    use base64::Engine as _;
    let payload = jwt.split('.').nth(1).expect("jwt has three segments");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .expect("jwt payload is base64url");
    serde_json::from_slice(&bytes).expect("jwt payload is JSON")
}
