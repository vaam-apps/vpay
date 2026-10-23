//! Unit tests: the adapter in process, with no container.
//!
//! Two kinds. Most point the adapter at a closed loopback port and assert
//! something is refused before any request leaves. A few need to see what the
//! adapter *sent*, and for those a loopback listener reads one raw HTTP
//! request and answers it with a fixed status line — a socket, not a double:
//! the adapter's own client speaks real HTTP to it. Everything about Notch
//! Pay's answers is the conformance suite's, against a WireMock stub.

use std::collections::BTreeMap;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use uuid::Uuid;
use vpay_core::{Currency, Money, ProviderFlow};
use vpay_provider::{ChargeRef, ProviderAdapter, ProviderConfig, ProviderError, RefundDestination};

use super::*;

/// Fixture keys, named so a secret scanner cannot mistake them for a real
/// provider's (`pk_test_…` / `sk_test_…` shapes are what Trivy flags). Both
/// carry `test_`, one of the two markers the livemode rule reads (the other,
/// `test.`, is what real sandbox keys carry).
const PUBLIC: &str = "public-key-test_0001";
const PRIVATE: &str = "private-key-test_0002";

fn adapter() -> Adapter {
    Adapter::new(vpay_provider::http::client().expect("the vendored-roots client builds"))
}

fn config(base_url: &str) -> ProviderConfig {
    ProviderConfig {
        base_url: base_url.to_owned(),
        callback_url: "http://127.0.0.1:1/provider/notchpay/callback".to_owned(),
        currency: Currency::Xaf,
        settings: BTreeMap::from([
            ("channel".to_owned(), "cm.mobile".to_owned()),
            ("description".to_owned(), "Payment".to_owned()),
        ]),
        credentials: BTreeMap::from([
            ("public_key".to_owned(), PUBLIC.to_owned()),
            ("private_key".to_owned(), PRIVATE.to_owned()),
        ]),
        connect_timeout: Duration::from_millis(200),
        request_timeout: Duration::from_millis(500),
    }
}

/// A configuration whose every request would fail to connect, for tests
/// that assert nothing left the process.
fn closed() -> ProviderConfig {
    config("http://127.0.0.1:1")
}

fn charge() -> ChargeRef {
    ChargeRef {
        reference_id: Uuid::from_u128(0x0202),
        amount: Money::new(5_000, Currency::Xaf).expect("non-negative"),
        payer_ref: Some("237670000000".to_owned()),
        ref_extra: BTreeMap::new(),
        // Filled to prove it goes nowhere: a push rail has no browser.
        return_url: Some("https://shop.example/order/1/return".to_owned()),
    }
}

/// Notch Pay's reference for [`charge`], shaped as the sandbox's
/// (`trx.test_…`).
const TRX: &str = "trx.test_0000000000000202";

/// [`charge`] after a `submit`: it holds Notch Pay's reference, the only one
/// Notch Pay's paths answer for.
fn submitted_charge() -> ChargeRef {
    let mut charge = charge();
    charge
        .ref_extra
        .insert(NOTCHPAY_REFERENCE.to_owned(), TRX.to_owned());
    charge
}

/// Reads one HTTP request from `socket` in full and returns its request line.
async fn read_request_line(socket: &mut tokio::net::TcpStream) -> String {
    let mut seen = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = socket.read(&mut buffer).await.expect("reads");
        seen.extend_from_slice(buffer.get(..read).expect("within the buffer"));
        let text = String::from_utf8_lossy(&seen).to_string();
        if let Some((head, rest)) = text.split_once("\r\n\r\n") {
            let length = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if rest.len() >= length {
                return head.lines().next().unwrap_or_default().to_owned();
            }
        }
    }
}

/// Listens on loopback, reads one HTTP request in full, answers it with
/// `status_line` and an empty JSON body, and hands back the request's text.
async fn capture_one(status_line: &'static str) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let origin = format!("http://{}", listener.local_addr().expect("an address"));
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("one connection");
        let mut seen = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = socket.read(&mut buffer).await.expect("reads");
            seen.extend_from_slice(buffer.get(..read).expect("a read never exceeds the buffer"));
            let text = String::from_utf8_lossy(&seen);
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if body.len() >= length {
                    break;
                }
            }
            if read == 0 {
                break;
            }
        }
        let reply = format!(
            "{status_line}\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}"
        );
        socket.write_all(reply.as_bytes()).await.expect("answers");
        String::from_utf8_lossy(&seen).into_owned()
    });
    (origin, handle)
}

