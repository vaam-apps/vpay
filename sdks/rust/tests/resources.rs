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

use serde_json::{Value, json};
use vpay_sdk::account_holders::RetrieveAccountHolderParams;
use vpay_sdk::checkout::{
    CheckoutPaymentStatus, CheckoutSessionStatus, CheckoutUiMode, CreateCheckoutSessionParams,
    ListCheckoutSessionsParams,
};
use vpay_sdk::payment_intents::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, ListPaymentIntentsParams,
    PaymentMethodType,
};
use vpay_sdk::{
    Client, CreateRefundParams, Credentials, Error, IntentStatus, KnownEventType, ListEventsParams,
    NextAction, RefundStatus, RequestOptions,
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
async fn create_surfaces_client_secret_when_the_server_sends_it() {
    // `POST /v1/payment_intents` renders `PaymentIntentWithSecret` (Step 5c's
    // D2) — the twelve documented keys plus `client_secret`.
    let (server, client) = fixture().await;
    let mut body = support::payment_intent_json("pi_1");
    body["client_secret"] = json!("pi_1_secret_abc123");
    Mock::given(method("POST"))
        .and(path("/v1/payment_intents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let intent = client
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

    assert_eq!(intent.client_secret.as_deref(), Some("pi_1_secret_abc123"));
}

#[tokio::test]
async fn retrieve_surfaces_client_secret_when_the_server_sends_it() {
    // `GET /v1/payment_intents/{id}` renders the same `PaymentIntentWithSecret`.
    let (server, client) = fixture().await;
    let mut body = support::payment_intent_json("pi_1");
    body["client_secret"] = json!("pi_1_secret_abc123");
    Mock::given(method("GET"))
        .and(path("/v1/payment_intents/pi_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let intent = client.payment_intents().retrieve("pi_1").await.unwrap();

    assert_eq!(intent.client_secret.as_deref(), Some("pi_1_secret_abc123"));
}

#[tokio::test]
async fn a_list_items_client_secret_is_none() {
    // `GET /v1/payment_intents` renders the plain `PaymentIntentObject` per
    // item — no `client_secret` key at all, unlike create/retrieve. A
    // merchant's own listing view must never receive a live payer
    // credential for every intent on the page.
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/payment_intents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [support::payment_intent_json("pi_1")],
            "has_more": false,
            "url": "/v1/payment_intents",
        })))
        .mount(&server)
        .await;

    let list = client
        .payment_intents()
        .list(ListPaymentIntentsParams::default())
        .await
        .unwrap();

    assert_eq!(list.data.len(), 1);
    assert_eq!(list.data[0].client_secret, None);
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

/// `checkout.session.expired` is in this SDK's event vocabulary, and its
/// payload decodes as a Checkout Session through the whole `events.list`
/// path — the object a merchant actually receives, not a hand-built one.
///
/// The **decisive** assertions are the last two: the delivered session
/// carries no `client_secret` and a `null` `url`, because both are live payer
/// credentials and an event body is stored, delivered at-least-once and
/// replayable (`vpay_api::model::CheckoutSessionObject::expired_snapshot`).
/// A server that started sending either would fail here rather than in a
/// merchant's log aggregator.
#[tokio::test]
async fn a_checkout_session_expired_event_is_a_known_type_and_decodes_as_a_session() {
    let (server, client) = fixture().await;
    // The event body the server actually emits: the session with `status`
    // already `expired`, `url` null, and no `client_secret` member at all.
    let mut object = support::checkout_session_json("cs_1", None);
    object["status"] = json!("expired");
    object["url"] = Value::Null;

    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [
                {
                    "id": "evt_9",
                    "object": "event",
                    "type": "checkout.session.expired",
                    "created": 1_753_401_600,
                    "livemode": false,
                    "data": { "object": object },
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
            event_type: Some(KnownEventType::CheckoutSessionExpired.to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    let event = &list.data[0];
    assert_eq!(event.kind, "checkout.session.expired");
    assert_eq!(
        KnownEventType::from_wire(&event.kind),
        Some(KnownEventType::CheckoutSessionExpired),
        "the type must be in this SDK's vocabulary, not merely a string"
    );
    // …and the vocabulary's spelling is what went out on the wire as the
    // filter, so a typo in the constant would fail here too.
    let request = only_request(&server, "/v1/events").await;
    assert_eq!(request.url.query(), Some("type=checkout.session.expired"));

    let session = event.checkout_session().unwrap();
    assert_eq!(session.id, "cs_1");
    assert_eq!(session.status, CheckoutSessionStatus::Expired);
    assert_eq!(session.payment_status, CheckoutPaymentStatus::Unpaid);
    // The session is `hosted`, so a reader must not infer the mode from a
    // null `url`: it is null because the credential in its fragment may not
    // be delivered, not because there was no page.
    assert_eq!(session.ui_mode, CheckoutUiMode::Hosted);
    assert_eq!(
        session.url, None,
        "a delivered session must not carry the url whose fragment is its credential"
    );
    assert_eq!(
        session.client_secret, None,
        "a delivered session must not carry its client_secret"
    );
    // Asserted on the serialised body, not only on the decoded struct: a
    // field this SDK does not model would still be in the bytes.
    let raw = serde_json::to_string(&event.data.object).unwrap();
    assert!(
        !raw.contains("_secret_"),
        "no credential in the payload: {raw}"
    );
}

/// An unknown type is not a decode failure, and the wrong accessor is an
/// error rather than a wrong answer.
///
/// The counterpart to the case above: `Event::kind` stays a `String`
/// precisely so a type this SDK version predates is still deliverable, and
/// `KnownEventType::from_wire` answering `None` is how a caller finds out.
#[test]
fn an_unknown_event_type_is_none_rather_than_a_failure_and_the_wrong_accessor_errs() {
    for (wire, expected) in [
        (
            "payment_intent.succeeded",
            Some(KnownEventType::PaymentIntentSucceeded),
        ),
        (
            "charge.refund.updated",
            Some(KnownEventType::ChargeRefundUpdated),
        ),
        (
            "checkout.session.expired",
            Some(KnownEventType::CheckoutSessionExpired),
        ),
        // Real Stripe types vpay does not document. Neither is an error.
        ("checkout.session.completed", None),
        ("some.future.type", None),
    ] {
        assert_eq!(KnownEventType::from_wire(wire), expected, "{wire}");
    }
    // Every variant round-trips through its own wire spelling, so a constant
    // that disagreed with `from_wire` could not pass.
    for known in [
        KnownEventType::PaymentIntentCreated,
        KnownEventType::PaymentIntentProcessing,
        KnownEventType::PaymentIntentSucceeded,
        KnownEventType::PaymentIntentPaymentFailed,
        KnownEventType::PaymentIntentCanceled,
        KnownEventType::ChargeRefunded,
        KnownEventType::ChargeRefundUpdated,
        KnownEventType::CheckoutSessionExpired,
    ] {
        assert_eq!(KnownEventType::from_wire(known.as_wire_str()), Some(known));
    }

    let event: vpay_sdk::Event = serde_json::from_value(json!({
        "id": "evt_9",
        "object": "event",
        "type": "checkout.session.expired",
        "created": 1_753_401_600,
        "livemode": false,
        "data": { "object": support::checkout_session_json("cs_1", None) },
    }))
    .unwrap();
    assert!(event.checkout_session().is_ok());
    assert!(
        event.payment_intent().is_err(),
        "asking for the wrong shape must fail rather than answer something plausible"
    );
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

// --------------------------------------------------------------------------
// /v1/account_holders — issue #47.
//
// These mirror `sdks/nodejs/src/client.test.ts`'s `accountHolders` block one
// for one, down to the encoded query string, for the reason the checkout
// block below states: ADR-0015's parity rule is about *wire semantics*, and
// only a byte-level assertion on both sides catches two SDKs that both
// "support account-holder lookup" while sending different query strings.
// --------------------------------------------------------------------------

#[tokio::test]
async fn retrieve_account_holder_sends_the_documented_query_and_decodes_the_name() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/account_holders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "account_holder",
            "payment_method_type": "mtn_momo",
            "name": "David Mbarga",
            "verified": true,
        })))
        .expect(1)
        .mount(&server)
        .await;

    let holder = client
        .account_holders()
        .retrieve(RetrieveAccountHolderParams::new(
            "237600000200",
            PaymentMethodType::MtnMomo,
        ))
        .await
        .unwrap();

    assert_eq!(holder.object, "account_holder");
    assert_eq!(holder.payment_method_type, "mtn_momo");
    assert_eq!(holder.name.as_deref(), Some("David Mbarga"));
    assert!(holder.verified);

    let request = only_request(&server, "/v1/account_holders").await;
    assert_eq!(request.method.as_str(), "GET");
    // Byte-for-byte, and in this order: the Node SDK pins the identical
    // string. A GET carries no body and no `Idempotency-Key` — that header
    // is a write-path property (`docs/flows/merchant-auth.md`, "Headers"),
    // and sending one here would be a second thing for the two SDKs to
    // disagree about.
    assert_eq!(
        request.url.query(),
        Some("msisdn=237600000200&payment_method_type=mtn_momo")
    );
    assert!(request.body.is_empty());
    assert_eq!(header_value(&request, "idempotency-key"), None);
}

/// **The answer a caller must not confuse with an error.** A rail that has no
/// record answers `200` with `name: null`, and it decodes as `None` — not as
/// a decode failure, and not as a missing key.
#[tokio::test]
async fn a_holder_the_rail_does_not_know_decodes_as_a_present_null_name() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/account_holders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "account_holder",
            "payment_method_type": "mtn_momo",
            "name": null,
            "verified": false,
        })))
        .expect(1)
        .mount(&server)
        .await;

    let holder = client
        .account_holders()
        .retrieve(RetrieveAccountHolderParams::new(
            "237600000404",
            PaymentMethodType::MtnMomo,
        ))
        .await
        .unwrap();

    assert_eq!(holder.name, None);
    assert!(!holder.verified);
}

