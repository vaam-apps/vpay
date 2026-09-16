//! The refund surface against a **real** `vpay-server`, over a socket.
//!
//! # Why this file exists at all
//!
//! `tests/resources.rs` proves what this SDK *sends*, against a `wiremock`
//! that answers whatever the case told it to. That is not evidence that the
//! server accepts it, and this repository has been burned by exactly that
//! twice: `cargo xtask verify-sdk-parity` proves a **name** exists and not
//! that the SDK sends the field (issue #122), and a fixture once invented a
//! `client_secret` the server never sends — unit tests, review, mutation
//! testing and CI all passed while the client was broken against real HTTP.
//!
//! The refund surface is the one where that would cost the most. Its
//! `destination` is a rail-agnostic envelope around a rail-specific interior
//! (`destination[<payment_method_type>][msisdn]`), the server strips the
//! outer key and hands the interior to that rail's own adapter, and the MSISDN
//! rule it applies there is **not** the rule `GET /v1/account_holders`
//! applies: a payee must be international, starting with `+`. Every one of
//! those is a thing a stub cannot disagree with and a real server can.
//!
//! # What it does NOT prove
//!
//! **That any money moved, or that MTN refunds work.** The rail here is a
//! `wiremock/wiremock` container answering `POST /disbursement/v1_0/transfer`
//! with the `202` this repository transcribed from MTN's documentation.
//! MTN's Disbursements product has never been called from this repository, in
//! sandbox or anywhere else. And **nothing settles a `pending` refund** —
//! there is no refund poll ladder (RFC-0003 open question 8) — which is why
//! the case below asserts `pending` after a `201` and would fail if the
//! server ever started claiming otherwise without one.
//!
//! # It never skips
//!
//! The target is compiled only under the `live-stack` feature (see
//! `Cargo.toml`), and once compiled it **fails** rather than skipping when
//! there is no stack: a missing `VPAY_BASE_URL`, an unreadable key, an
//! unanswered preflight are each a panic naming what is wrong. AGENTS.md
//! rule 2 — a green run never overstates coverage — is why there is no
//! `#[ignore]` here and no `if env.is_none() { return }`.
//!
//! # Running it
//!
//! ```text
//! just sdk-live                      # brings a stack up and runs this
//! # or, against a stack you already have:
//! VPAY_BASE_URL=http://localhost:18080 \
//! VPAY_MERCHANT_CLIENT_ID=demo-merchant \
//! VPAY_MERCHANT_PRIVATE_KEY_PATH=$PWD/.e2e/demo-merchant/oauth-signing-key.pem \
//!   cargo nextest run -p vpay-sdk --features live-stack --test live_refunds
//! ```
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a test binary; clippy.toml exempts #[cfg(test)] modules, which an integration \
              test file is not"
)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use vpay_sdk::events::ListEventsParams;
use vpay_sdk::payment_intents::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, PaymentMethodType,
};
use vpay_sdk::refunds::{
    CreateRefundParams, ListRefundsParams, RefundDestination, UpdateRefundParams,
};
use vpay_sdk::{Client, Credentials, Error, IntentStatus, RefundStatus, RequestOptions};

/// The payer the demo MTN stub settles for, and whose refund destination the
/// same stub knows a holder for (`wiremock/mtn/mappings/basicuserinfo.json`).
const PAYER_MSISDN: &str = "237600000100";

/// The payee, **in the form the refund path requires** — international, with
/// a `+`. `wiremock/mtn/mappings/basicuserinfo.json` answers a named holder
/// for its digits, which is what `verify_registered_holder` asks for.
const PAYEE_MSISDN: &str = "+237600000200";

/// The payee the rail has **no record of** — `basicuserinfo` answers `404`,
/// which the adapter maps to `Ok(None)` and the handler to a `400`.
const UNKNOWN_PAYEE_MSISDN: &str = "+237600000404";

/// A ceiling many times the usual settlement, not an expectation. A window
/// that closes on `processing` is an assertion failure naming the worker.
const SETTLE_WINDOW: Duration = Duration::from_secs(60);

