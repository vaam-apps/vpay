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
use vpay_core::{Currency, FailureCode, Money, ProviderFlow};
use vpay_provider::{
    ChargeRef, PriorSubmits, ProviderAdapter, ProviderConfig, ProviderError, RefundDestination,
    SubmitSteps,
};

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
        method_type: None,
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
    // Notch Pay issues its own reference at initialize and answers for no
    // other, so the core commits it before the prompt (ADR-0032).
    assert_eq!(capabilities.submit_steps, SubmitSteps::PrepareThenSubmit);
    // The aggregator: both wallet networks (ADR-0029 D6, D12).
    assert!(capabilities.method_types_are_coherent());
    assert_eq!(
        capabilities.method_types.iter().collect::<Vec<_>>(),
        vec![MethodType::MtnWallet, MethodType::OrangeWallet]
    );
}

/// RFC-0012 delivery step 8: a charge routed by the payer's method type is
/// processed on that network's channel, and a charge confirmed by the rail's
/// own code keeps the configured one. The setting is checked either way.
#[test]
fn the_channel_follows_the_payers_method_type_and_else_the_setting() {
    let config = closed();
    let mut charge = charge();
    assert_eq!(
        channel_for(&charge, &config).expect("configured"),
        "cm.mobile"
    );
    charge.method_type = Some(MethodType::MtnWallet);
    assert_eq!(channel_for(&charge, &config).expect("configured"), "cm.mtn");
    charge.method_type = Some(MethodType::OrangeWallet);
    assert_eq!(
        channel_for(&charge, &config).expect("configured"),
        "cm.orange"
    );

    let mut unknown = closed();
    unknown
        .settings
        .insert("channel".to_owned(), "ci.orange".to_owned());
    assert!(matches!(
        channel_for(&charge, &unknown),
        Err(ProviderError::Config(_))
    ));
}

#[test]
fn code_matches_the_payment_method_type() {
    assert_eq!(adapter().code(), "notchpay");
}