/// A rail that could not be asked is an `Error`, never an `Ok` whose `name`
/// happens to be `None` — the distinction the whole resource exists for.
/// A caller matching a nominated refund destination refuses on both, but
/// only one of them is the payer's to fix.
#[tokio::test]
async fn a_rail_that_could_not_be_asked_is_an_error_and_not_a_null_name() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/account_holders"))
        .respond_with(ResponseTemplate::new(502).set_body_json(json!({
            "error": {
                "type": "api_error",
                "code": "provider_unavailable",
                "message": "The payment provider is unavailable. We are retrying.",
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let error = client
        .account_holders()
        .retrieve(RetrieveAccountHolderParams::new(
            "237600000200",
            PaymentMethodType::MtnMomo,
        ))
        .await
        .expect_err("a 502 must not decode into an account_holder");

    match error {
        Error::Api { status, code, .. } => {
            assert_eq!(status, 502);
            assert_eq!(code.as_deref(), Some("provider_unavailable"));
        }
        other => panic!("expected an API error, got {other:?}"),
    }
}

/// A rail with no account-holder API is a `400` naming the parameter, which
/// this SDK surfaces rather than pre-empting: whether a rail can answer is a
/// property of the *deployment*, and an SDK-side table of it would refuse a
/// rail a later deployment enables.
#[tokio::test]
async fn a_rail_with_no_account_holder_api_surfaces_the_servers_named_parameter() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/account_holders"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "type": "invalid_request_error",
                "code": "invalid_request",
                "param": "payment_method_type",
                "message": "This payment method cannot look up an account holder on this deployment.",
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let error = client
        .account_holders()
        .retrieve(RetrieveAccountHolderParams::new(
            "237600000200",
            PaymentMethodType::OrangeMoney,
        ))
        .await
        .expect_err("a 400 must not decode into an account_holder");

    match error {
        Error::Api { status, param, .. } => {
            assert_eq!(status, 400);
            assert_eq!(param.as_deref(), Some("payment_method_type"));
        }
        other => panic!("expected an API error, got {other:?}"),
    }

    // The SDK sent the request rather than refusing locally: the query is in
    // the journal, spelling the rail the caller named.
    let request = only_request(&server, "/v1/account_holders").await;
    assert_eq!(
        request.url.query(),
        Some("msisdn=237600000200&payment_method_type=orange_money")
    );
}

// --------------------------------------------------------------------------
// /v1/checkout/sessions — Step 9's four merchant operations.
//
// These mirror `sdks/nodejs/src/client.test.ts`'s `checkout.sessions` block
// one for one, down to the encoded body strings, because ADR-0015's parity
// rule is about *wire semantics*: two SDKs that both "support checkout
// sessions" but encode `ui_mode` differently are not at parity, and only a
// byte-level assertion on both sides catches that.
// --------------------------------------------------------------------------

#[tokio::test]
async fn create_checkout_session_sends_the_documented_body_and_decodes_the_object() {
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/checkout/sessions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::checkout_session_json(
                "cs_123",
                Some("cs_123_secret_abc123"),
            )),
        )
        .expect(1)
        .mount(&server)
        .await;

    let session = client
        .checkout()
        .sessions()
        .create(
            CreateCheckoutSessionParams {
                payment_intent: "pi_123".to_string(),
                ui_mode: Some(CheckoutUiMode::Hosted),
                success_url: Some("https://shop.example/ok?sid={CHECKOUT_SESSION_ID}".to_string()),
                cancel_url: Some("https://shop.example/cancel".to_string()),
                return_url: None,
            },
            RequestOptions::new().with_idempotency_key("order_1234_session_1"),
        )
        .await
        .unwrap();

    assert_eq!(session.id, "cs_123");
    assert_eq!(session.object, "checkout.session");
    assert_eq!(session.ui_mode, CheckoutUiMode::Hosted);
    assert_eq!(session.status, CheckoutSessionStatus::Open);
    assert_eq!(session.payment_status, CheckoutPaymentStatus::Unpaid);
    assert_eq!(session.payment_intent, "pi_123");
    assert_eq!(
        session.client_secret.as_deref(),
        Some("cs_123_secret_abc123")
    );

    let request = only_request(&server, "/v1/checkout/sessions").await;
    assert_eq!(request.method.as_str(), "POST");
    assert_eq!(
        header_value(&request, "content-type").as_deref(),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        header_value(&request, "idempotency-key").as_deref(),
        Some("order_1234_session_1")
    );
    assert_eq!(
        body_string(&request),
        "payment_intent=pi_123&ui_mode=hosted&success_url=https%3A%2F%2Fshop.example%2Fok%3Fsid%3D%7BCHECKOUT_SESSION_ID%7D&cancel_url=https%3A%2F%2Fshop.example%2Fcancel"
    );
}

#[tokio::test]
async fn create_checkout_session_body_matches_the_node_sdk_byte_for_byte() {
    // The exact string `sdks/nodejs/src/client.test.ts` asserts in
    // "checkout.sessions.create: exact path, method, Idempotency-Key, and
    // body". Written as a literal on both sides on purpose: a re-parsing
    // comparison would pass while the two SDKs sent different bodies.
    let params = CreateCheckoutSessionParams {
        payment_intent: "pi_123".to_string(),
        ui_mode: Some(CheckoutUiMode::Embedded),
        success_url: None,
        cancel_url: None,
        return_url: Some("https://shop.example/order/42".to_string()),
    };
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/checkout/sessions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::checkout_session_json(
                "cs_123",
                Some("cs_123_secret_abc123"),
            )),
        )
        .mount(&server)
        .await;
    client
        .checkout()
        .sessions()
        .create(params, RequestOptions::new())
        .await
        .unwrap();

    let request = only_request(&server, "/v1/checkout/sessions").await;
    assert_eq!(
        body_string(&request),
        "payment_intent=pi_123&ui_mode=embedded&return_url=https%3A%2F%2Fshop.example%2Forder%2F42"
    );
}

