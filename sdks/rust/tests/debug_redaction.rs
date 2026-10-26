//! The signing seed, a `PaymentIntent`'s
//! `client_secret` and a `CheckoutSession`'s must never reach a `{:?}`.
//!
//! This file is named by the `Debug` implementations of
//! [`vpay_sdk::Credentials`], `Client`, `PaymentIntent` and
//! `CheckoutSession` themselves: they
//! are hand-written rather than derived *because* of these tests, and
//! replacing any of them with `#[derive(Debug)]` must fail here. A
//! merchant's signing seed or payer credential in a log line
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

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use vpay_sdk::payment_intents::PaymentMethodType;
use vpay_sdk::refunds::{CreateRefundParams, RefundDestination};
use vpay_sdk::{CheckoutSession, Client, Credentials, PaymentIntent, ServerKeys};

/// A seed that is its own giveaway: 32 bytes of one value, so the base64url of
/// it is a recognisable run and a `Debug` that printed the seed in any common
/// encoding contains one of the strings below.
const SEED: [u8; 32] = [0x5a; 32];

/// The forms a leaked seed would take: its base64url, its standard base64, its
/// hex, and its `[u8; 32]` debug rendering.
fn assert_no_key_material(rendered: &str) {
    let forms = [
        URL_SAFE_NO_PAD.encode(SEED),
        base64::engine::general_purpose::STANDARD.encode(SEED),
        "5a".repeat(32),
        format!("{SEED:?}"),
    ];
    for form in forms {
        assert!(
            !rendered.contains(&form),
            "the seed leaked as {form}: {rendered}"
        );
    }
}

fn keys() -> ServerKeys {
    let public_of = |seed: u8| {
        cratestack_client_rust::cose::Ed25519Signer::from_seed(&[seed; 32])
            .verify_key()
            .ed25519_bytes()
            .expect("an Ed25519 key")
    };
    ServerKeys::new(public_of(1), public_of(2))
}

#[test]
fn credentials_debug_shows_the_client_id_and_the_public_key_but_no_key_material() {
    let credentials = Credentials::ed25519_seed("merchant_acme", &SEED);

    for rendered in [format!("{credentials:?}"), format!("{credentials:#?}")] {
        assert_no_key_material(&rendered);
        // Still useful: the fields an operator actually needs are present, and
        // the public half is what they register under `cose_keys`.
        assert!(rendered.contains("merchant_acme"));
        assert!(rendered.contains(&credentials.public_key_base64url()));
        assert!(rendered.contains("[redacted]"));
    }
}

#[test]
fn client_and_builder_debug_carry_no_key_material() {
    let builder = Client::builder("https://api.vpay.test")
        .credentials(Credentials::ed25519_seed("merchant_acme", &SEED))
        .server_keys(keys());
    assert_no_key_material(&format!("{builder:?}"));

    let client = builder.build().unwrap();
    assert_no_key_material(&format!("{client:?}"));
    assert_no_key_material(&format!("{client:#?}"));
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
fn a_rejected_seed_does_not_echo_the_seed_into_the_error() {
    // The error path is the easiest place for a credential to escape: an
    // error string built from the input is a log line by another name.
    let err = Credentials::ed25519_seed_base64url("merchant_acme", "SUPERSECRETSEEDTEXTxx")
        .expect_err("a seed of the wrong length is refused");
    let rendered = format!("{err} / {err:?}");
    assert!(
        !rendered.contains("SUPERSECRETSEEDTEXT"),
        "the supplied seed was echoed: {rendered}"
    );
}

/// A refund's payee is a **third party's** phone number, and the merchant is
/// holding it on their behalf.
///
/// vpay's own handler logs it masked and never raw, renders it in no response
/// and in no event body, and keeps no column for it — RFC-0003 rejected
/// carrying it in `metadata` precisely because metadata is inside every
/// signed webhook. An SDK whose params struct printed it in full would put
/// the same number in the *merchant's* logs one hop earlier, which is the
/// same leak with a different owner.
///
/// Both types are named, because either one is enough to lose it: the
/// destination alone, and the params struct that carries it.
#[test]
fn a_create_refund_params_debug_output_never_contains_the_payees_number() {
    let msisdn = "+237600000200";
    let destination = RefundDestination::mobile_money(PaymentMethodType::MtnMomo, msisdn);
    let params = CreateRefundParams {
        payment_intent: "pi_1".to_string(),
        amount: Some(2500),
        destination: Some(destination.clone()),
        ..Default::default()
    };

    for rendered in [
        format!("{destination:?}"),
        format!("{destination:#?}"),
        format!("{params:?}"),
        format!("{params:#?}"),
    ] {
        assert!(
            !rendered.contains(msisdn),
            "the payee's number leaked: {rendered}"
        );
        // Not merely absent — absent *and* replaced by something that says
        // so, so an operator reading the line knows a value was withheld
        // rather than never supplied.
        assert!(rendered.contains("[redacted]"), "{rendered}");
        // The number without its `+` must not survive either: a `Debug` that
        // printed the digits would be no better for the person they identify.
        assert!(!rendered.contains("237600000200"), "{rendered}");
        // Still useful: the rail is what a merchant debugging a 400 needs.
        assert!(
            rendered.contains("mtn_momo") || rendered.contains("MtnMomo"),
            "{rendered}"
        );
    }
}
