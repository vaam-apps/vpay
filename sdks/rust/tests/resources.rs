//! Every `/v1` resource method, asserted at the byte level: the exact path,
//! the exact method, the exact encoded body or query string, the headers
//! `docs/flows/merchant-auth.md`'s "Headers" table requires, and the typed
//! decode of the response.
//!
//! The body assertions are string equality against a literal, not a
//! field-by-field comparison of a re-parsed body. That is deliberate: the
//! contract these SDKs share is the *bytes*, and the Node SDK's tests pin the
//! same strings. A re-parsing assertion would pass while the two SDKs sent
//! different bodies.

// See `tests/support/mod.rs` for why this allow list mirrors
// `backends/apps/vpay-server/tests/cli.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde_json::json;
use vpay_sdk::payment_intents::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, ListPaymentIntentsParams,
    PaymentMethodType,
};
use vpay_sdk::{
    Client, CreateRefundParams, Credentials, Error, IntentStatus, ListEventsParams, NextAction,
    RefundStatus, RequestOptions,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

mod support;

static KEY: LazyLock<support::TestKey> = LazyLock::new(|| support::generate_key(None));

/// A server with the token endpoint already mounted, plus a client pointed at
/// it. Every test here is about the *resource* call, so the handshake is
/// setup rather than subject matter (it has its own file).
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

/// The single recorded request for `path`, panicking if there was not exactly
/// one — "the SDK sent this, and only this" is the claim under test.
async fn only_request(server: &MockServer, wanted: &str) -> Request {
    let requests = server.received_requests().await.unwrap();
    let mut matching: Vec<Request> = requests
        .into_iter()
        .filter(|r| r.url.path() == wanted)
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one request to {wanted}, got {}",
        matching.len()
    );
    matching.remove(0)
}

fn body_string(request: &Request) -> String {
    String::from_utf8(request.body.clone()).expect("request body is UTF-8")
}

fn header_value(request: &Request, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
}

#[tokio::test]
async fn create_payment_intent_sends_the_documented_body_and_decodes_the_object() {
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/payment_intents"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut metadata = BTreeMap::new();
    metadata.insert("order_id".to_string(), "1234".to_string());
    let intent = client
        .payment_intents()
        .create(
            CreatePaymentIntentParams {
                amount: 5000,
                // Upper-cased deliberately: the wire contract says lowercase,
                // and the SDK normalises rather than passing it through.
                currency: "XAF".to_string(),
                payment_method_types: vec![
                    PaymentMethodType::MtnMomo,
                    PaymentMethodType::OrangeMoney,
                ],
                metadata,
                description: Some("Order #42".to_string()),
            },
            RequestOptions::new().with_idempotency_key("idem_abc"),
        )
        .await
        .unwrap();

    assert_eq!(intent.id, "pi_1");
    assert_eq!(intent.amount, 5000);
    assert_eq!(intent.status, IntentStatus::RequiresPaymentMethod);
    assert_eq!(
        intent.metadata.get("order_id").map(String::as_str),
        Some("1234")
    );

    let request = only_request(&server, "/v1/payment_intents").await;
    assert_eq!(request.method.as_str(), "POST");
    assert_eq!(
        body_string(&request),
        "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo\
         &payment_method_types[1]=orange_money&metadata[order_id]=1234\
         &description=Order%20%2342"
    );
    assert_eq!(
        header_value(&request, "content-type").as_deref(),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        header_value(&request, "idempotency-key").as_deref(),
        Some("idem_abc")
    );
}