/// RFC-0013 § 2 as amended 2026-10-23: with `callbacks` unset, vpay leaves a
/// Notch Pay account alone. An endpoint created through the API delivered
/// nothing in the sandbox (and one registered in the dashboard does, as
/// measured on 2026-10-24), so the default is the operator's dashboard, not a
/// write vpay makes unasked.
#[test]
fn the_callback_registration_default_is_off() {
    assert_eq!(
        adapter().default_callback_registration(),
        CallbackRegistrationDefault::Off
    );
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
        format!(
            "{:?}",
            adapter
                .prepare(&charge(), &config, PriorSubmits::None)
                .await
        ),
        format!("{:?}", adapter.submit(&submitted_charge(), &config).await),
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
            adapter()
                .prepare(&charge(), &config, PriorSubmits::None)
                .await
                .map(|_| ()),
            adapter()
                .submit(&submitted_charge(), &config)
                .await
                .map(|_| ()),
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
        for outcome in [
            adapter()
                .prepare(&charge(), &config, PriorSubmits::None)
                .await
                .map(|_| ()),
            adapter()
                .submit(&submitted_charge(), &config)
                .await
                .map(|_| ()),
        ] {
            assert!(
                matches!(outcome, Err(ProviderError::Config(_))),
                "{channel:?}: {outcome:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_charge_in_another_currency_is_refused_before_any_request() {
    let euros = Money::new(5_000, Currency::Eur).expect("non-negative");
    let mut fresh = charge();
    fresh.amount = euros;
    let mut prepared = submitted_charge();
    prepared.amount = euros;
    for outcome in [
        adapter()
            .prepare(&fresh, &closed(), PriorSubmits::None)
            .await
            .map(|_| ()),
        adapter().submit(&prepared, &closed()).await.map(|_| ()),
    ] {
        assert!(
            matches!(outcome, Err(ProviderError::Config(_))),
            "{outcome:?}"
        );
    }
}

#[tokio::test]
async fn a_push_charge_without_a_payer_never_reaches_the_rail() {
    let mut charge = charge();
    charge.payer_ref = None;
    let mut prepared = submitted_charge();
    prepared.payer_ref = None;
    for outcome in [
        adapter()
            .prepare(&charge, &closed(), PriorSubmits::None)
            .await
            .map(|_| ()),
        adapter().submit(&prepared, &closed()).await.map(|_| ()),
    ] {
        assert!(
            matches!(outcome, Err(ProviderError::Config(_))),
            "{outcome:?}"
        );
    }
}

/// A closed port is a transport failure: the charge's fate is unknown, never
/// a decline.
#[tokio::test]
async fn an_unreachable_rail_is_transport_never_a_decline() {
    for outcome in [
        adapter()
            .prepare(&charge(), &closed(), PriorSubmits::None)
            .await
            .map(|_| ()),
        adapter()
            .submit(&submitted_charge(), &closed())
            .await
            .map(|_| ()),
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
    // A 503, and `prepare` is call 1 alone, so the one request captured is
    // the initialize.
    let (origin, request) = capture_one("HTTP/1.1 503 Service Unavailable").await;
    let outcome = adapter()
        .prepare(&charge(), &config(&origin), PriorSubmits::None)
        .await;
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

/// `prepare` is initialize and nothing else (ADR-0032): one request, and
/// what comes back is the reference Notch Pay issued, for the core to commit
/// before anyone is prompted.
#[tokio::test]
async fn prepare_returns_notch_pays_reference_and_prompts_nobody() {
    let ours = Uuid::from_u128(0x0202).to_string();
    let (origin, server) = serve(vec![(
        "HTTP/1.1 201 Created",
        payment_body(201, &ours, "pending"),
    )])
    .await;
    let prepared = adapter()
        .prepare(&charge(), &config(&origin), PriorSubmits::None)
        .await
        .expect("an accepted initialize");
    assert_eq!(
        server.await.expect("the server finishes"),
        vec!["POST /payments HTTP/1.1".to_owned()],
        "initialize only: the prompt is submit's, after the commit"
    );
    assert_eq!(
        prepared,
        BTreeMap::from([(NOTCHPAY_REFERENCE.to_owned(), TRX.to_owned())])
    );
}

/// `submit` is process alone, addressed by the reference `prepare` returned
/// and the core committed, and `Submitted` carries it back.
#[tokio::test]
async fn the_process_call_is_addressed_by_the_held_reference() {
    let ours = Uuid::from_u128(0x0202).to_string();
    let (origin, server) = serve(vec![(
        "HTTP/1.1 202 Accepted",
        payment_body(202, &ours, "processing"),
    )])
    .await;
    let submitted = adapter()
        .submit(&submitted_charge(), &config(&origin))
        .await
        .expect("an accepted submit");
    assert_eq!(
        server.await.expect("the server finishes"),
        vec![format!("POST /payments/{TRX} HTTP/1.1")],
        "process only: the payment exists already"
    );
    assert_eq!(
        submitted
            .ref_extra
            .get(NOTCHPAY_REFERENCE)
            .map(String::as_str),
        Some(TRX)
    );
}

/// The window ADR-0032 closed, guarded at the adapter: `submit` never
/// initializes. A charge that reaches it without Notch Pay's reference has
/// lost the only address Notch Pay answers for, so the answer is a person,
/// before any request — not an initialize, whose `409` could no longer say
/// whether anyone was prompted. `closed()` would have made a request
/// `Transport`.
#[tokio::test]
async fn submit_without_a_held_reference_never_initializes() {
    let outcome = adapter().submit(&charge(), &closed()).await;
    assert!(
        matches!(&outcome, Err(ProviderError::NeedsReconciliation { context }) if context.contains(NOTCHPAY_REFERENCE)),
        "{outcome:?}"
    );
}

/// A charge that already holds Notch Pay's reference is prepared: `prepare`
/// answers it from `ref_extra` and sends nothing. `closed()` would have made
/// a request `Transport`.
#[tokio::test]
async fn prepare_of_a_charge_that_holds_the_reference_sends_nothing() {
    for prior in [PriorSubmits::None, PriorSubmits::Some] {
        let prepared = adapter()
            .prepare(&submitted_charge(), &closed(), prior)
            .await
            .expect("answered from the charge");
        assert_eq!(
            prepared.get(NOTCHPAY_REFERENCE).map(String::as_str),
            Some(TRX)
        );
    }
}

/// An initialize answer about another merchant reference is refused, and
/// nothing is returned for the core to commit or to process.
#[tokio::test]
async fn an_initialize_answer_for_another_reference_never_prompts() {
    let (origin, server) = serve(vec![(
        "HTTP/1.1 201 Created",
        payment_body(201, "00000000-0000-0000-0000-00000000beef", "pending"),
    )])
    .await;
    let outcome = adapter()
        .prepare(&charge(), &config(&origin), PriorSubmits::None)
        .await;
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

/// Notch Pay's `409` "Reference already existing", as the sandbox answered it
/// on 2026-10-21.
fn duplicate_reference() -> (&'static str, String) {
    (
        "HTTP/1.1 409 Conflict",
        r#"{"code":"409","status":"Conflict","message":"Reference already existing","errors":{"reference":["Reference already existing"]}}"#.to_owned(),
    )
}

/// The crash ADR-0032 makes safe: an initialize whose answer was lost, and
/// no submit ever recorded for the charge. Nobody asked Notch Pay to prompt
/// the payer, so the `409` is a clean failure the payer can retry on a new
/// charge — `Rejected`, `provider_unavailable` — and nothing more is sent.
#[tokio::test]
async fn a_duplicate_no_submit_ever_prompted_for_fails_the_charge_cleanly() {
    let (origin, server) = serve(vec![duplicate_reference()]).await;
    let outcome = adapter()
        .prepare(&charge(), &config(&origin), PriorSubmits::None)
        .await;
    assert!(
        matches!(
            &outcome,
            Err(ProviderError::Rejected {
                code: FailureCode::ProviderUnavailable,
                ..
            })
        ),
        "{outcome:?}"
    );
    assert_eq!(
        server.await.expect("finishes").len(),
        1,
        "only initialize was sent"
    );
}

/// The same `409` after a recorded submit — a charge the one-call build
/// before ADR-0032 submitted, whose process may have prompted — cannot be
/// resolved: Notch Pay finds payments by its own reference only. It is
/// `NeedsReconciliation`, never `Submitted` and never a decline, and nothing
/// more is sent (vaam-apps/vpay#313).
#[tokio::test]
async fn a_duplicate_a_submit_may_have_prompted_for_needs_reconciliation() {
    let (origin, server) = serve(vec![duplicate_reference()]).await;
    let outcome = adapter()
        .prepare(&charge(), &config(&origin), PriorSubmits::Some)
        .await;
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

/// A second process of a payment already processed — a resubmit after a
/// lost answer — reads the payment, and one past `pending` is `Submitted`
/// with no second prompt.
#[tokio::test]
async fn a_repeated_process_reads_the_payment_and_does_not_prompt_again() {
    let ours = Uuid::from_u128(0x0202).to_string();
    let (origin, server) = serve(vec![
        (
            "HTTP/1.1 422 Unprocessable Entity",
            r#"{"code":422,"status":"Unprocessable Entity","message":"This payment has already been processed"}"#
                .to_owned(),
        ),
        ("HTTP/1.1 200 OK", payment_body(200, &ours, "processing")),
    ])
    .await;
    let submitted = adapter()
        .submit(&submitted_charge(), &config(&origin))
        .await
        .expect("a processed payment is Submitted");
    assert_eq!(
        server.await.expect("finishes"),
        vec![
            format!("POST /payments/{TRX} HTTP/1.1"),
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

/// The fixture signing secret: the shape of Notch Pay's Hash Key
/// (`hsk_test_…`) is deliberately not imitated, so no scanner and no reader
/// mistakes it for one.
const SIGNING: &str = "signing-secret-test_0003";

fn signed_config() -> ProviderConfig {
    let mut config = closed();
    config.credentials.insert(
        vpay_provider::CALLBACK_SIGNING_SECRET.to_owned(),
        SIGNING.to_owned(),
    );
    config
}

/// A `payment.complete` webhook as Notch Pay's documentation shows one,
/// serialised compactly, for [`charge`].
fn webhook_body() -> Vec<u8> {
    serde_json::json!({
        "id": "evt.test_0001",
        "type": "payment.complete",
        "created_at": "2026-10-20T10:00:00Z",
        "data": {
            "id": "pay.test_0001",
            "reference": Uuid::from_u128(0x0202).to_string(),
            "status": "complete",
            "amount": 5_000,
            "currency": "XAF"
        }
    })
    .to_string()
    .into_bytes()
}

fn sign(secret: &str, body: &[u8]) -> String {
    vpay_provider::callback::hmac_sha256_hex(secret.as_bytes(), body).expect("HMAC takes any key")
}

fn headers_with(signature: Option<&str>) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    if let Some(signature) = signature {
        headers.insert(
            "x-notch-signature",
            reqwest::header::HeaderValue::from_str(signature).expect("hex is a header value"),
        );
    }
    headers
}

async fn verdict(
    headers: &reqwest::header::HeaderMap,
    body: &[u8],
    config: &ProviderConfig,
) -> CallbackVerdict {
    let request = CallbackRequest {
        headers,
        body,
        received_at: std::time::SystemTime::now(),
    };
    adapter().verify_callback(&request, &charge(), config).await
}

#[test]
fn notch_pay_declares_the_signature_check() {
    let capabilities = adapter().capabilities();
    assert_eq!(capabilities.callback_check, CallbackCheck::Signature);
    assert!(capabilities.callback_check_is_coherent());
}

/// RFC-0014 § 4: a body signed under the configured secret verifies, and the
/// adapter reads nothing else to say so. The config is [`closed`]: a check
/// that reached for the rail would fail to connect, not verify.
#[tokio::test]
async fn a_webhook_signed_under_the_configured_secret_is_verified() {
    let body = webhook_body();
    let signature = sign(SIGNING, &body);
    assert_eq!(
        verdict(&headers_with(Some(&signature)), &body, &signed_config()).await,
        CallbackVerdict::Verified
    );
    // Notch Pay's samples print lower-case hex; upper-case is the same MAC.
    assert_eq!(
        verdict(
            &headers_with(Some(&signature.to_uppercase())),
            &body,
            &signed_config()
        )
        .await,
        CallbackVerdict::Verified
    );
}

#[tokio::test]
async fn a_webhook_with_no_signature_is_invalid() {
    let body = webhook_body();
    let answer = verdict(&headers_with(None), &body, &signed_config()).await;
    assert!(matches!(answer, CallbackVerdict::Invalid(_)), "{answer:?}");
}

#[tokio::test]
async fn a_webhook_signed_under_another_secret_or_not_at_all_is_invalid() {
    let body = webhook_body();
    for signature in [
        sign("another-secret-test_0004", &body),
        sign(PUBLIC, &body),
        sign(PRIVATE, &body),
        "0".repeat(64),
        "not hex at all".to_owned(),
        String::new(),
    ] {
        let answer = verdict(&headers_with(Some(&signature)), &body, &signed_config()).await;
        assert!(
            matches!(answer, CallbackVerdict::Invalid(_)),
            "{signature:?}: {answer:?}"
        );
    }
}

/// The bytes the route received are what is compared: the same event
/// re-serialised with spaces, or with one byte changed, is not the message
/// Notch Pay signed.
#[tokio::test]
async fn the_signature_covers_the_raw_body_byte_for_byte() {
    let body = webhook_body();
    let signature = sign(SIGNING, &body);
    let value: serde_json::Value = serde_json::from_slice(&body).expect("the fixture is JSON");
    let pretty = serde_json::to_vec_pretty(&value).expect("re-serialises");
    let mut tampered = body.clone();
    let five = tampered
        .windows(4)
        .position(|window| window == b"5000")
        .expect("the amount is in the body");
    *tampered.get_mut(five).expect("within the body") = b'9';
    for altered in [pretty, tampered, [body.as_slice(), b"\n"].concat()] {
        let answer = verdict(&headers_with(Some(&signature)), &altered, &signed_config()).await;
        assert!(
            matches!(answer, CallbackVerdict::Invalid(_)),
            "{}: {answer:?}",
            String::from_utf8_lossy(&altered)
        );
    }
}

/// A rotation: a webhook signed under the previous secret still verifies
/// while it is configured, as `VerifiedPrevious` so that its counter says
/// when the rotation can end, while one under the current secret is plain
/// `Verified`; and a secret rotated out of both keys is stale and refused.
/// Notch Pay signs no timestamp, so "stale" can only mean the key: a genuine
/// webhook replayed later verifies for as long as its secret is configured,
/// which `a_replayed_genuine_webhook_still_verifies` pins. Mutation-checked
/// (2026-10-23): answering `Verified` for either key fails the first
/// assertion.
#[tokio::test]
async fn the_previous_secret_verifies_during_a_rotation_and_a_retired_one_does_not() {
    let body = webhook_body();
    let mut rotating = signed_config();
    rotating.credentials.insert(
        vpay_provider::CALLBACK_SIGNING_SECRET_PREVIOUS.to_owned(),
        "previous-secret-test_0005".to_owned(),
    );
    let under_previous = sign("previous-secret-test_0005", &body);
    assert_eq!(
        verdict(&headers_with(Some(&under_previous)), &body, &rotating).await,
        CallbackVerdict::VerifiedPrevious
    );
    assert_eq!(
        verdict(&headers_with(Some(&sign(SIGNING, &body))), &body, &rotating).await,
        CallbackVerdict::Verified,
        "the current secret is still `verified` during a rotation"
    );
    // The rotation finished: the previous key was removed.
    let answer = verdict(
        &headers_with(Some(&under_previous)),
        &body,
        &signed_config(),
    )
    .await;
    assert!(matches!(answer, CallbackVerdict::Invalid(_)), "{answer:?}");
}

/// The exposure RFC-0012 § 6 and RFC-0014 open question 4 accept, pinned so
/// that it is a decision rather than a surprise: Notch Pay's signature has no
/// timestamp, so a captured genuine webhook verifies again. What it can do
/// is pull one poll forward; it cannot move a charge.
#[tokio::test]
async fn a_replayed_genuine_webhook_still_verifies() {
    let body = webhook_body();
    let signature = sign(SIGNING, &body);
    let headers = headers_with(Some(&signature));
    for _ in 0..2 {
        assert_eq!(
            verdict(&headers, &body, &signed_config()).await,
            CallbackVerdict::Verified
        );
    }
}

/// No secret configured: `NoSecret`, whatever the header says, so the route
/// keeps today's behaviour and the counter says why. Boot is what refuses
/// this in livemode (RFC-0014 § 6).
#[tokio::test]
async fn with_no_signing_secret_the_check_answers_no_secret() {
    let body = webhook_body();
    let forged = sign("anything", &body);
    let mut blank = closed();
    blank.credentials.insert(
        vpay_provider::CALLBACK_SIGNING_SECRET.to_owned(),
        "   ".to_owned(),
    );
    for config in [closed(), blank] {
        for headers in [headers_with(None), headers_with(Some(&forged))] {
            assert_eq!(
                verdict(&headers, &body, &config).await,
                CallbackVerdict::NoSecret
            );
        }
    }
}

/// A refusal's reason is a fixed sentence: it never quotes the signature it
/// was handed, the secret it compared against, or a body byte, because the
/// route logs it.
#[tokio::test]
async fn a_refusal_carries_no_secret_signature_or_body() {
    let body = webhook_body();
    let forged = sign("forger-secret-test_0006", &body);
    for headers in [headers_with(None), headers_with(Some(&forged))] {
        let answer = verdict(&headers, &body, &signed_config()).await;
        let reason = answer.reason().expect("a refusal has a reason");
        for leaked in [SIGNING, forged.as_str(), "evt.test_0001", "5000"] {
            assert!(!reason.contains(leaked), "{reason}");
        }
        assert!(!format!("{answer:?}").contains(SIGNING));
    }
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
/// callback in that shape names our charge. (Measured on a delivered webhook
/// on 2026-10-24: see the next test.)
/// A `payment.complete` webhook byte for byte as the sandbox delivered it on
/// 2026-10-24. It names itself under `event`, not the documented `type`:
/// until that date `parse_callback` read `type` only and refused all four
/// real deliveries as malformed, so the route would have answered `400`.
const DELIVERED_PAYMENT_COMPLETE: &str = r#"{"id":"whc_test.hwFmiN6rmtv96e7z","event":"payment.complete","data":{"amount":151,"fees":[],"amounts":{"total":151,"converted":151,"currency":"XAF","rate":1},"sandbox":1,"payment_method":"pm.test_DkA37f7vq7SouqFH","description":"vpay sandbox probe (ADR-0032 \/ RFC-0012 step 8)","reference":"trx.test_vwTAcbfxYr5cgcnpCQpYzglh","merchant_reference":"a9d73439-cb96-4b41-956b-46c520d9600d","trxref":"a9d73439-cb96-4b41-956b-46c520d9600d","status":"complete","currency":"XAF","customer":"cus.test_Ph6xD6j0XGHRRTbp","charge":"business","callback":null,"created_at":"2026-10-24T08:26:53.000000Z"}}"#;

#[test]
fn a_webhook_the_sandbox_delivered_names_our_charge() {
    let parsed = adapter()
        .parse_callback(DELIVERED_PAYMENT_COMPLETE.as_bytes())
        .expect("a delivered payment webhook parses");
    assert_eq!(
        parsed.reference_id,
        Uuid::parse_str("a9d73439-cb96-4b41-956b-46c520d9600d").expect("uuid"),
        "named by data.trxref, vpay's own reference"
    );
    assert!(parsed.ref_extra.is_empty());
}

/// The unsigned ping the dashboard sends on registration (2026-10-24) is not
/// a payment event and names no charge, so it is refused as today. Whether
/// Notch Pay needs a 2xx for it to keep an endpoint is not known.
#[test]
fn the_registration_ping_is_not_a_payment_event() {
    let ping = r#"{"id":"evt_verification_yiU5Gsm0Ti","type":"verification.test","created":1792829985,"data":{"message":"This is a check of your webhook endpoint","timestamp":"2026-10-24T09:19:45+01:00"}}"#;
    assert!(matches!(
        adapter().parse_callback(ping.as_bytes()),
        Err(ProviderError::Malformed { .. })
    ));
}

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