#[tokio::test]
async fn create_checkout_session_omits_absent_optional_fields_rather_than_sending_them_empty() {
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/checkout/sessions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(support::checkout_session_json("cs_123", None)),
        )
        .mount(&server)
        .await;

    client
        .checkout()
        .sessions()
        .create(
            CreateCheckoutSessionParams {
                payment_intent: "pi_123".to_string(),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .unwrap();

    let request = only_request(&server, "/v1/checkout/sessions").await;
    // `ui_mode=` and no `ui_mode` are different requests; only the second
    // means "let the server default it to hosted".
    assert_eq!(body_string(&request), "payment_intent=pi_123");
    // Generated when the caller supplied none, exactly as every other POST.
    let key = header_value(&request, "idempotency-key").unwrap();
    assert_eq!(key.len(), 36, "a UUIDv4: {key}");
}

#[tokio::test]
async fn retrieve_checkout_session_is_a_get_with_no_body_and_surfaces_client_secret() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/checkout/sessions/cs_123"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::checkout_session_json(
                "cs_123",
                Some("cs_123_secret_abc123"),
            )),
        )
        .expect(1)
        .mount(&server)
        .await;

    let session = client
        .checkout()
        .sessions()
        .retrieve("cs_123")
        .await
        .unwrap();

    assert_eq!(
        session.client_secret.as_deref(),
        Some("cs_123_secret_abc123")
    );
    assert_eq!(
        session.success_url.as_deref(),
        Some("https://shop.example/ok?sid={CHECKOUT_SESSION_ID}")
    );

    let request = only_request(&server, "/v1/checkout/sessions/cs_123").await;
    assert_eq!(request.method.as_str(), "GET");
    assert!(request.body.is_empty());
    assert!(request.url.query().is_none());
}