#[tokio::test]
async fn a_post_without_a_caller_supplied_key_generates_a_uuid_v4_idempotency_key() {
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/payment_intents"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .mount(&server)
        .await;

    client
        .payment_intents()
        .create(
            CreatePaymentIntentParams {
                amount: 5000,
                currency: "xaf".to_string(),
                payment_method_types: vec![PaymentMethodType::MtnMomo],
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .unwrap();

    let request = only_request(&server, "/v1/payment_intents").await;
    let key = header_value(&request, "idempotency-key").expect("every POST carries one");
    let parsed = uuid::Uuid::parse_str(&key).expect("the generated key is a UUID");
    assert_eq!(parsed.get_version_num(), 4);
}

#[tokio::test]
async fn create_omits_absent_optional_fields_rather_than_sending_them_empty() {
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/payment_intents"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .mount(&server)
        .await;

    client
        .payment_intents()
        .create(
            CreatePaymentIntentParams {
                amount: 5000,
                currency: "xaf".to_string(),
                payment_method_types: vec![PaymentMethodType::MtnMomo],
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .unwrap();

    let request = only_request(&server, "/v1/payment_intents").await;
    assert_eq!(
        body_string(&request),
        "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo"
    );
}

#[tokio::test]
async fn retrieve_payment_intent_is_a_get_with_no_body_and_decodes_next_action() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/payment_intents/pi_redirect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "pi_redirect",
            "object": "payment_intent",
            "amount": 12_000,
            "currency": "xaf",
            "status": "requires_action",
            "payment_method_types": ["orange_money"],
            "next_action": {
                "type": "redirect_to_url",
                "redirect_to_url": {
                    "url": "https://rail.example/pay/abc",
                    "return_url": "https://merchant.example/return"
                }
            },
            "last_payment_error": null,
            "metadata": {},
            "description": null,
            "created": 1_753_401_600,
            "livemode": true,
        })))
        .expect(1)
        .mount(&server)
        .await;

    let intent = client
        .payment_intents()
        .retrieve("pi_redirect")
        .await
        .unwrap();

    assert_eq!(intent.status, IntentStatus::RequiresAction);
    match intent.next_action {
        Some(NextAction::RedirectToUrl { redirect_to_url }) => {
            assert_eq!(redirect_to_url.url, "https://rail.example/pay/abc");
            assert_eq!(
                redirect_to_url.return_url.as_deref(),
                Some("https://merchant.example/return")
            );
        }
        other => panic!("expected a redirect next_action, got {other:?}"),
    }

    let request = only_request(&server, "/v1/payment_intents/pi_redirect").await;
    assert_eq!(request.method.as_str(), "GET");
    assert!(request.body.is_empty());
    assert!(header_value(&request, "idempotency-key").is_none());
    assert_eq!(request.url.query(), None);
}

#[tokio::test]
async fn a_failed_charge_decodes_as_last_payment_error_with_no_failed_status() {
    // `docs/flows/payment-lifecycle.md`: there is no `failed` intent status.
    // A rail failure comes back as `requires_payment_method` plus
    // `last_payment_error`, and the SDK must model exactly that.
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/payment_intents/pi_failed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "pi_failed",
            "object": "payment_intent",
            "amount": 5000,
            "currency": "xaf",
            "status": "requires_payment_method",
            "payment_method_types": ["mtn_momo"],
            "next_action": null,
            "last_payment_error": { "code": "insufficient_funds", "message": "not enough funds" },
            "metadata": {},
            "description": null,
            "created": 1_753_401_600,
            "livemode": false,
        })))
        .mount(&server)
        .await;

    let intent = client
        .payment_intents()
        .retrieve("pi_failed")
        .await
        .unwrap();
    assert_eq!(intent.status, IntentStatus::RequiresPaymentMethod);
    let error = intent.last_payment_error.expect("the failure is carried");
    assert_eq!(error.code, "insufficient_funds");
}

#[tokio::test]
async fn confirm_sends_the_push_rail_instrument() {
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/payment_intents/pi_1/confirm"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .expect(1)
        .mount(&server)
        .await;

    client
        .payment_intents()
        .confirm(
            "pi_1",
            ConfirmPaymentIntentParams::mtn_momo("237670000000"),
            RequestOptions::new(),
        )
        .await
        .unwrap();

    let request = only_request(&server, "/v1/payment_intents/pi_1/confirm").await;
    assert_eq!(request.method.as_str(), "POST");
    assert_eq!(
        body_string(&request),
        "payment_method_data[type]=mtn_momo&payment_method_data[mtn_momo][msisdn]=237670000000"
    );
}

#[tokio::test]
async fn confirm_sends_the_redirect_rail_return_url() {
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/payment_intents/pi_1/confirm"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .mount(&server)
        .await;

    client
        .payment_intents()
        .confirm(
            "pi_1",
            ConfirmPaymentIntentParams::orange_money("https://m.example/return?x=1"),
            RequestOptions::new(),
        )
        .await
        .unwrap();

    let request = only_request(&server, "/v1/payment_intents/pi_1/confirm").await;
    assert_eq!(
        body_string(&request),
        "payment_method_data[type]=orange_money\
         &return_url=https%3A%2F%2Fm.example%2Freturn%3Fx%3D1"
    );
}

#[tokio::test]
async fn cancel_posts_an_empty_body_and_still_carries_an_idempotency_key() {
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/payment_intents/pi_1/cancel"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .expect(1)
        .mount(&server)
        .await;

    client
        .payment_intents()
        .cancel("pi_1", RequestOptions::new())
        .await
        .unwrap();

    let request = only_request(&server, "/v1/payment_intents/pi_1/cancel").await;
    assert_eq!(body_string(&request), "");
    assert!(header_value(&request, "idempotency-key").is_some());
}