/// Reads one required variable, or fails the case by name.
///
/// Not `unwrap_or_default`, and not an `Option` the caller may ignore: the
/// two mistakes this file exists to rule out are running against nothing and
/// reporting that as success.
fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_error| {
        panic!(
            "{name} is not set. This suite runs against a REAL vpay stack; `just sdk-live` \
             brings one up and sets VPAY_BASE_URL, VPAY_MERCHANT_CLIENT_ID and \
             VPAY_MERCHANT_PRIVATE_KEY_PATH for you."
        )
    })
}

/// A client for the configured merchant, after proving the stack answers it.
///
/// `tests/live_invoices.rs`'s preflight, and its reason: the probe is one
/// authenticated read through this very SDK, so "nothing is listening" and
/// "this key is not the pair this stack registered" both surface here with
/// the server's own words rather than inside the first case.
async fn live_client() -> Client {
    let base = required("VPAY_BASE_URL");
    let client_id = required("VPAY_MERCHANT_CLIENT_ID");
    let key_path = required("VPAY_MERCHANT_PRIVATE_KEY_PATH");
    let pem = std::fs::read_to_string(&key_path).unwrap_or_else(|error| {
        panic!("cannot read the merchant private key at {key_path}: {error}")
    });

    let credentials = Credentials::rsa_pem(&client_id, &pem)
        .unwrap_or_else(|error| panic!("{key_path} is not a usable RSA private key: {error}"));
    let client = Client::builder(&base)
        .credentials(credentials)
        .build()
        .expect("the client builds from a base URL and credentials");

    if let Err(error) = client.events().list(ListEventsParams::default()).await {
        panic!(
            "the preflight read of {base}/v1/events failed as {client_id}: {error}. Either no \
             vpay is listening there, or {key_path} is not the key this stack registered. \
             `just sdk-live` brings up a stack whose `demo` overlay registers it."
        );
    }
    client
}

/// The `(status, param)` of an API error, or a panic naming what came back
/// instead — so a case that expected a refusal and got a refund says so.
fn api_refusal(error: &Error, what: &str) -> (u16, Option<String>) {
    match error {
        Error::Api {
            status,
            param,
            message,
            ..
        } => {
            // The rule the server applies must never reach a merchant with the
            // number in it: the payee is a third party. Asserted here, on the
            // real body, because this is the only place in the workspace where
            // the real message and the real number are both in hand.
            assert!(
                !message.contains("600000200") && !message.contains("600000404"),
                "{what}: the refusal echoed the payee's number: {message}"
            );
            (*status, param.clone())
        }
        other => panic!("{what}: expected an API refusal, got {other}"),
    }
}