#[tokio::test]
async fn list_checkout_sessions_encodes_its_pagination_and_intent_filter() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/checkout/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [support::checkout_session_json("cs_123", None)],
            "has_more": false,
            "url": "/v1/checkout/sessions",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let list = client
        .checkout()
        .sessions()
        .list(ListCheckoutSessionsParams {
            limit: Some(10),
            payment_intent: Some("pi_123".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(list.data.len(), 1);
    // A list item never carries the payer credential — the same rule the
    // intent list obeys, for the same reason.
    assert_eq!(list.data[0].client_secret, None);

    let request = only_request(&server, "/v1/checkout/sessions").await;
    assert_eq!(request.url.query(), Some("limit=10&payment_intent=pi_123"));
}

#[tokio::test]
async fn expire_checkout_session_posts_an_empty_body_and_still_carries_an_idempotency_key() {
    let (server, client) = fixture().await;
    let mut expired = support::checkout_session_json("cs_123", None);
    expired["status"] = json!("expired");
    Mock::given(method("POST"))
        .and(path("/v1/checkout/sessions/cs_123/expire"))
        .respond_with(ResponseTemplate::new(200).set_body_json(expired))
        .expect(1)
        .mount(&server)
        .await;

    let session = client
        .checkout()
        .sessions()
        .expire("cs_123", RequestOptions::new())
        .await
        .unwrap();

    assert_eq!(session.status, CheckoutSessionStatus::Expired);

    let request = only_request(&server, "/v1/checkout/sessions/cs_123/expire").await;
    assert_eq!(request.method.as_str(), "POST");
    assert!(request.body.is_empty());
    assert!(header_value(&request, "idempotency-key").is_some());
}

#[tokio::test]
async fn a_checkout_session_id_with_url_metacharacters_is_percent_encoded_into_the_path() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::checkout_session_json("cs_1", None)),
        )
        .mount(&server)
        .await;

    let _ = client.checkout().sessions().retrieve("../../admin").await;

    let requests = server.received_requests().await.unwrap();
    let paths: Vec<String> = requests
        .iter()
        .map(|r| r.url.path().to_string())
        .filter(|p| p != "/v1/oauth/token")
        .collect();
    assert_eq!(paths, vec!["/v1/checkout/sessions/..%2F..%2Fadmin"]);
}