#[test]
fn capabilities_are_coherent_and_name_the_rails_truth() {
    let capabilities = adapter().capabilities();
    assert!(capabilities.is_coherent());
    assert_eq!(capabilities.flow, ProviderFlow::Push);
    assert!(capabilities.supports_refunds && capabilities.supports_partial_refunds);
    assert_eq!(capabilities.refund_destination, RefundDestination::Origin);
    assert!(!capabilities.supports_account_holder_lookup);
}

#[test]
fn code_matches_the_payment_method_type() {
    assert_eq!(adapter().code(), "notchpay");
}

/// ADR-0031 condition 4: the exposure is declared through the port, names the
/// credential by its configuration key, lists the four operations RFC-0012
/// records, and points at the page that explains them.
#[test]
fn the_exposure_is_declared_through_the_port() {
    let exposure = adapter()
        .accepted_exposure()
        .expect("a rail on ADR-0031's second limb declares its exposure");
    assert_eq!(exposure.credential, "public_key");
    assert_eq!(exposure.operations.len(), 4);
    for word in ["read", "initialize", "process", "cancel"] {
        assert!(
            exposure.operations.iter().any(|op| op.contains(word)),
            "the exposure must name what the key can {word}: {:?}",
            exposure.operations
        );
    }
    assert_eq!(exposure.document, "docs/flows/adapter-notchpay.md");
    let page = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(exposure.document);
    assert!(page.is_file(), "{} must exist", page.display());
}

/// The payer field is `mtn_momo`'s, so a checkout renders both rails the same.
#[test]
fn the_payer_field_is_the_mobile_number() {
    let fields = adapter().payer_fields();
    assert_eq!(fields.len(), 1);
    let field = fields.first().expect("one field");
    assert_eq!(field.name, "msisdn");
    assert!(field.required);
}

#[tokio::test]
async fn keys_never_reach_a_debug_or_an_error() {
    let adapter = adapter();
    let config = closed();
    let rendered = [
        format!("{adapter:?}"),
        format!("{config:?}"),
        format!("{:?}", adapter.submit(&charge(), &config).await),
        format!(
            "{:?}",
            adapter.query_status(&submitted_charge(), &config).await
        ),
    ];
    for text in rendered {
        for key in [PUBLIC, PRIVATE] {
            assert!(!text.contains(key), "a key reached {text}");
        }
    }
}

/// ADR-0031 condition 2's belt in the adapter: either key missing is a
/// `Config` naming that key, before any request.
#[tokio::test]
async fn a_missing_key_is_refused_by_name_before_any_request() {
    for missing in ["public_key", "private_key"] {
        let mut config = closed();
        config.credentials.remove(missing);
        for outcome in [
            adapter().submit(&charge(), &config).await.map(|_| ()),
            adapter()
                .query_status(&submitted_charge(), &config)
                .await
                .map(|_| ()),
        ] {
            match outcome {
                Err(ProviderError::Config(message)) => assert!(message.contains(missing)),
                other => panic!("{missing}: expected Config, got {other:?}"),
            }
        }
    }
}

#[tokio::test]
async fn a_channel_the_adapter_does_not_know_is_refused_before_any_request() {
    for channel in [None, Some("cm.mtn"), Some("ci.orange"), Some("")] {
        let mut config = closed();
        match channel {
            Some(value) => config
                .settings
                .insert("channel".to_owned(), value.to_owned()),
            None => config.settings.remove("channel"),
        };
        let outcome = adapter().submit(&charge(), &config).await;
        assert!(
            matches!(outcome, Err(ProviderError::Config(_))),
            "{channel:?}: {outcome:?}"
        );
    }
}

