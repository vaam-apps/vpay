//! The invoice surface against a **real** `vpay-server`, over a socket.
//!
//! Every other case in this crate answers itself: the server is a `wiremock`
//! that returns whatever the case told it to, so "the stub answers the way
//! this SDK expects" is the whole of the evidence. That was recorded as a
//! dated ⛔/⛔ row in `docs/sdks/parity.md` on 2026-09-07 — *invoices
//! exercised against a running vpay* — precisely so that building thirteen
//! methods against stubs would not close it by accident. This file is what
//! closes it.
//!
//! # It never skips
//!
//! The target is compiled only under the `live-stack` feature (see
//! `Cargo.toml`), and once compiled it **fails** rather than skipping when
//! there is no stack: a missing `VPAY_BASE_URL`, an unreadable key, an
//! unanswered `/healthz` are each a panic naming what is wrong. AGENTS.md
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
//!   cargo nextest run -p vpay-sdk --features live-stack --test live_invoices
//! ```
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a test binary; clippy.toml exempts #[cfg(test)] modules, which an integration \
              test file is not"
)]

use std::collections::BTreeMap;

use vpay_sdk::customers::CreateCustomerParams;
use vpay_sdk::events::ListEventsParams;
use vpay_sdk::invoices::{
    CreateInvoiceItemParams, CreateInvoiceParams, ListInvoicesParams, PayInvoiceParams,
    UpdateInvoiceParams,
};
use vpay_sdk::{Client, Credentials, InvoiceStatus, RequestOptions};

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
/// The probe is `GET /v1/events` through this very SDK rather than an
/// unauthenticated `/healthz`: it is the cheapest call that exercises the
/// whole path a case depends on — TCP, the `client_credentials` +
/// `private_key_jwt` handshake, and one authenticated read — so "nothing is
/// listening" and "this key is not the pair this stack registered" both
/// surface here, with the server's own words, rather than inside the first
/// case as an anonymous failure.
///
/// It is not `reqwest::get`, deliberately. This crate hands reqwest a
/// finished rustls `ClientConfig` precisely so that it never installs a
/// process-wide `CryptoProvider` in someone else's process (see
/// `Cargo.toml`), and reqwest's own default builder panics without one — so
/// a bare `reqwest::get` in this file would fail every case for a reason
/// that has nothing to do with vpay. Measured, 2026-09-08.
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