#[tokio::test]
async fn a_404_for_an_unknown_checkout_session_maps_to_an_api_error() {
    let (server, client) = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/checkout/sessions/cs_nope"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "error": {
                "type": "invalid_request_error",
                "code": "resource_missing",
                "message": "No such checkout session: cs_nope",
            }
        })))
        .mount(&server)
        .await;

    let err = client
        .checkout()
        .sessions()
        .retrieve("cs_nope")
        .await
        .expect_err("an unknown session is refused");

    match err {
        Error::Api {
            status,
            code,
            message,
            ..
        } => {
            assert_eq!(status, 404);
            assert_eq!(code.as_deref(), Some("resource_missing"));
            assert_eq!(message, "No such checkout session: cs_nope");
        }
        other => panic!("expected Error::Api, got {other:?}"),
    }
}

#[tokio::test]
async fn a_409_on_expiring_a_session_with_a_live_charge_maps_to_an_api_error() {
    let (server, client) = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/checkout/sessions/cs_123/expire"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "error": {
                "type": "invalid_request_error",
                "code": "invalid_state",
                "message": "This checkout session has a charge in flight.",
            }
        })))
        .mount(&server)
        .await;

    let err = client
        .checkout()
        .sessions()
        .expire("cs_123", RequestOptions::new())
        .await
        .expect_err("a session with a live charge cannot be expired");

    match err {
        Error::Api { status, code, .. } => {
            assert_eq!(status, 409);
            assert_eq!(code.as_deref(), Some("invalid_state"));
        }
        other => panic!("expected Error::Api, got {other:?}"),
    }
}