/// An intent charged on MTN and settled by the worker, which is what a refund
/// needs: `POST /v1/refunds` reads the rail off the **charge**, and an intent
/// with no charge is a `409` before any destination is looked at.
async fn a_settled_intent(client: &Client, amount: i64) -> String {
    let created = client
        .payment_intents()
        .create(
            CreatePaymentIntentParams {
                amount,
                currency: "XAF".to_owned(),
                payment_method_types: vec![PaymentMethodType::MtnMomo],
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("an intent is created");

    let confirmed = client
        .payment_intents()
        .confirm(
            &created.id,
            ConfirmPaymentIntentParams::mtn_momo(PAYER_MSISDN),
            RequestOptions::new(),
        )
        .await
        .expect("the intent confirms on a push rail");
    assert!(
        matches!(
            confirmed.status,
            IntentStatus::Processing | IntentStatus::Succeeded
        ),
        "a confirmed push-rail intent is processing or already settled, not {:?}",
        confirmed.status
    );

    // Bounded, and it FAILS rather than hangs. A status that is neither
    // `processing` nor `succeeded` fails immediately instead of being polled
    // past: a decline is a real, terminal answer about this payment.
    let deadline = Instant::now() + SETTLE_WINDOW;
    loop {
        let read = client
            .payment_intents()
            .retrieve(&created.id)
            .await
            .expect("the intent reads back");
        match read.status {
            IntentStatus::Succeeded => return created.id,
            IntentStatus::Processing => {}
            other => panic!("the intent reached {other:?}, which is terminal and is not a charge"),
        }
        assert!(
            Instant::now() < deadline,
            "the intent was still `processing` after {SETTLE_WINDOW:?}: the `vpay-worker` \
             container is the most likely cause"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// **The MSISDN rule, against the real server.**
///
/// `RefundTarget::mobile_money` canonicalises and requires a leading `+`.
/// The bare national `600000200` is refused — even though
/// `GET /v1/account_holders` accepts it, because that route is
/// Cameroon-specific and `vpay-provider` is not: read as an international
/// number, `600000200` begins with country code `6` and names a payee in
/// Malaysia.
///
/// This is the case that a fixture cannot write honestly. An SDK that
/// normalised the number into the national form, or that flattened the
/// envelope, would pass every case in `tests/resources.rs` and land here.
///
/// The three refusals are one case because they share an expensive fixture
/// (an intent that has to settle) and because what they assert is one thing:
/// the refusal names `destination`, and it names it for a different reason
/// each time.
#[tokio::test]
async fn live_refund_destination_refusals() {
    let client = live_client().await;
    let intent = a_settled_intent(&client, 5_000).await;

    // 1. No destination at all. Both rails vpay carries declare that a refund
    //    needs a payee, so this is the regression Arm F left and this arm
    //    closes: it was a `404` before the route was mounted.
    let error = client
        .refunds()
        .create(
            CreateRefundParams {
                payment_intent: intent.clone(),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect_err("a Required rail refuses a refund with no payee");
    let (status, param) = api_refusal(&error, "no destination");
    assert_eq!(status, 400);
    assert_eq!(param.as_deref(), Some("destination"));

    // 2. A number with no `+`. It reaches the adapter's parser — which is the
    //    half that proves the envelope was read — and is refused there.
    let error = client
        .refunds()
        .create(
            CreateRefundParams {
                payment_intent: intent.clone(),
                destination: Some(RefundDestination::mobile_money(
                    PaymentMethodType::MtnMomo,
                    "600000200",
                )),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect_err("a payee with no `+` is refused");
    let (status, param) = api_refusal(&error, "no leading +");
    assert_eq!(status, 400);
    assert_eq!(param.as_deref(), Some("destination"));

    // 3. A well-formed number the rail has no holder for. Past the parser,
    //    refused by the account-holder lookup — so this one proves the
    //    interior really was handed to `mtn_momo`'s adapter and not merely
    //    shape-checked.
    let error = client
        .refunds()
        .create(
            CreateRefundParams {
                payment_intent: intent.clone(),
                destination: Some(RefundDestination::mobile_money(
                    PaymentMethodType::MtnMomo,
                    UNKNOWN_PAYEE_MSISDN,
                )),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect_err("a payee the rail has no record of is refused");
    let (status, param) = api_refusal(&error, "unregistered payee");
    assert_eq!(status, 400);
    assert_eq!(param.as_deref(), Some("destination"));

    // None of the three wrote a refund. A refusal that cost a row would show
    // up here, and a merchant's next full refund would be short by its amount.
    let page = client
        .refunds()
        .list(ListRefundsParams {
            payment_intent: Some(intent.clone()),
            ..Default::default()
        })
        .await
        .expect("the list filter is accepted");
    assert!(
        page.data.is_empty(),
        "a refused refund must cost no row, and {} exist",
        page.data.len()
    );
}

/// **create → retrieve → update → list → refused cancel**, against a running
/// vpay.
///
/// One case for the sequence rather than five, because the states are
/// sequential: a refund must exist to be read, and what the cancel proves is
/// about the refund the four steps before it built, so five independent cases
/// would each rebuild the ones before.
///
/// What it pins that no stub could:
///
/// * the `destination[<rail>][msisdn]` envelope this SDK writes is one the
///   real server strips, hands to the real `mtn_momo` adapter, and accepts;
/// * a `201` leaves the refund **`pending`** — nothing settles one, and an
///   `Ok` from the rail is an acceptance and not a settlement;
/// * the refund object really carries ten keys' worth of what this SDK
///   decodes, including `fee: null`;
/// * the update really merges metadata key-wise, and an empty value really
///   deletes a key — a rule this SDK only documents;
/// * a refund the rail has already been given is **not** cancelable — a real
///   `409` — and the refusal leaves both the refund and its reservation
///   exactly where they were.
///
/// The cancel that *does* fire has no live case here and cannot have one: it
/// serves the crashed-create state (a `pending` refund over a minute old with
/// no `provider_requests` row), which a client driving a healthy server
/// through HTTP has no way to produce. `backends/tests/integration/tests/refunds.rs`
/// stages it directly; `docs/sdks/parity.md` says so rather than letting this
/// file look like it covers a route it does not.
#[tokio::test]
async fn live_refund_lifecycle() {
    let client = live_client().await;
    let intent = a_settled_intent(&client, 5_000).await;

    let refund = client
        .refunds()
        .create(
            CreateRefundParams {
                payment_intent: intent.clone(),
                amount: Some(2_000),
                reason: Some("requested_by_customer".to_owned()),
                destination: Some(RefundDestination::mobile_money(
                    PaymentMethodType::MtnMomo,
                    PAYEE_MSISDN,
                )),
                metadata: BTreeMap::from([
                    ("order_id".to_owned(), "w3sdks".to_owned()),
                    ("stale".to_owned(), "delete-me".to_owned()),
                ]),
            },
            RequestOptions::new(),
        )
        .await
        .expect("a partial refund with a registered payee is accepted");

    assert!(refund.id.starts_with("re_"), "{}", refund.id);
    assert_eq!(refund.payment_intent, intent);
    assert_eq!(refund.amount, 2_000);
    assert_eq!(refund.currency, "xaf", "lower-case on the wire");
    assert_eq!(refund.reason.as_deref(), Some("requested_by_customer"));
    // **The claim this file must not let drift.** Nothing settles a refund —
    // there is no refund poll ladder — so an accepted instruction leaves the
    // refund `pending` and this assertion is what fails the day something
    // starts writing `succeeded` without one.
    assert_eq!(refund.status, RefundStatus::Pending);
    // No rail reports a refund fee and nothing in this repository stores one.
    assert_eq!(refund.fee, None);

    // The destination is on no response. It is a third party's personal data,
    // it is in no column, and RFC-0003 rejected carrying it in `metadata`
    // precisely because metadata is inside every signed webhook vpay
    // delivers. Asserted on the real object rather than trusted.
    let rendered = format!("{refund:?}");
    assert!(
        !rendered.contains("600000200"),
        "the payee reached the refund object: {rendered}"
    );

    let read = client
        .refunds()
        .retrieve(&refund.id)
        .await
        .expect("the refund reads back");
    assert_eq!(read.id, refund.id);
    assert_eq!(read.status, refund.status);
    assert_eq!(
        read.metadata.get("order_id").map(String::as_str),
        Some("w3sdks")
    );

    let updated = client
        .refunds()
        .update(
            &refund.id,
            UpdateRefundParams {
                metadata: BTreeMap::from([
                    ("order_id".to_owned(), "w3sdks-updated".to_owned()),
                    ("added".to_owned(), "yes".to_owned()),
                    // Stripe's per-key delete.
                    ("stale".to_owned(), String::new()),
                ]),
            },
            RequestOptions::new(),
        )
        .await
        .expect("metadata is patchable while the refund exists");
    assert_eq!(
        updated.metadata.get("order_id").map(String::as_str),
        Some("w3sdks-updated"),
        "a key sent with a value is replaced"
    );
    assert_eq!(
        updated.metadata.get("added").map(String::as_str),
        Some("yes"),
        "a key not previously present is added"
    );
    assert!(
        !updated.metadata.contains_key("stale"),
        "an empty value deletes the key, rather than storing an empty string"
    );

    // The four fields this params type cannot send are refused by the server,
    // which is what makes the type's omission of them a statement rather than
    // a convenience. Sent by hand, because there is deliberately no SDK path
    // to it.
    let error = client
        .refunds()
        .create(
            CreateRefundParams {
                payment_intent: intent.clone(),
                amount: Some(-1),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect_err("a negative amount is refused");
    assert!(
        matches!(error, Error::InvalidParams { .. }),
        "and refused BEFORE any request: {error}"
    );

    let page = client
        .refunds()
        .list(ListRefundsParams {
            payment_intent: Some(intent.clone()),
            ..Default::default()
        })
        .await
        .expect("the payment_intent filter is accepted");
    assert!(
        page.data.iter().any(|item| item.id == refund.id),
        "the refund is on its own intent's page"
    );
    assert_eq!(page.url, "/v1/refunds");

    // **The rail already has this instruction, so it cannot be cancelled.**
    // `POST /v1/refunds` writes the `provider_requests` row *before* it sends
    // the transfer, and `vpay_db::refunds::cancel_in_tx`'s `NOT EXISTS` reads
    // that row as "a rail may already be moving this money". A cancel would
    // write `canceled` — a promise that no money will move — and hand the
    // reservation back: measured on this branch before the guard existed as
    // two 5 000 transfers on WireMock's journal against one 5 000 charge
    // (commit 4bcf6491).
    //
    // **This case asserted a `200 canceled` here until 2026-09-16.** It was
    // written against the contract as it stood, the guard landed the same
    // day, and nothing objected because no CI job compiled this binary —
    // `verify-sdk-parity` proves a test *name* exists, never that anything
    // runs it (issue #122). Nothing settles a refund (RFC-0003 open question
    // 8), so every refund this route creates sits `pending` for ever with its
    // transfer already accepted, which is what made every one of them
    // cancelable.
    let error = client
        .refunds()
        .cancel(&refund.id, RequestOptions::new())
        .await
        .expect_err("a refund the rail has already been given cannot be cancelled");
    let (status, _param) = api_refusal(&error, "cancel after the rail was instructed");
    assert_eq!(
        status, 409,
        "the state machine is the server's WHERE clause"
    );

    // The refusal cost nothing. A cancel that failed *after* releasing the
    // reservation would be the same double refund with an error code on it,
    // so both halves are asserted: the refund did not move, and the money it
    // reserved is still reserved.
    let after_refusal = client
        .refunds()
        .retrieve(&refund.id)
        .await
        .expect("the refused cancel left the refund readable");
    assert_eq!(
        after_refusal.status,
        RefundStatus::Pending,
        "a refused cancel must not move the refund"
    );

    // The reservation, read the only way a merchant can: through what is left
    // to refund. 2 000 of the 5 000 is spoken for, so a refund that names no
    // `amount` is the remaining 3 000 — and a 5 000 here would be the
    // over-refund the reservation exists to prevent.
    let again = client
        .refunds()
        .create(
            CreateRefundParams {
                payment_intent: intent.clone(),
                destination: Some(RefundDestination::mobile_money(
                    PaymentMethodType::MtnMomo,
                    PAYEE_MSISDN,
                )),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("what is left of the charge can still be refunded");
    assert_eq!(
        again.amount, 3_000,
        "the first refund's reservation is still held, so a full refund is what is LEFT of \
         the charge and not the charge again"
    );

    // Nothing can be cancelled back, so the intent ends this case fully spoken
    // for — asserted rather than left implied, because it is the arithmetic a
    // merchant's ledger has to agree with.
    let error = client
        .refunds()
        .create(
            CreateRefundParams {
                payment_intent: intent.clone(),
                amount: Some(1),
                destination: Some(RefundDestination::mobile_money(
                    PaymentMethodType::MtnMomo,
                    PAYEE_MSISDN,
                )),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect_err("5 000 of a 5 000 charge is reserved; there is nothing left");
    let (status, _param) = api_refusal(&error, "over-refund");
    assert_eq!(
        status, 409,
        "an over-refund is a conflict, not a validation"
    );
}