#[tokio::test]
async fn a_charge_in_another_currency_is_refused_before_any_request() {
    let mut charge = charge();
    charge.amount = Money::new(5_000, Currency::Eur).expect("non-negative");
    let outcome = adapter().submit(&charge, &closed()).await;
    assert!(
        matches!(outcome, Err(ProviderError::Config(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_push_charge_without_a_payer_never_reaches_the_rail() {
    let mut charge = charge();
    charge.payer_ref = None;
    let outcome = adapter().submit(&charge, &closed()).await;
    assert!(
        matches!(outcome, Err(ProviderError::Config(_))),
        "{outcome:?}"
    );
}

/// A closed port is a transport failure: the charge's fate is unknown, never
/// a decline.
#[tokio::test]
async fn an_unreachable_rail_is_transport_never_a_decline() {
    for outcome in [
        adapter().submit(&charge(), &closed()).await.map(|_| ()),
        adapter()
            .query_status(&submitted_charge(), &closed())
            .await
            .map(|_| ()),
    ] {
        assert!(
            matches!(outcome, Err(ProviderError::Transport { .. })),
            "{outcome:?}"
        );
    }
}

/// ADR-0031 condition 1, and RFC-0012 § 3's body: the private key rides on
/// the call even though Notch Pay's specification does not ask for it there,
/// the reference is ours, the number is `+`-prefixed E.164, and nothing else
/// of the payer's goes — no email, no name, no return URL, no callback.
#[tokio::test]
async fn the_initialize_body_carries_the_phone_and_nothing_else_of_the_payers() {
    // A 503, so the adapter stops after call 1 and the one request captured
    // is the initialize.
    let (origin, request) = capture_one("HTTP/1.1 503 Service Unavailable").await;
    let outcome = adapter().submit(&charge(), &config(&origin)).await;
    assert!(
        matches!(outcome, Err(ProviderError::Transport { .. })),
        "{outcome:?}"
    );
    let request = request.await.expect("the listener finishes");
    let (head, body) = request.split_once("\r\n\r\n").expect("a whole request");
    let head = head.to_ascii_lowercase();

    assert!(head.starts_with("post /payments "), "{head}");
    assert!(head.contains(&format!("authorization: {}", PUBLIC.to_ascii_lowercase())));
    assert!(head.contains(&format!("x-grant: {}", PRIVATE.to_ascii_lowercase())));

    let body: serde_json::Value = serde_json::from_str(body).expect("a JSON body");
    let mut keys: Vec<&str> = body
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["amount", "currency", "description", "phone", "reference"]
    );
    let field = |name: &str| body.get(name).cloned().unwrap_or_default();
    assert_eq!(field("amount"), 5_000);
    assert_eq!(field("currency"), "XAF");
    assert_eq!(field("phone"), "+237670000000");
    assert_eq!(field("reference"), Uuid::from_u128(0x0202).to_string());
    assert!(
        !request.contains("shop.example"),
        "a push rail has no browser"
    );
    assert!(!request.contains("/provider/notchpay/callback"));
}

/// The read is addressed by Notch Pay's reference from `ref_extra`, never by
/// ours (RFC-0012 § 10 item 2: the sandbox answered `404` to ours), and
/// carries both keys.
#[tokio::test]
async fn the_read_is_addressed_by_notch_pays_reference_and_carries_both_keys() {
    let (origin, request) = capture_one("HTTP/1.1 503 Service Unavailable").await;
    let _ = adapter()
        .query_status(&submitted_charge(), &config(&origin))
        .await;
    let request = request
        .await
        .expect("the listener finishes")
        .to_ascii_lowercase();
    assert!(
        request.starts_with(&format!("get /payments/{} ", TRX.to_ascii_lowercase())),
        "{request}"
    );
    assert!(request.contains(&format!("x-grant: {}", PRIVATE.to_ascii_lowercase())));
}

/// A charge with no Notch Pay reference is key material vpay lost, not a
/// payment Notch Pay never heard of: `NeedsReconciliation`, before any
/// request, and never `NotFound`. It was `Config` until 2026-10-23, which
/// dead-lettered the poll job (vaam-apps/vpay#313). `closed()` is a base URL
/// nothing listens on, so a request would have been `Transport`.
#[tokio::test]
async fn a_read_without_notch_pays_reference_needs_reconciliation_before_any_request() {
    let outcome = adapter().query_status(&charge(), &closed()).await;
    assert!(
        matches!(&outcome, Err(ProviderError::NeedsReconciliation { context }) if context.contains(NOTCHPAY_REFERENCE)),
        "{outcome:?}"
    );
}

/// Serves `answers` in order, one per connection, and returns each request
/// line it read.
async fn serve(
    answers: Vec<(&'static str, String)>,
) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let origin = format!("http://{}", listener.local_addr().expect("an address"));
    let handle = tokio::spawn(async move {
        let mut lines = Vec::new();
        for (status_line, body) in answers {
            let (mut socket, _) = listener.accept().await.expect("a connection");
            lines.push(read_request_line(&mut socket).await);
            let answer = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(answer.as_bytes()).await.expect("writes");
        }
        lines
    });
    (origin, handle)
}

fn payment_body(code: u16, trxref: &str, status: &str) -> String {
    format!(
        r#"{{"code":{code},"transaction":{{"reference":"{TRX}","trxref":"{trxref}","amount":5000,"amounts":{{"total":5000}},"currency":"XAF","status":"{status}"}}}}"#
    )
}

/// The process call goes to Notch Pay's reference from the initialize
/// answer, and `Submitted` carries it.
#[tokio::test]
async fn the_process_call_is_addressed_by_the_reference_initialize_returned() {
    let ours = Uuid::from_u128(0x0202).to_string();
    let (origin, server) = serve(vec![
        ("HTTP/1.1 201 Created", payment_body(201, &ours, "pending")),
        (
            "HTTP/1.1 202 Accepted",
            payment_body(202, &ours, "processing"),
        ),
    ])
    .await;
    let submitted = adapter()
        .submit(&charge(), &config(&origin))
        .await
        .expect("an accepted submit");
    let lines = server.await.expect("the server finishes");
    assert_eq!(
        lines,
        vec![
            "POST /payments HTTP/1.1".to_owned(),
            format!("POST /payments/{TRX} HTTP/1.1"),
        ]
    );
    assert_eq!(
        submitted
            .ref_extra
            .get(NOTCHPAY_REFERENCE)
            .map(String::as_str),
        Some(TRX)
    );
}

/// An initialize answer about another merchant reference is refused before
/// the prompt: nothing is sent to the payer.
#[tokio::test]
async fn an_initialize_answer_for_another_reference_never_prompts() {
    let (origin, server) = serve(vec![(
        "HTTP/1.1 201 Created",
        payment_body(201, "00000000-0000-0000-0000-00000000beef", "pending"),
    )])
    .await;
    let outcome = adapter().submit(&charge(), &config(&origin)).await;
    assert!(
        matches!(outcome, Err(ProviderError::Malformed { .. })),
        "{outcome:?}"
    );
    assert_eq!(
        server.await.expect("finishes").len(),
        1,
        "only initialize was sent"
    );
}

/// A `409` for a charge that holds no Notch Pay reference cannot be
/// resolved: Notch Pay finds payments by its own reference only. It is
/// `NeedsReconciliation` (fate unknown, and no second submit can learn it),
/// never `Submitted` and never a decline, and nothing more is sent. It was
/// `Malformed` until 2026-10-23 (vaam-apps/vpay#313).
#[tokio::test]
async fn a_duplicate_with_no_held_reference_needs_reconciliation_and_sends_nothing_more() {
    let (origin, server) = serve(vec![(
        "HTTP/1.1 409 Conflict",
        r#"{"code":"409","status":"Conflict","message":"Reference already existing","errors":{"reference":["Reference already existing"]}}"#.to_owned(),
    )])
    .await;
    let outcome = adapter().submit(&charge(), &config(&origin)).await;
    assert!(
        matches!(&outcome, Err(ProviderError::NeedsReconciliation { .. })),
        "{outcome:?}"
    );
    assert_eq!(
        server.await.expect("finishes").len(),
        1,
        "only initialize was sent"
    );
}

/// A `409` for a charge that holds Notch Pay's reference reads it by that
/// reference, and a payment past `pending` is `Submitted` with no second
/// prompt.
#[tokio::test]
async fn a_duplicate_with_a_held_reference_reads_it_and_does_not_prompt_again() {
    let ours = Uuid::from_u128(0x0202).to_string();
    let (origin, server) = serve(vec![
        (
            "HTTP/1.1 409 Conflict",
            r#"{"code":"409","status":"Conflict","message":"Reference already existing"}"#
                .to_owned(),
        ),
        ("HTTP/1.1 200 OK", payment_body(200, &ours, "processing")),
    ])
    .await;
    let submitted = adapter()
        .submit(&submitted_charge(), &config(&origin))
        .await
        .expect("a resolvable duplicate is Submitted");
    let lines = server.await.expect("finishes");
    assert_eq!(
        lines,
        vec![
            "POST /payments HTTP/1.1".to_owned(),
            format!("GET /payments/{TRX} HTTP/1.1"),
        ]
    );
    assert_eq!(
        submitted
            .ref_extra
            .get(NOTCHPAY_REFERENCE)
            .map(String::as_str),
        Some(TRX)
    );
}

/// A read addressed by Notch Pay's reference that names another merchant
/// reference must not settle this charge.
#[tokio::test]
async fn a_read_naming_another_trxref_is_never_a_success() {
    let (origin, _server) = serve(vec![(
        "HTTP/1.1 200 OK",
        payment_body(200, "00000000-0000-0000-0000-00000000beef", "complete"),
    )])
    .await;
    let outcome = adapter()
        .query_status(&submitted_charge(), &config(&origin))
        .await;
    assert!(
        matches!(outcome, Err(ProviderError::Malformed { .. })),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_configured_deadline_fires_against_a_silent_rail() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let origin = format!("http://{}", listener.local_addr().expect("an address"));
    // Accept and hold the connection without ever answering.
    let held = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("one connection");
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(socket);
    });
    let mut config = config(&origin);
    config.request_timeout = Duration::from_millis(100);
    let started = std::time::Instant::now();
    let outcome = adapter().query_status(&submitted_charge(), &config).await;
    assert!(
        matches!(outcome, Err(ProviderError::Transport { .. })),
        "{outcome:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    held.abort();
}

#[test]
fn a_callback_is_identifiers_only_and_carries_no_key_material() {
    let reference = Uuid::from_u128(0x0202);
    for event in [
        "payment.created",
        "payment.complete",
        "payment.failed",
        "payment.canceled",
        "payment.expired",
    ] {
        let body = serde_json::json!({
            "id": "evt.test_0001",
            "type": event,
            "created_at": "2026-10-20T10:00:00Z",
            "data": {
                "id": "pay.test_0001",
                "reference": reference.to_string(),
                "status": "complete",
                "amount": 5_000,
                "currency": "XAF"
            }
        });
        let parsed = adapter()
            .parse_callback(body.to_string().as_bytes())
            .unwrap_or_else(|error| panic!("{event}: {error}"));
        assert_eq!(parsed.reference_id, reference);
        assert!(
            parsed.ref_extra.is_empty(),
            "nothing from an unauthenticated request may reach a charge: {:?}",
            parsed.ref_extra
        );
    }
}

#[test]
fn a_callback_for_an_event_vpay_does_not_handle_is_refused() {
    let reference = Uuid::from_u128(0x0202).to_string();
    let bodies = [
        serde_json::json!({ "type": "payment.processing", "data": { "reference": reference } }),
        serde_json::json!({ "type": "payment.refunded", "data": { "reference": reference } }),
        serde_json::json!({ "type": "transfer.complete", "data": { "reference": reference } }),
        serde_json::json!({ "data": { "reference": reference } }),
        serde_json::json!({ "type": "payment.complete", "data": { "reference": "trx.test_0001" } }),
        serde_json::json!({ "type": "payment.complete", "data": {} }),
        serde_json::json!({}),
    ];
    for body in bodies {
        let outcome = adapter().parse_callback(body.to_string().as_bytes());
        assert!(
            matches!(outcome, Err(ProviderError::Malformed { .. })),
            "{body}: {outcome:?}"
        );
    }
    assert!(matches!(
        adapter().parse_callback(b"not json"),
        Err(ProviderError::Malformed { .. })
    ));
}

/// A declared gap, naming this rail: `verify-status` matches the token
/// string, and the conformance suite checks it names `notchpay`.
#[tokio::test]
async fn refund_is_a_token_about_vpay_not_an_answer_about_notch_pay() {
    let outcome = adapter()
        .refund(&charge(), charge().amount, None, &closed())
        .await;
    assert!(
        matches!(
            outcome,
            Err(ProviderError::NotImplemented("notchpay::refund"))
        ),
        "{outcome:?}"
    );
    assert!(adapter().capabilities().supports_refunds);
}

/// An `Origin` rail takes the port's default, which is the permanent answer.
#[test]
fn an_origin_rail_parses_no_destination() {
    let mut raw = serde_json::Map::new();
    raw.insert("msisdn".to_owned(), serde_json::json!("+237600000200"));
    assert!(matches!(
        adapter().parse_destination(&raw),
        Err(ProviderError::Unsupported)
    ));
}

/// The payment object's shape Notch Pay's sandbox answered with on
/// 2026-10-21: `reference` is Notch Pay's `trx.…`, `trxref` is ours. A
/// callback in that shape names our charge. (No webhook was received, so the
/// event's own `data` is assumed, not measured, to share the shape.)
#[test]
fn a_callback_in_the_sandboxs_payment_shape_names_our_charge_by_trxref() {
    let reference = Uuid::from_u128(0x0202);
    let body = serde_json::json!({
        "id": "evt.test_0001",
        "type": "payment.complete",
        "data": { "reference": TRX, "trxref": reference.to_string(), "status": "complete" }
    });
    let parsed = adapter()
        .parse_callback(body.to_string().as_bytes())
        .expect("trxref names our charge");
    assert_eq!(parsed.reference_id, reference);
    assert!(parsed.ref_extra.is_empty());
}