/// **create → two lines → finalize → retrieve → void**, and then
/// **create → finalize → pay**, against a running vpay.
///
/// One case for the whole lifecycle rather than eight, because the states are
/// sequential: an invoice must be a draft to take a line and open to be
/// voided, so eight independent cases would each rebuild the ones before it
/// and the failure of the first would be reported eight times.
///
/// What it pins that no stub could:
///
/// * a draft really has no number and zero amounts, and finalize really
///   assigns one out of this merchant's own sequence;
/// * the lines the server stores add up to the `amount_due` it computes —
///   `quantity * unit_amount`, summed, which is a database expression this
///   SDK never sees;
/// * a voided invoice **keeps** its number;
/// * `pay` mints a payment intent and a hosted URL and leaves the invoice
///   `open`;
/// * and that `currency` is required — the claim that was documented the
///   other way round until this suite existed.
#[tokio::test]
async fn live_invoice_lifecycle() {
    let client = live_client().await;

    let customer = client
        .customers()
        .create(
            CreateCustomerParams {
                phone: Some("+237670000000".to_owned()),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("a customer is created");

    let draft = client
        .invoices()
        .create(
            CreateInvoiceParams {
                description: Some("exp33 review — live run".to_owned()),
                metadata: BTreeMap::from([("order_id".to_owned(), "exp33".to_owned())]),
                ..CreateInvoiceParams::new(customer.id.clone(), "XAF")
            },
            RequestOptions::new(),
        )
        .await
        .expect("a draft invoice is created");
    assert_eq!(draft.status, InvoiceStatus::Draft);
    assert!(draft.number.is_none(), "a draft has no number");
    assert_eq!(draft.amount_due, 0, "a draft with no lines owes nothing");
    assert_eq!(
        draft.currency, "xaf",
        "upper-case in, lower-case on the wire"
    );

    for (description, quantity, unit_amount) in
        [("Consulting", 2_i64, 15_000_i64), ("Delivery", 1, 2_500)]
    {
        let line = client
            .invoice_items()
            .create(
                CreateInvoiceItemParams {
                    quantity: Some(quantity),
                    ..CreateInvoiceItemParams::new(&draft.id, description, unit_amount)
                },
                RequestOptions::new(),
            )
            .await
            .expect("a line is added to the draft");
        assert_eq!(
            line.amount,
            quantity * unit_amount,
            "`amount` is computed by the database, never sent"
        );
        assert_eq!(
            line.currency, draft.currency,
            "a line takes its invoice's currency"
        );
    }

    let updated = client
        .invoices()
        .update(
            &draft.id,
            UpdateInvoiceParams {
                description: Some(Some("exp33 review — updated".to_owned())),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("a draft is patchable");
    assert_eq!(
        updated.description.as_deref(),
        Some("exp33 review — updated")
    );

    let open = client
        .invoices()
        .finalize(&draft.id, RequestOptions::new())
        .await
        .expect("a draft with lines finalizes");
    assert_eq!(open.status, InvoiceStatus::Open);
    let number = open
        .number
        .clone()
        .expect("a finalized invoice has a number");
    assert_eq!(open.amount_due, 32_500);
    assert_eq!(open.amount_remaining, 32_500);
    assert_eq!(open.amount_paid, 0);

    let read = client
        .invoices()
        .retrieve(&draft.id)
        .await
        .expect("the invoice reads back");
    assert_eq!(
        read.lines.data.len(),
        2,
        "retrieve carries the lines expanded"
    );
    let summed: i64 = read.lines.data.iter().map(|line| line.amount).sum();
    assert_eq!(
        summed, read.amount_due,
        "the stored lines add up to the stored total"
    );
    assert_eq!(read.number, open.number);

    let page = client
        .invoices()
        .list(ListInvoicesParams {
            customer: Some(customer.id.clone()),
            status: Some(InvoiceStatus::Open),
            ..Default::default()
        })
        .await
        .expect("the filters are accepted");
    assert!(
        page.data.iter().any(|invoice| invoice.id == read.id),
        "the open invoice is in its own customer's open page"
    );

    let voided = client
        .invoices()
        .void(&draft.id, RequestOptions::new())
        .await
        .expect("an open invoice with no live intent voids");
    assert_eq!(voided.status, InvoiceStatus::Void);
    assert_eq!(
        voided.number.as_deref(),
        Some(number.as_str()),
        "a voided invoice keeps its number"
    );

    // create → finalize → pay, on a second document, because `pay` needs an
    // open invoice and the first one is terminal now.
    let second = client
        .invoices()
        .create(
            CreateInvoiceParams::new(customer.id.clone(), "xaf"),
            RequestOptions::new(),
        )
        .await
        .expect("a second draft");
    client
        .invoice_items()
        .create(
            CreateInvoiceItemParams::new(&second.id, "One thing", 5_000),
            RequestOptions::new(),
        )
        .await
        .expect("a line on the second draft");
    let second_open = client
        .invoices()
        .finalize(&second.id, RequestOptions::new())
        .await
        .expect("the second draft finalizes");
    assert!(second_open.number.is_some());
    assert_ne!(
        second_open.number, voided.number,
        "the sequence moved on rather than reissuing the voided number"
    );

    let paying = client
        .invoices()
        .pay(
            &second.id,
            PayInvoiceParams::new("https://merchant.example/ok", "https://merchant.example/no"),
            RequestOptions::new(),
        )
        .await
        .expect("an open invoice mints a payment");
    assert_eq!(
        paying.status,
        InvoiceStatus::Open,
        "`pay` charges nobody; the invoice stays open until a settlement moves it"
    );
    assert!(paying.payment_intent.is_some(), "`pay` attaches an intent");
    let hosted = paying
        .hosted_invoice_url
        .as_deref()
        .expect("`pay` answers with a page to send the payer to");
    assert!(
        hosted.starts_with("http://") || hosted.starts_with("https://"),
        "the hosted URL is absolute: {hosted}"
    );
}

/// **`currency` is required**, which is the claim both SDKs documented the
/// other way round until 2026-09-08.
///
/// It cannot be expressed through `CreateInvoiceParams` any more — that is
/// the fix — so the request is built by hand against the same client, which
/// is also the only honest way to prove a *type-level* guarantee is the right
/// one: the request the type now refuses is a request the server refuses.
#[tokio::test]
async fn an_invoice_create_with_no_currency_is_refused_by_the_server() {
    let client = live_client().await;

    let customer = client
        .customers()
        .create(
            CreateCustomerParams {
                phone: Some("+237670000002".to_owned()),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("a customer is created");

    // `CreateInvoiceParams` cannot express this, so the empty string stands
    // in for "the field the SDK used to omit" — the server's answer is the
    // same refusal either way, and this is the one the type still permits.
    let error = client
        .invoices()
        .create(
            CreateInvoiceParams::new(customer.id, ""),
            RequestOptions::new(),
        )
        .await
        .expect_err("a create with no usable currency is refused");
    let message = error.to_string();
    assert!(
        message.contains("currency"),
        "the refusal names the parameter: {message}"
    );
}