#[tokio::test]
async fn list_payment_intents_encodes_its_pagination_into_the_query_string() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/payment_intents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [support::payment_intent_json("pi_1")],
            "has_more": true,
            "url": "/v1/payment_intents",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let list = client
        .payment_intents()
        .list(ListPaymentIntentsParams {
            limit: Some(2),
            starting_after: Some("pi_0".to_string()),
            ending_before: None,
        })
        .await
        .unwrap();

    assert_eq!(list.object, "list");
    assert!(list.has_more);
    assert_eq!(list.data.len(), 1);

    let request = only_request(&server, "/v1/payment_intents").await;
    assert_eq!(request.method.as_str(), "GET");
    assert_eq!(request.url.query(), Some("limit=2&starting_after=pi_0"));
    assert!(request.body.is_empty());
}

#[tokio::test]
async fn a_list_call_with_no_parameters_sends_no_query_string_at_all() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/payment_intents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [],
            "has_more": false,
            "url": "/v1/payment_intents",
        })))
        .mount(&server)
        .await;

    client
        .payment_intents()
        .list(ListPaymentIntentsParams::default())
        .await
        .unwrap();

    // Not `?` with an empty query — a bare path.
    let request = only_request(&server, "/v1/payment_intents").await;
    assert_eq!(request.url.query(), None);
}

#[tokio::test]
async fn create_refund_sends_the_documented_body_and_decodes_the_object() {
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/refunds"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "re_1",
            "object": "refund",
            "amount": 2500,
            "currency": "xaf",
            "payment_intent": "pi_1",
            "status": "pending",
            "reason": "requested_by_customer",
            "metadata": {},
            "created": 1_753_401_600,
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut metadata = BTreeMap::new();
    metadata.insert("case".to_string(), "77".to_string());
    let refund = client
        .refunds()
        .create(
            CreateRefundParams {
                payment_intent: "pi_1".to_string(),
                amount: Some(2500),
                reason: Some("requested_by_customer".to_string()),
                metadata,
            },
            RequestOptions::new(),
        )
        .await
        .unwrap();

    assert_eq!(refund.id, "re_1");
    assert_eq!(refund.status, RefundStatus::Pending);

    let request = only_request(&server, "/v1/refunds").await;
    assert_eq!(
        body_string(&request),
        "payment_intent=pi_1&amount=2500&reason=requested_by_customer&metadata[case]=77"
    );
}

#[tokio::test]
async fn a_full_refund_omits_the_amount_entirely() {
    // "omit for full" — `amount=` and no `amount` are different requests.
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/refunds"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "re_1",
            "object": "refund",
            "amount": 5000,
            "currency": "xaf",
            "payment_intent": "pi_1",
            "status": "succeeded",
            "reason": null,
            "metadata": {},
            "created": 1_753_401_600,
        })))
        .mount(&server)
        .await;

    client
        .refunds()
        .create(
            CreateRefundParams {
                payment_intent: "pi_1".to_string(),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .unwrap();

    let request = only_request(&server, "/v1/refunds").await;
    assert_eq!(body_string(&request), "payment_intent=pi_1");
}

#[tokio::test]
async fn list_events_filters_by_type_and_keeps_data_object_as_raw_json() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [
                {
                    "id": "evt_1",
                    "object": "event",
                    "type": "payment_intent.succeeded",
                    "created": 1_753_401_600,
                    "livemode": false,
                    "data": { "object": support::payment_intent_json("pi_1") },
                },
                {
                    // An event carrying an object this SDK does not model must
                    // still be deliverable, not a decode failure.
                    "id": "evt_2",
                    "object": "event",
                    "type": "some.future.type",
                    "created": 1_753_401_601,
                    "livemode": false,
                    "data": { "object": { "id": "xx_1", "object": "not_yet_modelled" } },
                }
            ],
            "has_more": false,
            "url": "/v1/events",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let list = client
        .events()
        .list(ListEventsParams {
            limit: Some(10),
            event_type: Some("payment_intent.succeeded".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(list.data.len(), 2);
    let first = &list.data[0];
    assert_eq!(first.kind, "payment_intent.succeeded");
    assert_eq!(first.payment_intent().unwrap().id, "pi_1");
    // The unmodelled one decodes as an event and only fails when a caller
    // asks for a shape it does not have.
    let second = &list.data[1];
    assert_eq!(second.kind, "some.future.type");
    assert!(second.payment_intent().is_err());
    assert_eq!(
        second.data.object.get("object").and_then(|v| v.as_str()),
        Some("not_yet_modelled")
    );

    let request = only_request(&server, "/v1/events").await;
    assert_eq!(
        request.url.query(),
        Some("limit=10&type=payment_intent.succeeded")
    );
}

#[tokio::test]
async fn an_id_with_url_metacharacters_is_percent_encoded_into_the_path() {
    // An id is merchant-controlled input. Unescaped, `/` would move the
    // request to a different route and `?`/`#` would truncate the path — so
    // `pi_a/b?c#d` must address one segment named exactly that, not
    // `/payment_intents/pi_a/b` with a query. `encodeURIComponent` is what
    // the Node SDK applies here, and this SDK's encoder reproduces it.
    let (server, client) = fixture().await;
    let raw_id = "pi_a/b?c#d e";
    let encoded = "pi_a%2Fb%3Fc%23d%20e";
    Mock::given(method("GET"))
        .and(path(format!("/v1/payment_intents/{encoded}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .expect(1)
        .mount(&server)
        .await;

    client.payment_intents().retrieve(raw_id).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let resource: Vec<&str> = requests
        .iter()
        .map(|r| r.url.path())
        .filter(|p| p.starts_with("/v1/payment_intents"))
        .collect();
    // `Url::path()` gives the path still percent-encoded, so this asserts the
    // bytes that went out, not a decoded interpretation of them.
    assert_eq!(resource, vec![format!("/v1/payment_intents/{encoded}")]);
}

#[tokio::test]
async fn confirm_and_cancel_encode_the_id_too() {
    let (server, client) = fixture().await;
    let raw_id = "pi_a/b";
    Mock::given(method("POST"))
        .and(path("/v1/payment_intents/pi_a%2Fb/confirm"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/payment_intents/pi_a%2Fb/cancel"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .expect(1)
        .mount(&server)
        .await;

    client
        .payment_intents()
        .confirm(
            raw_id,
            ConfirmPaymentIntentParams::mtn_momo("237670000000"),
            RequestOptions::new(),
        )
        .await
        .unwrap();
    client
        .payment_intents()
        .cancel(raw_id, RequestOptions::new())
        .await
        .unwrap();
}

#[tokio::test]
async fn an_amount_outside_the_cross_sdk_safe_range_is_refused_before_any_request() {
    // Parity with `sdks/nodejs/src/validate.ts`: a negative amount, or one
    // past `Number.MAX_SAFE_INTEGER`, is refused rather than sent. "Before
    // any request" is half the point — a rejected amount must not spend an
    // assertion `jti` or an idempotency key, so the token endpoint must see
    // nothing either.
    let (server, client) = fixture().await;

    for amount in [-1_i64, 9_007_199_254_740_992, i64::MAX] {
        let err = client
            .payment_intents()
            .create(
                CreatePaymentIntentParams {
                    amount,
                    currency: "xaf".to_string(),
                    payment_method_types: vec![PaymentMethodType::MtnMomo],
                    ..Default::default()
                },
                RequestOptions::new(),
            )
            .await
            .unwrap_err();
        match err {
            Error::InvalidParams { param, .. } => assert_eq!(param, "amount"),
            other => panic!("expected an invalid-params error for {amount}, got {other:?}"),
        }
    }

    let err = client
        .refunds()
        .create(
            CreateRefundParams {
                payment_intent: "pi_1".to_string(),
                amount: Some(-1),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidParams { .. }), "{err:?}");

    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a refused amount must not reach the wire — not even the token endpoint"
    );
}

#[tokio::test]
async fn the_largest_safe_amount_is_still_sent() {
    // The boundary the check must *not* move: `2^53-1` is a legal amount in
    // both SDKs, and rejecting it would be a silent ceiling on how much a
    // merchant can charge.
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/payment_intents"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::payment_intent_json("pi_1")),
        )
        .expect(1)
        .mount(&server)
        .await;

    client
        .payment_intents()
        .create(
            CreatePaymentIntentParams {
                amount: 9_007_199_254_740_991,
                currency: "xaf".to_string(),
                payment_method_types: vec![PaymentMethodType::MtnMomo],
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .unwrap();

    let request = only_request(&server, "/v1/payment_intents").await;
    assert_eq!(
        body_string(&request),
        "amount=9007199254740991&currency=xaf&payment_method_types[0]=mtn_momo"
    );
}

#[tokio::test]
async fn retrieve_balance_is_a_bare_get_and_decodes_both_buckets() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "balance",
            "available": [{ "amount": 125_000, "currency": "xaf" }],
            "pending": [{ "amount": 5000, "currency": "xaf" }],
        })))
        .expect(1)
        .mount(&server)
        .await;

    let balance = client.balance().retrieve().await.unwrap();
    assert_eq!(balance.available.len(), 1);
    assert_eq!(balance.available[0].amount, 125_000);
    assert_eq!(balance.pending[0].currency, "xaf");

    let request = only_request(&server, "/v1/balance").await;
    assert_eq!(request.method.as_str(), "GET");
    assert_eq!(request.url.query(), None);
    assert!(request.body.is_empty());
}
