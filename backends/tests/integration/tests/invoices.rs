//! `/v1/invoices` and `/v1/invoice_items`, end to end: the real
//! `vpay_api::router` on a real socket, over a real Postgres.
//!
//! The claims only this file can make:
//!
//! 1. **the number sequence is a sequence.** Two finalizes racing each other
//!    take two *consecutive* numbers — no gap, no reuse — and a finalize that
//!    is refused takes none at all, because the sequence is an ordinary table
//!    row rolled back with its transaction rather than a Postgres `SEQUENCE`;
//! 2. **an issued document is frozen.** After finalize, `PATCH`, `DELETE` and
//!    every line write are refused, and the refusal is the statement's own
//!    `WHERE` rather than a check above it;
//! 3. **the state machine is enforced by the database.** A paid invoice
//!    cannot be voided, a voided one cannot be paid, a draft cannot be voided
//!    and an invoice being paid cannot be voided or written off;
//! 4. `invoice.created`, `invoice.finalized` and `invoice.voided` are written
//!    **in the same transaction** as the transition they describe, and a
//!    refused transition writes none;
//! 5. a draft's totals always equal its lines, through add, change and
//!    remove;
//! 6. every route answers another merchant's `in_…` and an id that never
//!    existed with the **byte for byte** identical `404`;
//! 7. an `Idempotency-Key` replays the stored response, and the same key with
//!    a different body is the `400` `idempotency_key_in_use` envelope —
//!    **not** a `422`, which is what this line said until the S4b review on
//!    2026-09-07 and what the case below has never asserted
//!    (`docs/api/README.md`'s idempotency table is the contract).
//!
//! `invoice.paid` is **not** here. It is emitted by the settlement
//! transaction, which needs a rail: `backends/crates/vpay-db/tests/repositories.rs`
//! proves the write and the event are one transaction (and that abandoning it
//! writes neither), and `worker_e2e.rs` drives the loop. See
//! `docs/flows/invoices.md`'s Status section, which says so plainly rather
//! than implying this file covers it.
//!
//! # No test doubles
//!
//! Real Postgres, the shipping router. The rail is configured and
//! **unreachable**, exactly as `customers.rs` configures it: nothing here
//! confirms against a rail, and an intent only has to exist.

// See `tests/support/mod.rs` for why this allow list mirrors the other
// integration suites'.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::Context as _;
use serde_json::Value;
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_config::{
    CheckoutConfig, Config, CurrencyEntry, Deployment, HostEntry, MERCHANT_AUDIENCE, ProviderHost,
};
use vpay_db::Repositories;

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client,
    merchant_client_with_invoice_urls, merchant_client_with_publishable_keys, migrated_postgres,
    serve,
};

/// The merchant every test acts as, and the tenant it acts for. Never the
/// same string: a query filtered by `client_id` instead of `merchant_id`
/// would otherwise pass.
const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";

/// The second merchant, which exists so the tenancy cases have someone else's
/// invoice to fail to read.
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

/// The third merchant: the one that has configured
/// `merchant_clients[].invoices` (issue #91, D2), so
/// `POST /v1/invoices/{id}/pay` may omit both URLs.
///
/// A third registration rather than defaults on `CLIENT_A`, deliberately.
/// Merchant A is what every other case in this file pays with, and giving it
/// defaults would mean the refusal — "neither configured nor passed is a
/// `400` naming both" — had no merchant left to be proved against. The two
/// halves of D2 need two registrations or one of them is untested.
const CLIENT_C: &str = "gamma-yaounde";
const MERCHANT_C: &str = "gamma-yaounde-tenant";

const RAIL: &str = "mtn_momo";
const CANONICAL_PHONE: &str = "237600000200";

/// An id of exactly the shape `vpay_core::ids::invoice_id` mints, that no
/// merchant has ever had.
const MISSING_INVOICE_ID: &str = "in_00000000000000000000000x";

/// Merchant A's publishable key — `POST /v1/invoices/{id}/pay` creates a
/// hosted checkout session, and a tenant with no registered key cannot.
const PK_A: &str = "pk_test_acmecameroonsandbox01";
const CHECKOUT_BASE: &str = "https://checkout.vpay.test";

/// Where `POST /v1/invoices/{id}/pay` forwards the payer. Required, because
/// paying an invoice creates a **hosted** checkout session and migration
/// `0028`'s `urls_match_ui_mode` demands both — see that handler for why vpay
/// will not invent them.
const SUCCESS_URL: &str = "https://shop.acme.example/invoice-paid";
const CANCEL_URL: &str = "https://shop.acme.example/invoice-cancelled";

/// Merchant C's publishable key. Distinct from [`PK_A`] because
/// `Config::validate_all` refuses a key claimed by two merchants.
const PK_C: &str = "pk_test_gammayaoundesandbox02";

/// What merchant C configured in `merchant_clients[].invoices`.
///
/// Deliberately different strings from [`SUCCESS_URL`] and [`CANCEL_URL`]:
/// the case that proves a *passed* URL wins sends those two and asserts the
/// session carries them and not these, which a shared constant would make
/// impossible to tell apart.
const CONFIGURED_SUCCESS_URL: &str = "https://shop.gamma.example/thank-you";
const CONFIGURED_CANCEL_URL: &str = "https://shop.gamma.example/cart";

// ------------------------------------------------------------------ harness

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    server: tokio::task::JoinHandle<()>,
    /// The repository seam, for the one thing a merchant cannot do over
    /// HTTP: read the retention sweep's backlog. `vpay_worker` reaches it the
    /// same way.
    repositories: Arc<dyn Repositories>,
    /// The plain `sqlx` pool: several cases read `events` and
    /// `invoice_number_sequences` back directly, because "the event is in the
    /// same transaction" and "the sequence was not advanced" are claims about
    /// rows rather than about responses.
    pool: PgPool,
    base_url: String,
    signing_key: LoadedSigningKey,
    http: reqwest::Client,
}

impl Harness {
    /// A bearer token for `client_id`.
    ///
    /// Raw HTTP rather than the merchant SDK, unlike `customers.rs`: the SDK
    /// has no invoice methods yet (`docs/sdks/parity.md` carries the dated
    /// gap row), and writing this suite against a client that does not exist
    /// would be the "test asserts the implementation back to itself" failure
    /// this repository names in `CLAUDE.md`.
    fn bearer(&self, client_id: &str) -> String {
        self.signing_key
            .token_manager()
            .issue_client_token_with_extra(
                client_id,
                900,
                Some(vpay_api::SCOPE_PAYMENTS_WRITE.to_owned()),
                Some(MERCHANT_AUDIENCE.to_owned()),
                HashMap::new(),
            )
            .expect("the server's own signer mints a merchant token")
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// A `POST` with a form body and a fresh idempotency key.
    async fn post(
        &self,
        client_id: &str,
        path: &str,
        form: &[(&str, &str)],
    ) -> anyhow::Result<(reqwest::StatusCode, Value)> {
        self.post_with_key(client_id, path, form, &fresh_key())
            .await
    }

    /// A `POST` with a caller-chosen idempotency key, for the replay cases.
    async fn post_with_key(
        &self,
        client_id: &str,
        path: &str,
        form: &[(&str, &str)],
        key: &str,
    ) -> anyhow::Result<(reqwest::StatusCode, Value)> {
        let response = self
            .http
            .post(self.url(path))
            .bearer_auth(self.bearer(client_id))
            .header("Idempotency-Key", key)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form_body(form))
            .send()
            .await
            .context("posting to the invoice surface")?;
        let status = response.status();
        Ok((status, response.json().await.unwrap_or(Value::Null)))
    }

    async fn get(
        &self,
        client_id: &str,
        path: &str,
    ) -> anyhow::Result<(reqwest::StatusCode, Value)> {
        let response = self
            .http
            .get(self.url(path))
            .bearer_auth(self.bearer(client_id))
            .send()
            .await
            .context("reading the invoice surface")?;
        let status = response.status();
        Ok((status, response.json().await.unwrap_or(Value::Null)))
    }

    /// A `GET` whose **body bytes** are returned unparsed, for the tenancy
    /// cases: "the same 404" is a claim about bytes, and parsing then
    /// re-serialising would hide a difference in key order or in a message.
    async fn get_bytes(&self, client_id: &str, path: &str) -> anyhow::Result<(u16, String)> {
        let response = self
            .http
            .get(self.url(path))
            .bearer_auth(self.bearer(client_id))
            .send()
            .await
            .context("reading the invoice surface")?;
        let status = response.status().as_u16();
        Ok((status, response.text().await.unwrap_or_default()))
    }

    async fn delete(
        &self,
        client_id: &str,
        path: &str,
    ) -> anyhow::Result<(reqwest::StatusCode, Value)> {
        let response = self
            .http
            .delete(self.url(path))
            .bearer_auth(self.bearer(client_id))
            .header("Idempotency-Key", fresh_key())
            .send()
            .await
            .context("deleting on the invoice surface")?;
        let status = response.status();
        Ok((status, response.json().await.unwrap_or(Value::Null)))
    }

    /// Every event of one type, oldest first — the proof that a transition
    /// wrote one and that a refused transition wrote none.
    async fn events_of(&self, event_type: &str) -> anyhow::Result<Vec<(String, Value)>> {
        let rows: Vec<(String, Value)> =
            sqlx::query_as("SELECT object_id, data FROM events WHERE type = $1 ORDER BY seq")
                .bind(event_type)
                .fetch_all(&self.pool)
                .await
                .context("reading the events this transition should have written")?;
        Ok(rows)
    }

    /// The number this merchant's next finalize would take, or `None` if they
    /// have never finalized anything.
    ///
    /// Read from `invoice_number_sequences` rather than inferred from the
    /// last invoice, because the two are exactly what a burnt number would
    /// make disagree.
    async fn next_number(&self, merchant_id: &str) -> anyhow::Result<Option<i64>> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT next_number FROM invoice_number_sequences WHERE merchant_id = $1",
        )
        .bind(merchant_id)
        .fetch_optional(&self.pool)
        .await
        .context("reading the merchant's invoice number sequence")?;
        Ok(row.map(|(next,)| next))
    }

    /// A customer to bill — phone-only, which is the S4a maintainer decision
    /// and the shape a Cameroonian merchant actually has.
    async fn customer(&self, client_id: &str) -> anyhow::Result<String> {
        let (status, body) = self
            .post(client_id, "/v1/customers", &[("phone", CANONICAL_PHONE)])
            .await?;
        anyhow::ensure!(status == 201, "creating a customer: {status} {body}");
        Ok(field(&body, "id")
            .as_str()
            .expect("a customer has an id")
            .to_owned())
    }

    /// A draft invoice with one 5,000 FCFA line — the fixture almost every
    /// case starts from.
    async fn draft_with_a_line(&self, client_id: &str) -> anyhow::Result<String> {
        let customer = self.customer(client_id).await?;
        let (status, body) = self
            .post(
                client_id,
                "/v1/invoices",
                &[("customer", &customer), ("currency", "xaf")],
            )
            .await?;
        anyhow::ensure!(status == 201, "creating an invoice: {status} {body}");
        let id = field(&body, "id")
            .as_str()
            .expect("an invoice has an id")
            .to_owned();

        let (status, body) = self
            .post(
                client_id,
                "/v1/invoice_items",
                &[
                    ("invoice", &id),
                    ("description", "One month of hosting"),
                    ("unit_amount", "5000"),
                ],
            )
            .await?;
        anyhow::ensure!(status == 201, "adding a line: {status} {body}");
        Ok(id)
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

/// A fresh `Idempotency-Key`. A UUID rather than a counter so two cases
/// running in the same process cannot collide on one.
fn fresh_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// One field of a JSON body, or `Null`.
///
/// Not `field(&body, "field")`: `clippy::indexing_slicing` is denied in these suites,
/// and `serde_json::Value`'s `Index` impl panics on a type mismatch — so a
/// handler that answered an array where a case expects an object would fail
/// as a panic rather than as an assertion naming the field.
/// `staff_sign_in.rs` and `dashboard_read_surface.rs` carry the same helper.
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

/// One field of a nested object — `at(&body, &["error", "param"])`.
///
/// A path rather than chained [`field`] calls, because this suite reads two-
/// and three-deep paths often enough (`lines.data[0].id`,
/// `status_transitions.finalized_at`) that the chained form is what a reader
/// has to parse rather than read.
fn at<'a>(value: &'a Value, path: &[&str]) -> &'a Value {
    path.iter().fold(value, |current, key| field(current, key))
}

/// The `n`th element of a JSON array, or `Null` — [`field`]'s reason applied
/// to a list.
fn nth(value: &Value, index: usize) -> &Value {
    value
        .as_array()
        .and_then(|items| items.get(index))
        .unwrap_or(&Value::Null)
}

/// The only element of a slice, asserting there is exactly one.
///
/// `clippy::indexing_slicing` is denied in these suites, and `rows[0]` after
/// an `assert_eq!(rows.len(), 1)` is two statements saying one thing. This
/// says it once, and its message names what was expected — which matters most
/// on the event assertions, where "a refused transition wrote no event" and
/// "the transition wrote two" are different bugs with the same symptom.
fn only<'a, T>(items: &'a [T], what: &str) -> &'a T {
    assert_eq!(
        items.len(),
        1,
        "expected exactly one {what}, got {}",
        items.len()
    );
    items
        .first()
        .unwrap_or_else(|| panic!("checked non-empty above"))
}

/// The `n`th element of a slice, or a panic naming what was missing.
fn item<'a, T>(items: &'a [T], index: usize, what: &str) -> &'a T {
    items.get(index).unwrap_or_else(|| {
        panic!(
            "expected at least {} {what}, got {}",
            index + 1,
            items.len()
        )
    })
}

/// Encodes a form body with **literal brackets in the keys**.
///
/// `reqwest`'s own `.form()` percent-escapes `[` and `]`, and
/// `vpay_api::form::parse_key` splits a key on its brackets *before* any
/// decoding (deliberately — see that module's header, which explains why a
/// decoded `[` must never become structure). So `.form()` sends
/// `metadata%5Border_id%5D`, which arrives as one flat key called
/// `metadata[order_id]` and is silently ignored.
///
/// That is a real property of this wire and not a quirk of the test: both
/// merchant SDKs and Stripe's own clients send literal brackets. Encoding
/// them here is what makes this suite drive the same bytes a merchant does —
/// and the first version of this file used `.form()` and had a
/// `metadata[order_id]` assertion that failed for exactly this reason, which
/// is why the comment is this long.
fn form_body(pairs: &[(&str, &str)]) -> String {
    fn encode_value(value: &str) -> String {
        value
            .bytes()
            .map(|byte| match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    char::from(byte).to_string()
                }
                // `%20`, not `+`. `vpay_api::form` percent-decodes and
                // **only** percent-decodes — a `+` is a literal plus, which is
                // the rule that keeps an MSISDN intact (see that module, and
                // its `a_plus_is_a_plus_and_a_space_is_percent_twenty`). This
                // helper encoded a space as `+` until the S4b review on
                // 2026-09-07, so every description this suite sent was stored
                // as `One+month+of+hosting`. No assertion read one back, which
                // is why nothing said so.
                b' ' => "%20".to_owned(),
                other => format!("%{other:02X}"),
            })
            .collect()
    }

    pairs
        .iter()
        .map(|(key, value)| format!("{key}={}", encode_value(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Three merchants, one rail, one currency, `livemode: false`.
///
/// A and B are the tenancy pair. C is the one with
/// `merchant_clients[].invoices` configured — see [`CLIENT_C`] for why it is
/// a third registration and not a field on A.
fn config_with(base_url: &str, jwks_a: Value, jwks_b: Value, jwks_c: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "invoices".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
            surfaces: None,
        },
        providers: vec![ProviderHost {
            code: RAIL.to_owned(),
            enabled: true,
            host: HostEntry {
                url: "http://127.0.0.1:1".to_owned(),
                label: "unreachable-by-design".to_owned(),
            },
            settings: BTreeMap::from([
                ("target_environment".to_owned(), "sandbox".to_owned()),
                (
                    "api_user".to_owned(),
                    "11111111-2222-3333-4444-555555555555".to_owned(),
                ),
            ]),
            callback_url: None,
            currency: "XAF".to_owned(),
            credentials: BTreeMap::from([
                (
                    "subscription_key".to_owned(),
                    "stub-subscription-key".to_owned(),
                ),
                ("api_key".to_owned(), "stub-api-key".to_owned()),
            ]),
            display_name: None,
        }],
        currencies: vec![CurrencyEntry {
            code: "XAF".to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![
            merchant_client_with_publishable_keys(CLIENT_A, MERCHANT_A, jwks_a, &[PK_A]),
            merchant_client(CLIENT_B, MERCHANT_B, jwks_b),
            merchant_client_with_invoice_urls(
                CLIENT_C,
                MERCHANT_C,
                jwks_c,
                &[PK_C],
                CONFIGURED_SUCCESS_URL,
                CONFIGURED_CANCEL_URL,
            ),
        ],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: CheckoutConfig {
            public_base_url: Some(CHECKOUT_BASE.to_owned()),
        },
        dashboard_client: None,
        staff_auth: vpay_config::StaffAuth::default(),
    }
}

async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (container, repositories, pool) = migrated_postgres().await?;

    let (server_pem, _server_jwks) = generate_key();
    let (_pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();
    let (_pem_c, jwks_c) = generate_key();

    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, jwks_a, jwks_b, jwks_c)
    })
    .await?;

    Ok(Harness {
        _container: container,
        server: served.server,
        repositories,
        pool,
        base_url: served.base_url,
        signing_key: served.signing_key,
        http: reqwest::Client::builder()
            .build()
            .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed"),
    })
}

// ------------------------------------------------------- objects and lines

/// A draft is created, listed, retrieved, and its total is its lines.
///
/// The `object` discriminators are asserted explicitly because a
/// Stripe-shaped client switches on them, and because `invoice_items` and
/// `line_item` are deliberately *different* words for one row — see
/// `vpay_api::model::LineItemTag`.
#[tokio::test]
async fn a_draft_is_created_listed_and_retrieved_with_its_lines() -> anyhow::Result<()> {
    let harness = harness().await?;
    let customer = harness.customer(CLIENT_A).await?;

    let (status, invoice) = harness
        .post(
            CLIENT_A,
            "/v1/invoices",
            &[
                ("customer", &customer),
                ("currency", "xaf"),
                ("description", "September hosting"),
                ("metadata[order_id]", "1234"),
            ],
        )
        .await?;
    assert_eq!(status, 201, "{invoice}");
    let id = field(&invoice, "id").as_str().expect("an id").to_owned();

    assert_eq!(field(&invoice, "object"), "invoice");
    assert_eq!(field(&invoice, "status"), "draft");
    assert_eq!(field(&invoice, "customer"), customer.as_str());
    assert_eq!(field(&invoice, "currency"), "xaf");
    assert_eq!(
        field(&invoice, "number"),
        &Value::Null,
        "a draft has no number"
    );
    assert_eq!(field(&invoice, "amount_due"), 0);
    assert_eq!(field(&invoice, "amount_remaining"), 0);
    assert_eq!(
        field(&invoice, "description"),
        "September hosting",
        "a space in a merchant's description survives the wire intact"
    );
    assert_eq!(at(&invoice, &["metadata", "order_id"]), "1234");
    assert_eq!(at(&invoice, &["lines", "object"]), "list");
    assert_eq!(
        at(&invoice, &["lines", "data"])
            .as_array()
            .expect("a list")
            .len(),
        0
    );
    assert_eq!(field(&invoice, "payment_intent"), &Value::Null);
    assert_eq!(field(&invoice, "hosted_invoice_url"), &Value::Null);
    assert!(
        field(&invoice, "id")
            .as_str()
            .expect("an id")
            .starts_with("in_")
    );

    // `invoice.created` is in the same transaction as the insert. Asserted by
    // reading the row rather than by trusting the response, because the whole
    // claim is about what was committed.
    let created = harness.events_of("invoice.created").await?;
    let event = only(&created, "created event");
    assert_eq!(event.0, id);
    assert_eq!(field(&event.1, "object"), "invoice");
    assert_eq!(field(&event.1, "status"), "draft");

    // Two lines, and the total follows both.
    for (description, quantity, unit_amount) in
        [("Hosting", "1", "5000"), ("Support hours", "3", "2000")]
    {
        let (status, line) = harness
            .post(
                CLIENT_A,
                "/v1/invoice_items",
                &[
                    ("invoice", &id),
                    ("description", description),
                    ("quantity", quantity),
                    ("unit_amount", unit_amount),
                ],
            )
            .await?;
        assert_eq!(status, 201, "{line}");
        assert_eq!(field(&line, "object"), "line_item");
        assert!(
            field(&line, "id")
                .as_str()
                .expect("an id")
                .starts_with("ii_")
        );
        assert_eq!(
            field(&line, "currency"),
            "xaf",
            "a line inherits its invoice's currency"
        );
    }

    let (status, invoice) = harness.get(CLIENT_A, &format!("/v1/invoices/{id}")).await?;
    assert_eq!(status, 200, "{invoice}");
    assert_eq!(field(&invoice, "amount_due"), 11_000, "5000 + 3 x 2000");
    assert_eq!(field(&invoice, "amount_remaining"), 11_000);
    assert_eq!(field(&invoice, "amount_paid"), 0);
    let lines = at(&invoice, &["lines", "data"]).as_array().expect("a list");
    assert_eq!(lines.len(), 2);
    assert_eq!(field(item(lines, 0, "line"), "description"), "Hosting");
    assert_eq!(
        field(item(lines, 1, "line"), "amount"),
        6000,
        "3 x 2000, computed by the database"
    );
    assert_eq!(
        at(&invoice, &["lines", "has_more"]),
        false,
        "an invoice's lines are never paged"
    );

    let (status, page) = harness.get(CLIENT_A, "/v1/invoices").await?;
    assert_eq!(status, 200, "{page}");
    assert_eq!(field(&page, "object"), "list");
    assert_eq!(field(&page, "url"), "/v1/invoices");
    let data = field(&page, "data").as_array().expect("a list");
    assert_eq!(data.len(), 1);
    assert_eq!(field(item(data, 0, "invoice"), "id"), id.as_str());

    // Both filters, in the same `WHERE` as `merchant_id`.
    let (_, page) = harness.get(CLIENT_A, "/v1/invoices?status=draft").await?;
    assert_eq!(field(&page, "data").as_array().expect("a list").len(), 1);
    let (_, page) = harness.get(CLIENT_A, "/v1/invoices?status=open").await?;
    assert_eq!(field(&page, "data").as_array().expect("a list").len(), 0);
    let (_, page) = harness
        .get(CLIENT_A, &format!("/v1/invoices?customer={customer}"))
        .await?;
    assert_eq!(field(&page, "data").as_array().expect("a list").len(), 1);

    // An unknown status is a `400` naming the parameter, and NOT an empty
    // page — which would read as "you have no invoices like that".
    let (status, body) = harness.get(CLIENT_A, "/v1/invoices?status=sent").await?;
    assert_eq!(status, 400, "{body}");
    assert_eq!(at(&body, &["error", "param"]), "status");

    harness.shutdown().await;
    Ok(())
}

/// A line's `amount` follows whichever of its two factors changed, and the
/// invoice's total follows the line — through add, change and remove.
///
/// The two-factor patch is the case that matters: an `UPDATE`'s `SET` list
/// sees the row as it *was*, so a naive `amount = quantity * unit_amount`
/// would multiply the old values and trip `amount_is_the_product`.
#[tokio::test]
async fn a_line_amount_follows_its_own_factors_and_the_invoice_follows_the_lines()
-> anyhow::Result<()> {
    let harness = harness().await?;
    let invoice = harness.draft_with_a_line(CLIENT_A).await?;

    let (_, page) = harness
        .get(CLIENT_A, &format!("/v1/invoices/{invoice}"))
        .await?;
    let line = at(nth(at(&page, &["lines", "data"]), 0), &["id"])
        .as_str()
        .expect("the line has an id")
        .to_owned();
    assert_eq!(field(&page, "amount_due"), 5000);

    // Both factors at once.
    let (status, updated) = harness
        .post(
            CLIENT_A,
            &format!("/v1/invoice_items/{line}"),
            &[("quantity", "4"), ("unit_amount", "250")],
        )
        .await?;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(field(&updated, "amount"), 1000, "4 x 250, not 1 x 5000");

    let (_, page) = harness
        .get(CLIENT_A, &format!("/v1/invoices/{invoice}"))
        .await?;
    assert_eq!(field(&page, "amount_due"), 1000);
    assert_eq!(field(&page, "amount_remaining"), 1000);

    // One factor only.
    let (_, updated) = harness
        .post(
            CLIENT_A,
            &format!("/v1/invoice_items/{line}"),
            &[("quantity", "2")],
        )
        .await?;
    assert_eq!(
        field(&updated, "amount"),
        500,
        "2 x 250 — unit_amount was left alone"
    );

    // Removed, and the total goes back to zero.
    let (status, deleted) = harness
        .delete(CLIENT_A, &format!("/v1/invoice_items/{line}"))
        .await?;
    assert_eq!(status, 200, "{deleted}");
    assert_eq!(field(&deleted, "deleted"), true);
    assert_eq!(field(&deleted, "object"), "line_item");

    let (_, page) = harness
        .get(CLIENT_A, &format!("/v1/invoices/{invoice}"))
        .await?;
    assert_eq!(field(&page, "amount_due"), 0);
    assert_eq!(
        at(&page, &["lines", "data"])
            .as_array()
            .expect("a list")
            .len(),
        0
    );

    harness.shutdown().await;
    Ok(())
}

// -------------------------------------------------------------- transitions

/// Finalize issues the document, and everything that changes a draft is
/// refused afterwards.
///
/// This is claim (2), and each refusal is a `409` naming the status rather
/// than a `404`: the invoice exists and the merchant can see it, so telling
/// them it does not would send them looking for the wrong problem.
#[tokio::test]
async fn a_finalized_invoice_refuses_every_write_that_changes_a_draft() -> anyhow::Result<()> {
    let harness = harness().await?;
    let invoice = harness.draft_with_a_line(CLIENT_A).await?;
    let (_, draft) = harness
        .get(CLIENT_A, &format!("/v1/invoices/{invoice}"))
        .await?;
    let line = at(nth(at(&draft, &["lines", "data"]), 0), &["id"])
        .as_str()
        .expect("the line has an id")
        .to_owned();

    let (status, open) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{invoice}/finalize"), &[])
        .await?;
    assert_eq!(status, 200, "{open}");
    assert_eq!(field(&open, "status"), "open");
    assert_eq!(
        field(&open, "amount_due"),
        5000,
        "the total is computed once, here"
    );
    assert_eq!(field(&open, "amount_remaining"), 5000);
    let number = field(&open, "number")
        .as_str()
        .expect("a finalized invoice has a number");
    assert!(
        number.ends_with("-000001"),
        "the merchant's first invoice is -000001, got {number}"
    );
    assert_eq!(number.len(), 15, "{{8-char prefix}}-{{6 digits}}: {number}");
    assert!(
        at(&open, &["status_transitions", "finalized_at"]).is_i64(),
        "finalize stamps its own transition"
    );

    // `invoice.finalized`, in the same transaction.
    let finalized = harness.events_of("invoice.finalized").await?;
    let event = only(&finalized, "finalized event");
    assert_eq!(event.0, invoice);
    assert_eq!(field(&event.1, "number"), number);

    // Every draft-only write, refused.
    let (status, body) = harness
        .post(
            CLIENT_A,
            &format!("/v1/invoices/{invoice}"),
            &[("description", "changed my mind")],
        )
        .await?;
    assert_eq!(status, 409, "PATCH after finalize: {body}");
    assert!(
        at(&body, &["error", "message"])
            .as_str()
            .expect("a message")
            .contains("`open`"),
        "the refusal names the status it found: {body}"
    );

    let (status, body) = harness
        .delete(CLIENT_A, &format!("/v1/invoices/{invoice}"))
        .await?;
    assert_eq!(status, 409, "DELETE after finalize: {body}");

    let (status, body) = harness
        .post(
            CLIENT_A,
            &format!("/v1/invoice_items/{line}"),
            &[("unit_amount", "1")],
        )
        .await?;
    assert_eq!(status, 409, "changing a frozen line: {body}");

    let (status, body) = harness
        .delete(CLIENT_A, &format!("/v1/invoice_items/{line}"))
        .await?;
    assert_eq!(status, 409, "removing a frozen line: {body}");

    let (status, body) = harness
        .post(
            CLIENT_A,
            "/v1/invoice_items",
            &[
                ("invoice", &invoice),
                ("description", "one more thing"),
                ("unit_amount", "100"),
            ],
        )
        .await?;
    assert_eq!(status, 400, "adding to a frozen invoice: {body}");
    assert_eq!(at(&body, &["error", "param"]), "invoice");

    // A second finalize is refused, and writes no second event.
    let (status, body) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{invoice}/finalize"), &[])
        .await?;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        harness.events_of("invoice.finalized").await?.len(),
        1,
        "a refused transition writes no event"
    );

    // The document itself is unchanged.
    let (_, after) = harness
        .get(CLIENT_A, &format!("/v1/invoices/{invoice}"))
        .await?;
    assert_eq!(field(&after, "amount_due"), 5000);
    assert_eq!(field(&after, "number"), number);
    assert_eq!(
        at(&after, &["lines", "data"])
            .as_array()
            .expect("a list")
            .len(),
        1
    );

    harness.shutdown().await;
    Ok(())
}

/// Two finalizes racing each other take two consecutive numbers — no gap, no
/// reuse.
///
/// This is claim (1), and it is the case the whole `invoice_number_sequences`
/// design exists for. **The decisive mutation:** delete the `ON CONFLICT
/// (merchant_id) DO UPDATE` clause from `vpay_db::invoices`'
/// `next_number_in_tx` — the two transactions stop serialising on the
/// sequence row, and this fails on the duplicate.
///
/// Both requests are in flight at once through `tokio::join!` on two
/// independent connections, so the serialisation is Postgres' row lock and
/// not this test's ordering.
#[tokio::test]
async fn two_concurrent_finalizes_take_consecutive_numbers() -> anyhow::Result<()> {
    let harness = harness().await?;
    let first = harness.draft_with_a_line(CLIENT_A).await?;
    let second = harness.draft_with_a_line(CLIENT_A).await?;

    // The paths are built before the join so neither `format!` temporary is
    // borrowed across the await point.
    let first_path = format!("/v1/invoices/{first}/finalize");
    let second_path = format!("/v1/invoices/{second}/finalize");
    let (left, right) = tokio::join!(
        harness.post(CLIENT_A, &first_path, &[]),
        harness.post(CLIENT_A, &second_path, &[]),
    );
    let (left_status, left_body) = left?;
    let (right_status, right_body) = right?;
    assert_eq!(left_status, 200, "{left_body}");
    assert_eq!(right_status, 200, "{right_body}");

    let mut numbers = [
        field(&left_body, "number")
            .as_str()
            .expect("a number")
            .to_owned(),
        field(&right_body, "number")
            .as_str()
            .expect("a number")
            .to_owned(),
    ];
    numbers.sort();

    let (first_number, second_number) = (item(&numbers, 0, "number"), item(&numbers, 1, "number"));
    assert_ne!(first_number, second_number, "two invoices, two numbers");

    let (prefix_a, seq_a) = first_number.split_once('-').expect("prefix-sequence");
    let (prefix_b, seq_b) = second_number.split_once('-').expect("prefix-sequence");
    assert_eq!(
        prefix_a, prefix_b,
        "one merchant, one prefix — it is minted once and stored"
    );
    assert_eq!(seq_a, "000001");
    assert_eq!(seq_b, "000002", "consecutive, with no gap: {numbers:?}");

    assert_eq!(
        harness.next_number(MERCHANT_A).await?,
        Some(3),
        "the sequence advanced exactly twice"
    );

    // A third merchant's sequence is their own, and starts at 1 again. Two
    // merchants sharing a counter would leak how many invoices the other has
    // issued.
    let other = harness.draft_with_a_line(CLIENT_B).await?;
    let (status, body) = harness
        .post(CLIENT_B, &format!("/v1/invoices/{other}/finalize"), &[])
        .await?;
    assert_eq!(status, 200, "{body}");
    assert!(
        field(&body, "number")
            .as_str()
            .expect("a number")
            .ends_with("-000001"),
        "a second merchant's first invoice is their own -000001: {body}"
    );

    harness.shutdown().await;
    Ok(())
}

/// A refused finalize does not burn a number.
///
/// This is the property a Postgres `SEQUENCE` would **not** have — `nextval`
/// is non-transactional — and it is the reason migration `0036` uses an
/// ordinary table row instead. Stripe's numbering does have holes; a
/// Cameroonian merchant's cannot, because a missing number is read by a tax
/// authority as a destroyed document.
///
/// **The decisive mutation:** move `next_number_in_tx` onto a Postgres
/// sequence, or take the number outside the transaction — the second
/// assertion below becomes `-000002`.
#[tokio::test]
async fn a_refused_finalize_does_not_burn_a_number() -> anyhow::Result<()> {
    let harness = harness().await?;

    // Merchant B finalizing merchant A's invoice: a `404`, and it must not
    // touch A's sequence *or* create one for B.
    let invoice = harness.draft_with_a_line(CLIENT_A).await?;
    let (status, body) = harness
        .post(CLIENT_B, &format!("/v1/invoices/{invoice}/finalize"), &[])
        .await?;
    assert_eq!(status, 404, "{body}");

    // The same merchant finalizing an invoice with no lines: a `400`, refused
    // before the sequence is reached at all.
    let customer = harness.customer(CLIENT_A).await?;
    let (_, empty) = harness
        .post(
            CLIENT_A,
            "/v1/invoices",
            &[("customer", &customer), ("currency", "xaf")],
        )
        .await?;
    let empty_id = field(&empty, "id").as_str().expect("an id");
    let (status, body) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{empty_id}/finalize"), &[])
        .await?;
    assert_eq!(status, 400, "a lineless invoice is refused: {body}");
    assert_eq!(at(&body, &["error", "param"]), "invoice");

    assert_eq!(
        harness.next_number(MERCHANT_A).await?,
        None,
        "no refused finalize created a sequence row"
    );
    assert_eq!(harness.next_number(MERCHANT_B).await?, None);

    // The real one now takes -000001, not -000002 or -000003.
    let (status, body) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{invoice}/finalize"), &[])
        .await?;
    assert_eq!(status, 200, "{body}");
    assert!(
        field(&body, "number")
            .as_str()
            .expect("a number")
            .ends_with("-000001"),
        "two refusals burnt a number: {body}"
    );

    harness.shutdown().await;
    Ok(())
}

/// `void` and `mark_uncollectible`, and every transition each one refuses.
///
/// This is claim (3). The refusals are what the state machine *is*: with the
/// `AND status = '<from>'` clauses deleted, every one of these assertions
/// becomes a `200`.
#[tokio::test]
async fn the_two_terminal_transitions_and_the_transitions_they_refuse() -> anyhow::Result<()> {
    let harness = harness().await?;

    // A draft cannot be voided — it is deleted instead. Migration `0036`'s
    // `number_is_assigned_at_finalize` is what makes this unavoidable rather
    // than a policy: a voided draft would be a non-draft row with no number.
    let draft = harness.draft_with_a_line(CLIENT_A).await?;
    let (status, body) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{draft}/void"), &[])
        .await?;
    assert_eq!(status, 409, "a draft is deleted, not voided: {body}");
    assert!(
        at(&body, &["error", "message"])
            .as_str()
            .expect("a message")
            .contains("`draft`")
    );
    assert_eq!(harness.events_of("invoice.voided").await?.len(), 0);

    // A draft cannot be written off either.
    let (status, body) = harness
        .post(
            CLIENT_A,
            &format!("/v1/invoices/{draft}/mark_uncollectible"),
            &[],
        )
        .await?;
    assert_eq!(status, 409, "{body}");

    // Finalize, then void. The number is kept.
    let (_, open) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{draft}/finalize"), &[])
        .await?;
    let number = field(&open, "number")
        .as_str()
        .expect("a number")
        .to_owned();

    let (status, voided) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{draft}/void"), &[])
        .await?;
    assert_eq!(status, 200, "{voided}");
    assert_eq!(field(&voided, "status"), "void");
    assert_eq!(
        field(&voided, "number"),
        number.as_str(),
        "a voided invoice keeps its number — a hole in the sequence is a \
         destroyed document"
    );
    assert!(at(&voided, &["status_transitions", "voided_at"]).is_i64());

    let events = harness.events_of("invoice.voided").await?;
    let event = only(&events, "events event");
    assert_eq!(event.0, draft);
    assert_eq!(field(&event.1, "status"), "void");

    // A voided invoice is terminal: nothing moves it again.
    for path in ["void", "mark_uncollectible", "pay", "finalize"] {
        let (status, body) = harness
            .post(
                CLIENT_A,
                &format!("/v1/invoices/{draft}/{path}"),
                // The two URLs go on every one of them so a missing-parameter
                // `400` from `pay` cannot masquerade as the `409` this case is
                // about. The other three ignore them.
                &[("success_url", SUCCESS_URL), ("cancel_url", CANCEL_URL)],
            )
            .await?;
        assert_eq!(status, 409, "{path} on a voided invoice: {body}");
    }
    assert_eq!(
        harness.events_of("invoice.voided").await?.len(),
        1,
        "the refused transitions wrote no second event"
    );

    // The other terminal transition, on a fresh invoice. It emits NO event —
    // `invoice.marked_uncollectible` is deliberately outside migration
    // `0036`'s vocabulary because this transition is a single statement with
    // no writer for one. If that ever changes, this assertion is what says
    // the document and the code disagree.
    let second = harness.draft_with_a_line(CLIENT_A).await?;
    harness
        .post(CLIENT_A, &format!("/v1/invoices/{second}/finalize"), &[])
        .await?;
    let (status, written_off) = harness
        .post(
            CLIENT_A,
            &format!("/v1/invoices/{second}/mark_uncollectible"),
            &[],
        )
        .await?;
    assert_eq!(status, 200, "{written_off}");
    assert_eq!(field(&written_off, "status"), "uncollectible");
    assert!(
        at(
            &written_off,
            &["status_transitions", "marked_uncollectible_at"]
        )
        .is_i64()
    );
    assert_eq!(
        field(&written_off, "amount_remaining"),
        5000,
        "written off is still owed — it is just never expected"
    );

    let (status, body) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{second}/void"), &[])
        .await?;
    assert_eq!(status, 409, "uncollectible is terminal too: {body}");

    // `?status=` sees both terminal states, which is the only way a merchant
    // learns about a write-off (there is no event for it).
    let (_, page) = harness
        .get(CLIENT_A, "/v1/invoices?status=uncollectible")
        .await?;
    let data = field(&page, "data").as_array().expect("a list");
    assert_eq!(data.len(), 1);
    assert_eq!(field(item(data, 0, "invoice"), "id"), second.as_str());

    harness.shutdown().await;
    Ok(())
}

/// An invoice with a live payment intent cannot be voided, written off, or
/// paid a second time — and cancelling the intent is the way back.
///
/// This is the rule `vpay_db::invoices`' `NO_LIVE_INTENT` spells, and it is
/// the one that stops money moving against a document that says nothing is
/// owed. **The decisive mutation:** delete `NO_LIVE_INTENT` from `void_in_tx`
/// and `attach_intent` — the first `void` below becomes a `200`, and so does
/// the second `pay`.
#[tokio::test]
async fn an_invoice_being_paid_refuses_void_write_off_and_a_second_payment() -> anyhow::Result<()> {
    let harness = harness().await?;
    let invoice = harness.draft_with_a_line(CLIENT_A).await?;
    harness
        .post(CLIENT_A, &format!("/v1/invoices/{invoice}/finalize"), &[])
        .await?;

    let (status, paying) = harness
        .post(
            CLIENT_A,
            &format!("/v1/invoices/{invoice}/pay"),
            &[("success_url", SUCCESS_URL), ("cancel_url", CANCEL_URL)],
        )
        .await?;
    assert_eq!(status, 200, "{paying}");
    let intent = field(&paying, "payment_intent")
        .as_str()
        .expect("pay attaches an intent")
        .to_owned();
    assert!(intent.starts_with("pi_"));
    assert_eq!(field(&paying, "status"), "open", "paying does not pay it");
    let hosted = field(&paying, "hosted_invoice_url")
        .as_str()
        .expect("pay produces a hosted URL");
    assert!(
        hosted.starts_with(CHECKOUT_BASE),
        "the hosted URL is the existing checkout page: {hosted}"
    );

    // The intent is for what is left, in the invoice's currency, and for the
    // invoice's customer.
    let (status, pi) = harness
        .get(CLIENT_A, &format!("/v1/payment_intents/{intent}"))
        .await?;
    assert_eq!(status, 200, "{pi}");
    assert_eq!(field(&pi, "amount"), 5000);
    assert_eq!(field(&pi, "currency"), "xaf");
    assert_eq!(field(&pi, "customer"), field(&paying, "customer"));

    for path in ["void", "mark_uncollectible", "pay"] {
        let (status, body) = harness
            .post(
                CLIENT_A,
                &format!("/v1/invoices/{invoice}/{path}"),
                &[("success_url", SUCCESS_URL), ("cancel_url", CANCEL_URL)],
            )
            .await?;
        assert_eq!(status, 409, "{path} while an intent is live: {body}");
        assert!(
            at(&body, &["error", "message"])
                .as_str()
                .expect("a message")
                .contains(&intent),
            "the refusal names the intent to cancel: {body}"
        );
    }
    assert_eq!(harness.events_of("invoice.voided").await?.len(), 0);

    // Cancelling the intent is the documented way back, and it works.
    let (status, body) = harness
        .post(
            CLIENT_A,
            &format!("/v1/payment_intents/{intent}/cancel"),
            &[],
        )
        .await?;
    assert_eq!(status, 200, "{body}");

    let (status, voided) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{invoice}/void"), &[])
        .await?;
    assert_eq!(
        status, 200,
        "a cancelled intent unblocks the void: {voided}"
    );
    assert_eq!(field(&voided, "status"), "void");

    harness.shutdown().await;
    Ok(())
}

// -------------------------------------------------------- tenancy and keys

/// Every route answers another merchant's `in_…` and an id that never existed
/// with the byte-for-byte identical `404`.
///
/// Bytes, not shapes: a message that named the id differently in the two
/// cases would be an existence oracle for another tenant's invoices, and
/// comparing parsed JSON would hide it.
#[tokio::test]
async fn another_merchants_invoice_is_byte_identical_to_one_that_never_existed()
-> anyhow::Result<()> {
    let harness = harness().await?;
    let theirs = harness.draft_with_a_line(CLIENT_A).await?;
    let (_, draft) = harness
        .get(CLIENT_A, &format!("/v1/invoices/{theirs}"))
        .await?;
    let their_line = at(nth(at(&draft, &["lines", "data"]), 0), &["id"])
        .as_str()
        .expect("a line id")
        .to_owned();

    // Read: the two 404s are compared verbatim, with the id substituted out
    // — the *only* thing that may differ is the id the caller sent, which
    // Stripe echoes and so does vpay.
    let (their_status, their_body) = harness
        .get_bytes(CLIENT_B, &format!("/v1/invoices/{theirs}"))
        .await?;
    let (missing_status, missing_body) = harness
        .get_bytes(CLIENT_B, &format!("/v1/invoices/{MISSING_INVOICE_ID}"))
        .await?;
    assert_eq!(their_status, 404);
    assert_eq!(missing_status, 404);
    assert_eq!(
        their_body.replace(&theirs, "<ID>"),
        missing_body.replace(MISSING_INVOICE_ID, "<ID>"),
        "another merchant's invoice must be indistinguishable from a missing one"
    );

    // Every write, too. A `409` here would say "this invoice exists and is
    // not a draft", which is exactly the leak the 404 exists to prevent.
    for (method, path) in [
        ("POST", format!("/v1/invoices/{theirs}")),
        ("POST", format!("/v1/invoices/{theirs}/finalize")),
        ("POST", format!("/v1/invoices/{theirs}/void")),
        ("POST", format!("/v1/invoices/{theirs}/mark_uncollectible")),
        ("POST", format!("/v1/invoices/{theirs}/pay")),
        ("DELETE", format!("/v1/invoices/{theirs}")),
        ("POST", format!("/v1/invoice_items/{their_line}")),
        ("DELETE", format!("/v1/invoice_items/{their_line}")),
        ("GET", format!("/v1/invoice_items/{their_line}")),
    ] {
        let (status, body) = match method {
            "GET" => harness.get(CLIENT_B, &path).await?,
            "DELETE" => harness.delete(CLIENT_B, &path).await?,
            _ => harness.post(CLIENT_B, &path, &[]).await?,
        };
        assert_eq!(status, 404, "{method} {path} leaked: {body}");
    }

    // And naming it as `invoice=` on a line create is a `400` on the
    // parameter, never a 404 and never a different sentence from the one an
    // id that does not exist gets.
    let (their_status, their_body) = harness
        .post(
            CLIENT_B,
            "/v1/invoice_items",
            &[
                ("invoice", &theirs),
                ("description", "x"),
                ("unit_amount", "1"),
            ],
        )
        .await?;
    let (missing_status, missing_body) = harness
        .post(
            CLIENT_B,
            "/v1/invoice_items",
            &[
                ("invoice", MISSING_INVOICE_ID),
                ("description", "x"),
                ("unit_amount", "1"),
            ],
        )
        .await?;
    assert_eq!(their_status, 400);
    assert_eq!(missing_status, 400);
    assert_eq!(
        their_body.to_string().replace(&theirs, "<ID>"),
        missing_body.to_string().replace(MISSING_INVOICE_ID, "<ID>"),
    );

    // Nothing of merchant A's moved.
    let (status, still) = harness
        .get(CLIENT_A, &format!("/v1/invoices/{theirs}"))
        .await?;
    assert_eq!(status, 200, "{still}");
    assert_eq!(field(&still, "status"), "draft");
    assert_eq!(field(&still, "amount_due"), 5000);

    harness.shutdown().await;
    Ok(())
}

/// An `Idempotency-Key` replays the stored response, and the same key with a
/// different body is the `400` `idempotency_key_in_use` envelope.
///
/// The transitions are the interesting half: a replayed `finalize` must give
/// back the *same number* rather than taking a second one, which is the one
/// way a retry could put a hole in a merchant's sequence.
#[tokio::test]
async fn an_idempotency_key_replays_a_create_and_a_finalize() -> anyhow::Result<()> {
    let harness = harness().await?;
    let customer = harness.customer(CLIENT_A).await?;
    let key = fresh_key();

    let body = [("customer", customer.as_str()), ("currency", "xaf")];
    let (first_status, first) = harness
        .post_with_key(CLIENT_A, "/v1/invoices", &body, &key)
        .await?;
    let (replay_status, replay) = harness
        .post_with_key(CLIENT_A, "/v1/invoices", &body, &key)
        .await?;
    assert_eq!(first_status, 201, "{first}");
    assert_eq!(replay_status, 201, "{replay}");
    assert_eq!(first, replay, "a replay answers the stored response");

    // One object, and one event: a replayed create must not write a second
    // `invoice.created`.
    let (_, page) = harness.get(CLIENT_A, "/v1/invoices").await?;
    assert_eq!(field(&page, "data").as_array().expect("a list").len(), 1);
    assert_eq!(harness.events_of("invoice.created").await?.len(), 1);

    // The same key, a different body. **`400`, not `422`** — this API's own
    // documented answer (`docs/api/README.md`'s idempotency table, and
    // `customers.rs`'s `a_reused_key_with_a_different_body_is_the_400_envelope`),
    // which is what a merchant's existing client already branches on. Asserted
    // on the `code` as well as the status, because the status alone would also
    // match an ordinary `invalid_request`.
    let (status, body) = harness
        .post_with_key(
            CLIENT_A,
            "/v1/invoices",
            &[
                ("customer", customer.as_str()),
                ("currency", "xaf"),
                ("description", "different"),
            ],
            &key,
        )
        .await?;
    assert_eq!(status, 400, "{body}");
    assert_eq!(at(&body, &["error", "code"]), "idempotency_key_in_use");
    assert_eq!(at(&body, &["error", "type"]), "idempotency_error");

    // A replayed finalize gives back the same number and does not advance the
    // sequence a second time.
    let invoice = field(&first, "id").as_str().expect("an id").to_owned();
    harness
        .post(
            CLIENT_A,
            "/v1/invoice_items",
            &[
                ("invoice", &invoice),
                ("description", "Hosting"),
                ("unit_amount", "5000"),
            ],
        )
        .await?;

    let finalize_key = fresh_key();
    let (_, first_finalize) = harness
        .post_with_key(
            CLIENT_A,
            &format!("/v1/invoices/{invoice}/finalize"),
            &[],
            &finalize_key,
        )
        .await?;
    let (_, replayed_finalize) = harness
        .post_with_key(
            CLIENT_A,
            &format!("/v1/invoices/{invoice}/finalize"),
            &[],
            &finalize_key,
        )
        .await?;
    assert_eq!(
        field(&first_finalize, "number"),
        field(&replayed_finalize, "number")
    );
    assert_eq!(
        harness.next_number(MERCHANT_A).await?,
        Some(2),
        "a replayed finalize took no second number"
    );
    assert_eq!(harness.events_of("invoice.finalized").await?.len(), 1);

    harness.shutdown().await;
    Ok(())
}

/// The event vocabulary is closed, and it is closed around exactly the four
/// types migration `0036` added.
///
/// **The decisive mutation:** drop one of the four labels from
/// `type_is_a_documented_event` in migration `0036` — the corresponding
/// transition above starts failing with a `23514`, and this case names which
/// label is missing.
///
/// The fifth, `invoice.marked_uncollectible`, is asserted **absent**: it is
/// Stripe's own type and vpay deliberately does not write it, so a future
/// change that adds the label without a writer fails here rather than
/// silently reopening a vocabulary this repository's own rule says must move
/// in lockstep with its code.
#[tokio::test]
async fn the_event_vocabulary_holds_exactly_the_invoice_types_that_have_writers()
-> anyhow::Result<()> {
    let harness = harness().await?;

    for accepted in [
        "invoice.created",
        "invoice.finalized",
        "invoice.paid",
        "invoice.voided",
    ] {
        let written = sqlx::query(
            "INSERT INTO events (id, merchant_id, livemode, type, object_id, data) \
             VALUES ($1, $2, false, $3, 'in_x', '{}'::jsonb)",
        )
        .bind(format!("evt_{}", uuid::Uuid::new_v4().simple()))
        .bind(MERCHANT_A)
        .bind(accepted)
        .execute(&harness.pool)
        .await;
        assert!(
            written.is_ok(),
            "`{accepted}` has a writer in vpay and must be in \
             type_is_a_documented_event: {written:?}"
        );
    }

    for refused in [
        "invoice.marked_uncollectible",
        "invoice.sent",
        "invoice.updated",
    ] {
        let written = sqlx::query(
            "INSERT INTO events (id, merchant_id, livemode, type, object_id, data) \
             VALUES ($1, $2, false, $3, 'in_x', '{}'::jsonb)",
        )
        .bind(format!("evt_{}", uuid::Uuid::new_v4().simple()))
        .bind(MERCHANT_A)
        .bind(refused)
        .execute(&harness.pool)
        .await;
        assert!(
            written.is_err(),
            "`{refused}` has no writer in vpay, so the database must refuse it — a label in a \
             closed vocabulary that nothing produces is what that CHECK exists to prevent"
        );
    }

    harness.shutdown().await;
    Ok(())
}

/// The four multi-column CHECKs that hold the invoice state machine together,
/// asserted directly against a real Postgres.
///
/// They are **invisible to `cratestack migrate baseline` in both
/// directions** — `introspect/postgres/constraints.rs:62` filters
/// `array_length(c.conkey, 1) = 1` — so `postgres_smoke.rs`'s drift count
/// cannot be the guard here, exactly as it cannot be for S4a's
/// `at_least_one_identifier`. Each case below writes the row the constraint
/// exists to refuse, straight past the API and the repository.
///
/// **The decisive mutation:** delete any one of the four CHECKs from
/// migration `0036` and the matching block here goes green — which is the
/// only signal that would.
#[tokio::test]
async fn the_invoice_invariants_are_enforced_by_the_database_itself() -> anyhow::Result<()> {
    let harness = harness().await?;
    let customer = harness.customer(CLIENT_A).await?;
    let invoice = harness.draft_with_a_line(CLIENT_A).await?;

    // `number_is_assigned_at_finalize` — a draft with a number.
    let refused = sqlx::query("UPDATE invoices SET number = 'AAAAAAAA-000001' WHERE id = $1")
        .bind(&invoice)
        .execute(&harness.pool)
        .await;
    assert!(
        refused.is_err(),
        "a draft with a number is a burnt number; number_is_assigned_at_finalize must refuse it"
    );

    // …and an issued invoice without one.
    let refused = sqlx::query("UPDATE invoices SET status = 'open' WHERE id = $1")
        .bind(&invoice)
        .execute(&harness.pool)
        .await;
    assert!(
        refused.is_err(),
        "an open invoice with no number is a document nobody can file"
    );

    // `only_a_live_invoice_has_an_intent` — a draft being paid. A real intent
    // has to exist for the foreign key to be satisfiable, or the statement
    // would be refused by the FK and prove nothing about this CHECK.
    let (_, intent) = harness
        .post(
            CLIENT_A,
            "/v1/payment_intents",
            &[
                ("amount", "5000"),
                ("currency", "xaf"),
                ("payment_method_types[]", "mtn_momo"),
            ],
        )
        .await?;
    let intent_id = field(&intent, "id")
        .as_str()
        .expect("an intent id")
        .to_owned();

    let (_, second) = harness
        .post(
            CLIENT_A,
            "/v1/invoices",
            &[("customer", &customer), ("currency", "xaf")],
        )
        .await?;
    let draft = field(&second, "id").as_str().expect("an id").to_owned();
    let refused = sqlx::query("UPDATE invoices SET payment_intent_id = $2 WHERE id = $1")
        .bind(&draft)
        .bind(&intent_id)
        .execute(&harness.pool)
        .await;
    assert!(
        refused.is_err(),
        "nothing may pay a document that has not been issued"
    );

    // `amounts_add_up` — a total that does not equal its parts.
    let refused = sqlx::query("UPDATE invoices SET amount_paid = 1 WHERE id = $1")
        .bind(&invoice)
        .execute(&harness.pool)
        .await;
    assert!(
        refused.is_err(),
        "amount_paid + amount_remaining must equal amount_due"
    );

    // `paid_means_nothing_remaining` — the one that stops a partial payment
    // being recorded as a settled bill.
    harness
        .post(CLIENT_A, &format!("/v1/invoices/{invoice}/finalize"), &[])
        .await?;
    // Since migration `0049`, a `paid` row must also say HOW it was paid
    // (`paid_names_how`: an intent, or `paid_out_of_band`). The two `paid`
    // rows below were written with neither until then, so on their own they
    // would now be refused for the WRONG reason — the refusal below would
    // pass while proving nothing about `paid_means_nothing_remaining`, and
    // the storable row after it would fail. The invoice therefore names the
    // intent this case created above, which is the shape the settlement
    // produces, and each refusal is pinned to the constraint that must
    // produce it.
    sqlx::query("UPDATE invoices SET payment_intent_id = $2 WHERE id = $1")
        .bind(&invoice)
        .bind(&intent_id)
        .execute(&harness.pool)
        .await
        .context("an open invoice may name an intent")?;
    let refused = sqlx::query(
        "UPDATE invoices SET status = 'paid', amount_paid = 1, amount_remaining = 4999 \
         WHERE id = $1",
    )
    .bind(&invoice)
    .execute(&harness.pool)
    .await;
    assert_eq!(
        refused_by(refused),
        Some("paid_means_nothing_remaining".to_owned()),
        "`paid` means paid in full; partial payments are out of scope and this is where that \
         stops being a sentence in a document"
    );

    // Migration `0042`. A refunded PAID invoice is storable — the whole of
    // D5's shape: `amount_paid` and `amount_remaining` do not move, so
    // neither `paid_means_nothing_remaining` nor `amounts_add_up` is amended
    // and a settled bill never claims the payer owes it again.
    let stored = sqlx::query(
        "UPDATE invoices SET status = 'paid', amount_paid = 5000, amount_remaining = 0, \
                             amount_refunded = 2500, paid_at = now() \
         WHERE id = $1",
    )
    .bind(&invoice)
    .execute(&harness.pool)
    .await;
    assert!(
        stored.is_ok(),
        "a paid invoice with part of it given back must be storable; that is what D5 decided \
         and this is the row it decided about: {stored:?}"
    );

    // …and `refunded_at_most_paid` is what stops it being a way to record
    // money nobody collected. This is the ceiling the refund settlement's
    // `amount_refunded + $n` runs into, which is why that transaction fails
    // closed rather than over-refunding.
    let refused = sqlx::query("UPDATE invoices SET amount_refunded = 5001 WHERE id = $1")
        .bind(&invoice)
        .execute(&harness.pool)
        .await;
    assert_eq!(
        refused_by(refused),
        Some("refunded_at_most_paid".to_owned()),
        "a merchant cannot give back more than was collected"
    );
    let refused = sqlx::query("UPDATE invoices SET amount_refunded = -1 WHERE id = $1")
        .bind(&invoice)
        .execute(&harness.pool)
        .await;
    assert!(
        refused.is_err(),
        "a negative refund is a rebate, which vpay has no concept of"
    );

    // `amount_is_the_product`, on the line.
    let refused = sqlx::query("UPDATE invoice_items SET amount = amount + 1")
        .execute(&harness.pool)
        .await;
    assert!(
        refused.is_err(),
        "a line's amount must equal quantity * unit_amount"
    );

    harness.shutdown().await;
    Ok(())
}

/// An invoiced customer is **anonymised**, never row-deleted — and its
/// invoice survives whole.
///
/// # What this case asserted until 2026-09-10, and why it had to change
///
/// It asserted the opposite: that an invoiced customer never reached the
/// retention sweep at all, and that `DELETE /v1/customers/{id}` answered a
/// `409`. Both were consequences of `invoices.customer_id` being a `NO
/// ACTION` foreign key (migration `0036`) rather than decisions — and
/// together they meant the twelve-month retention promise did not apply to a
/// payer a merchant had ever billed. Migration `0041` separates the two
/// things that were conflated: the invoice is not detached from its payer,
/// **and** the payer is erased.
///
/// # The clause is still load-bearing, and what it now guards is worse
///
/// `vpay_db::customers`' `UNREFERENCED` still names `invoices`, and it is now
/// what makes `erase_in_tx` choose the anonymise branch here. Delete that
/// third `NOT EXISTS` and the erasure takes the **hard-delete** branch, the
/// foreign key raises `23503`, and the whole transaction — event, redactions
/// and all — rolls back: the payer is not erased, the merchant is told
/// nothing, and the `DELETE` answers a `500`. The `200` below is what fails.
#[tokio::test]
async fn an_invoiced_customer_is_anonymised_rather_than_deleted() -> anyhow::Result<()> {
    use vpay_db::Customers;

    let harness = harness().await?;
    let customer = harness.customer(CLIENT_A).await?;
    let (status, invoice) = harness
        .post(
            CLIENT_A,
            "/v1/invoices",
            &[("customer", &customer), ("currency", "xaf")],
        )
        .await?;
    assert_eq!(status, 201, "{invoice}");
    let invoice_id = invoice
        .pointer("/id")
        .and_then(serde_json::Value::as_str)
        .expect("the created invoice's id")
        .to_owned();

    // Age it well past any horizon a sweep would use.
    sqlx::query("UPDATE customers SET last_used_at = now() - interval '400 days' WHERE id = $1")
        .bind(&customer)
        .execute(&harness.pool)
        .await
        .context("ageing the customer")?;

    // It IS offered to the sweep now — the `NOT EXISTS` triple left
    // `idle_since` in migration 0041, because a customer that can be erased
    // must not be exempt from the retention promise for having been billed.
    let backlog = Customers::idle_since(
        harness.repositories.as_ref(),
        time::OffsetDateTime::now_utc(),
        100,
    )
    .await
    .context("reading the sweep's backlog")?;
    assert!(
        backlog.iter().any(|row| row.id == customer),
        "an invoiced customer must reach the sweep: it can be anonymised, and exempting it \
         is what left every billed payer outside the twelve-month promise"
    );

    // And the API erases it rather than refusing. The 409 this case used to
    // assert is gone: its advice — clear name, email and phone — is what
    // `at_least_one_identifier` refuses, so no merchant could follow it.
    let (status, body) = harness
        .delete(CLIENT_A, &format!("/v1/customers/{customer}"))
        .await?;
    assert_eq!(status, 200, "{body}");

    let (name, anonymized): (String, bool) =
        sqlx::query_as("SELECT name, anonymized_at IS NOT NULL FROM customers WHERE id = $1")
            .bind(&customer)
            .fetch_one(&harness.pool)
            .await
            .context("the anonymised customer's row must still exist")?;
    assert!(anonymized);
    assert_eq!(name.as_str(), vpay_db::REDACTED);

    // The invoice is untouched and still names its customer — the whole
    // reason the row could not simply be deleted.
    let (invoice_customer,): (String,) =
        sqlx::query_as("SELECT customer_id FROM invoices WHERE id = $1")
            .bind(&invoice_id)
            .fetch_one(&harness.pool)
            .await
            .context("the invoice after its customer was erased")?;
    assert_eq!(invoice_customer, customer);

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------ review attacks (S4b)

/// Two `POST /v1/invoices/{id}/pay` in flight at once attach **exactly one**
/// intent.
///
/// # Why this case is not the one the suite already had
///
/// `an_invoice_being_paid_refuses_void_write_off_and_a_second_payment` pays
/// twice *in sequence*, so the second request reads the first one's committed
/// intent and `vpay_api::v1::invoices::pay`'s own `refuse_if_being_paid` is
/// what answers. That is the guard a mutation can delete without any wire
/// test noticing — the implementer measured exactly that and added
/// `attaching_a_second_intent_to_an_invoice_is_refused_by_the_statement` at
/// the repository seam for it.
///
/// This case is the third thing neither of those two says: that the whole
/// **handler**, run twice concurrently over a socket, cannot produce an
/// invoice with two intents. Both requests get past the read (they are in
/// flight together), both mint a `pi_…` and a checkout session, and then both
/// reach `Invoices::attach_intent`'s compare-and-swap on the same row. Under
/// `READ COMMITTED` the loser blocks on the winner's row lock and
/// re-evaluates `NO_LIVE_INTENT` against the committed row, so it matches
/// nothing and answers `409`.
///
/// The loser's intent is left unattached and unconfirmed, which is the
/// failure mode `pay`'s own doc names and prefers: an orphan `pi_…` nobody
/// paid, versus an invoice pointing at an intent that does not exist. This
/// case pins that it stays `requires_payment_method` and that it is not on
/// the invoice.
#[tokio::test]
async fn two_concurrent_pays_attach_exactly_one_intent() -> anyhow::Result<()> {
    let harness = harness().await?;
    let invoice = harness.draft_with_a_line(CLIENT_A).await?;
    let (status, body) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{invoice}/finalize"), &[])
        .await?;
    assert_eq!(status, 200, "{body}");

    // Built before the join so no `format!` temporary is borrowed across an
    // await point — `two_concurrent_finalizes_take_consecutive_numbers`' rule.
    let path = format!("/v1/invoices/{invoice}/pay");
    let form = [("success_url", SUCCESS_URL), ("cancel_url", CANCEL_URL)];
    let (left, right) = tokio::join!(
        harness.post(CLIENT_A, &path, &form),
        harness.post(CLIENT_A, &path, &form),
    );
    let (left_status, left_body) = left?;
    let (right_status, right_body) = right?;

    let mut statuses = [left_status.as_u16(), right_status.as_u16()];
    statuses.sort_unstable();
    assert_eq!(
        statuses,
        [200, 409],
        "one payment at a time: {left_body} | {right_body}"
    );

    // Exactly one intent is bound to the invoice, and it is the winner's.
    let attached: Vec<String> = sqlx::query_scalar(
        "SELECT payment_intent_id FROM invoices \
         WHERE payment_intent_id IS NOT NULL",
    )
    .fetch_all(&harness.pool)
    .await
    .context("reading which intents are bound to an invoice")?;
    let attached = only(&attached, "invoice with an intent attached");

    let winner = if left_status == 200 {
        &left_body
    } else {
        &right_body
    };
    assert_eq!(
        field(winner, "payment_intent"),
        attached.as_str(),
        "the request that answered 200 is the one whose intent is on the row"
    );

    // Every other intent this pair minted is an orphan: unattached, never
    // confirmed, and cancellable. It is not a second way to pay this bill.
    let orphans: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, status::TEXT FROM payment_intents \
         WHERE merchant_id = $1 AND id <> $2",
    )
    .bind(MERCHANT_A)
    .bind(attached)
    .fetch_all(&harness.pool)
    .await
    .context("reading the intents the losing request minted")?;
    assert!(
        orphans.len() <= 1,
        "two pay requests cannot mint more than two intents: {orphans:?}"
    );
    for (id, status) in &orphans {
        assert_eq!(
            status, "requires_payment_method",
            "the orphan {id} was never confirmed"
        );
    }

    harness.shutdown().await;
    Ok(())
}

/// The `starting_after` and `ending_before` cursors are resolved **inside the
/// merchant's scope**, so another merchant's `in_…` is neither an existence
/// oracle nor a way to widen the page.
///
/// # The ordering of the fixtures is the whole test
///
/// Measured, not assumed: with merchant B holding a single invoice *newer*
/// than merchant A's, both cursor sub-selects answer an empty page whether or
/// not they carry `AND merchant_id = $1` — `seq < seq(theirs)` excludes B's
/// only row either way, so the mutation that unscopes the cursor passes. The
/// fixtures below therefore straddle merchant A's invoice: one of B's is
/// older and one is newer, so an unscoped `starting_after` returns the older
/// one and an unscoped `ending_before` returns the newer one, and each cursor
/// has a row it would leak.
///
/// Two failures are possible and this case refuses both: answering a `404`
/// (which would say "that id exists, just not for you") and honouring the
/// foreign cursor against the caller's own rows (which hands back a page the
/// cursor never described). `list_page`'s sub-selects carry `AND merchant_id
/// = $1`, so a foreign id resolves to `NULL`, the comparison is `NULL`, and
/// the page is empty.
///
/// **The decisive mutation:** delete `AND merchant_id = $1` from either
/// cursor sub-select in `vpay_db::invoices`' `list_page` — the matching
/// assertion below sees one of merchant B's own invoices come back.
#[tokio::test]
async fn a_foreign_cursor_is_an_empty_page_and_never_an_oracle() -> anyhow::Result<()> {
    let harness = harness().await?;

    // Straddling merchant A's invoice, oldest first. See this case's doc.
    let older = harness.draft_with_a_line(CLIENT_B).await?;
    let theirs = harness.draft_with_a_line(CLIENT_A).await?;
    let newer = harness.draft_with_a_line(CLIENT_B).await?;

    // Merchant B's unpaged view, so "the cursor was honoured against my rows"
    // and "the cursor refused" are visibly different answers.
    let (status, page) = harness.get(CLIENT_B, "/v1/invoices").await?;
    assert_eq!(status, 200, "{page}");
    let data = field(&page, "data").as_array().expect("a list");
    assert_eq!(data.len(), 2, "merchant B has exactly two: {page}");
    assert_eq!(
        field(item(data, 0, "invoice"), "id"),
        newer.as_str(),
        "newest first"
    );
    assert_eq!(field(item(data, 1, "invoice"), "id"), older.as_str());

    for (cursor, would_leak) in [("starting_after", &older), ("ending_before", &newer)] {
        let (status, page) = harness
            .get(CLIENT_B, &format!("/v1/invoices?{cursor}={theirs}"))
            .await?;
        assert_eq!(status, 200, "a foreign cursor is not a 404: {page}");
        let data = field(&page, "data").as_array().expect("a list");
        assert!(
            data.is_empty(),
            "{cursor} pointing at another merchant's invoice leaked {would_leak}: {page}"
        );
    }

    // An id of the right shape that never existed answers identically, so a
    // caller cannot tell "not yours" from "never existed" by paging either.
    for cursor in ["starting_after", "ending_before"] {
        let (status, page) = harness
            .get(
                CLIENT_B,
                &format!("/v1/invoices?{cursor}={MISSING_INVOICE_ID}"),
            )
            .await?;
        assert_eq!(status, 200, "{page}");
        assert!(field(&page, "data").as_array().expect("a list").is_empty());
    }

    harness.shutdown().await;
    Ok(())
}

/// An invoice cannot be issued for more than this API can represent exactly,
/// and `pay` therefore cannot mint an intent above the ceiling
/// `POST /v1/payment_intents` enforces on its own `amount`.
///
/// # The hole this closes, measured before it was closed
///
/// `vpay_api::v1::payment_intents`' `parse_amount` refuses an `amount` above
/// `2^53 - 1` and says why: beyond it a JSON number stops round-tripping
/// through an IEEE-754 double, which is what every JavaScript client — the
/// `@vaam-apps/vpay-sdk` Node SDK and a merchant's own `stripe`-shaped
/// handler alike — parses a body with. `POST /v1/invoices/{id}/pay` does not
/// go through that function: it calls `PaymentIntents::insert` with
/// `amount_remaining` straight off the invoice, and nothing between a line
/// and that call bounded the total. Ninety-one lines at both parameter
/// ceilings (`quantity` 1,000,000 x `unit_amount` 100,000,000 = 10^14 each)
/// is 9.1 x 10^15, past `2^53 - 1 = 9,007,199,254,740,991` — so before this
/// bound a merchant could reach, through the public API and with no special
/// privilege, an invoice **and** a payment intent whose `amount` a merchant's
/// own SDK silently rounds.
///
/// The refusal is at **finalize** rather than at `pay` or at the line write,
/// deliberately. At `pay` it would leave an `open` invoice nobody could ever
/// pay; on the line write the row is already committed by the time the
/// re-summed total comes back. At finalize the draft is still editable, which
/// is the only refusal a merchant can act on.
///
/// **The decisive mutation:** delete the `amount_due` ceiling check from
/// `vpay_api::v1::invoices`' `transition_once` — the finalize below answers
/// `200` and the `pay` after it mints an intent for 9,100,000,000,000,000.
#[tokio::test]
async fn an_invoice_over_the_representable_ceiling_is_refused_at_finalize() -> anyhow::Result<()> {
    /// `vpay_api::v1::payment_intents::MAX_AMOUNT`, restated here because a
    /// test that imported it could not tell the two apart if both moved.
    const MAX_AMOUNT: i64 = (1_i64 << 53) - 1;
    /// One line at both of `invoice_items`' parameter ceilings.
    const PER_LINE: i64 = 1_000_000 * 100_000_000;

    let harness = harness().await?;
    let customer = harness.customer(CLIENT_A).await?;
    let (status, created) = harness
        .post(
            CLIENT_A,
            "/v1/invoices",
            &[("customer", &customer), ("currency", "xaf")],
        )
        .await?;
    assert_eq!(status, 201, "{created}");
    let invoice = field(&created, "id")
        .as_str()
        .expect("an invoice has an id")
        .to_owned();

    // The smallest number of maximal lines that passes the ceiling.
    let lines = (MAX_AMOUNT / PER_LINE) + 1;
    let mut last_line = String::new();
    for index in 0..lines {
        let (status, body) = harness
            .post(
                CLIENT_A,
                "/v1/invoice_items",
                &[
                    ("invoice", &invoice),
                    ("description", "a line at both parameter ceilings"),
                    ("quantity", "1000000"),
                    ("unit_amount", "100000000"),
                ],
            )
            .await?;
        anyhow::ensure!(status == 201, "line {index}: {status} {body}");
        last_line = field(&body, "id")
            .as_str()
            .expect("a line has an id")
            .to_owned();
    }

    let (_, draft) = harness
        .get(CLIENT_A, &format!("/v1/invoices/{invoice}"))
        .await?;
    let over = lines * PER_LINE;
    assert_eq!(field(&draft, "amount_due"), over);
    assert!(over > MAX_AMOUNT, "the fixture must actually be over it");

    let (status, body) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{invoice}/finalize"), &[])
        .await?;
    assert_eq!(status, 400, "a draft past the ceiling is refused: {body}");
    assert_eq!(at(&body, &["error", "param"]), "invoice");
    assert_eq!(
        harness.next_number(MERCHANT_A).await?,
        None,
        "a refused finalize burns no number, this one included"
    );

    // It is a ceiling and not a wall: remove one line and the document issues,
    // and the intent `pay` mints is inside the bound `POST /v1/payment_intents`
    // would have enforced on it.
    let (status, body) = harness
        .delete(CLIENT_A, &format!("/v1/invoice_items/{last_line}"))
        .await?;
    assert_eq!(status, 200, "{body}");

    let (status, open) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{invoice}/finalize"), &[])
        .await?;
    assert_eq!(status, 200, "one line lighter, it issues: {open}");
    assert_eq!(field(&open, "amount_due"), (lines - 1) * PER_LINE);

    let (status, paying) = harness
        .post(
            CLIENT_A,
            &format!("/v1/invoices/{invoice}/pay"),
            &[("success_url", SUCCESS_URL), ("cancel_url", CANCEL_URL)],
        )
        .await?;
    assert_eq!(status, 200, "{paying}");
    let intent = field(&paying, "payment_intent")
        .as_str()
        .expect("pay attaches an intent")
        .to_owned();
    let (_, pi) = harness
        .get(CLIENT_A, &format!("/v1/payment_intents/{intent}"))
        .await?;
    let amount = field(&pi, "amount").as_i64().expect("an amount");
    assert!(
        amount <= MAX_AMOUNT,
        "`pay` minted an intent for {amount}, past the {MAX_AMOUNT} `POST /v1/payment_intents` \
         refuses — a JSON number that no longer round-trips through a double"
    );

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------- per-merchant forwarding URLs

/// The URLs the checkout session for an invoice's intent actually carries,
/// read straight out of `checkout_sessions`.
///
/// Out of the table and not off the `pay` response, deliberately: the
/// response carries `hosted_invoice_url`, which is a link to vpay's own page
/// and says nothing about where the payer goes *afterwards*. The two columns
/// below are the whole of what D2 changes, and they are what migration
/// `0028`'s `urls_match_ui_mode` constrains.
async fn session_urls(
    pool: &PgPool,
    payment_intent_id: &str,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT success_url, cancel_url FROM checkout_sessions WHERE payment_intent_id = $1",
    )
    .bind(payment_intent_id)
    .fetch_one(pool)
    .await
    .context("reading the checkout session `pay` minted")
}

/// A finalized invoice of `client_id`'s, ready to be paid.
async fn open_invoice(harness: &Harness, client_id: &str) -> anyhow::Result<String> {
    let invoice = harness.draft_with_a_line(client_id).await?;
    let (status, body) = harness
        .post(client_id, &format!("/v1/invoices/{invoice}/finalize"), &[])
        .await?;
    anyhow::ensure!(status == 200, "finalizing: {status} {body}");
    Ok(invoice)
}

/// `POST /v1/invoices/{id}/pay` with **no body** uses the merchant's
/// configured `merchant_clients[].invoices` URLs; a request that sends its
/// own wins; a merchant with neither gets one `400` naming both (issue #91,
/// D2).
///
/// # Why all three halves are one case
///
/// They are one rule — *request, then configuration, then refuse* — and the
/// only way to be sure the fallback is a fallback rather than an override is
/// to see the same handler prefer a passed value in the next breath. Split
/// across three cases, a mutation that made configuration win could leave two
/// of them green.
///
/// # The decisive mutations
///
/// * Drop the configuration lookup from `vpay_api::v1::invoices`'
///   `forward_urls` (resolve the request's value alone) — the first assertion
///   below fails: paying with an empty body answers `400` for a merchant that
///   configured both.
/// * Reverse the precedence (configuration first) — the second block fails:
///   the session carries `https://shop.gamma.example/thank-you` where the
///   request sent `https://shop.acme.example/invoice-paid`.
/// * Answer the missing case one parameter at a time — the third block
///   fails: the message no longer names `cancel_url` as well.
#[tokio::test]
async fn paying_uses_the_merchants_configured_urls_and_a_passed_one_still_wins()
-> anyhow::Result<()> {
    let harness = harness().await?;

    // 1. Configured, and the request sends nothing at all.
    let invoice = open_invoice(&harness, CLIENT_C).await?;
    let (status, body) = harness
        .post(CLIENT_C, &format!("/v1/invoices/{invoice}/pay"), &[])
        .await?;
    assert_eq!(
        status, 200,
        "a merchant that configured both URLs may omit them: {body}"
    );
    let intent = field(&body, "payment_intent")
        .as_str()
        .expect("pay attaches an intent")
        .to_owned();
    assert_eq!(
        session_urls(&harness.pool, &intent).await?,
        (
            Some(CONFIGURED_SUCCESS_URL.to_owned()),
            Some(CONFIGURED_CANCEL_URL.to_owned())
        ),
        "the session must carry the merchant's configured destinations, not vpay's guess"
    );

    // 2. Configured, and the request sends its own. The request wins — a
    //    default that could not be overridden would make a one-off
    //    destination impossible without editing the deployment's YAML.
    let second = open_invoice(&harness, CLIENT_C).await?;
    let (status, body) = harness
        .post(
            CLIENT_C,
            &format!("/v1/invoices/{second}/pay"),
            &[("success_url", SUCCESS_URL), ("cancel_url", CANCEL_URL)],
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let intent = field(&body, "payment_intent")
        .as_str()
        .expect("pay attaches an intent")
        .to_owned();
    assert_eq!(
        session_urls(&harness.pool, &intent).await?,
        (Some(SUCCESS_URL.to_owned()), Some(CANCEL_URL.to_owned())),
        "a URL on the request wins over the configured default"
    );

    // 3. Neither configured nor passed: one `400`, naming both, so a merchant
    //    learns the whole of the mistake in one round trip.
    let bare = open_invoice(&harness, CLIENT_A).await?;
    let (status, body) = harness
        .post(CLIENT_A, &format!("/v1/invoices/{bare}/pay"), &[])
        .await?;
    assert_eq!(status, 400, "{body}");
    let message = at(&body, &["error", "message"])
        .as_str()
        .expect("a 400 carries a message")
        .to_owned();
    assert!(
        message.contains("`success_url`") && message.contains("`cancel_url`"),
        "one refusal names both missing parameters: {message}"
    );
    assert_eq!(
        at(&body, &["error", "param"]).as_str(),
        Some("success_url"),
        "`param` names the first absent one, so a client can still point at a field"
    );

    // Nothing was written for the refusal: no intent, no session, and the
    // invoice is still `open`. A `400` that had already minted a `pi_…` would
    // be an orphan for every merchant who ever forgets a URL.
    let (_, unchanged) = harness
        .get(CLIENT_A, &format!("/v1/invoices/{bare}"))
        .await?;
    assert_eq!(field(&unchanged, "status").as_str(), Some("open"));
    assert_eq!(field(&unchanged, "payment_intent").as_str(), None);

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------- out-of-band payments (RFC-0004 § 6)

/// The constraint a refused statement tripped, or `None` if it was not
/// refused by a constraint at all.
///
/// Every multi-column CHECK case below asserts **which** constraint refused
/// the row, not merely that something did: since migration `0049` a `paid`
/// row can be refused by `paid_names_how` as well as by the constraint a case
/// is about, and an `is_err()` would pass for the wrong one.
fn refused_by(result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>) -> Option<String> {
    result.err().and_then(|error| {
        error
            .as_database_error()
            .and_then(|database| database.constraint().map(str::to_owned))
    })
}

/// `(ledger_transactions, ledger_entries)` row counts — an out-of-band
/// payment must leave both at zero.
async fn ledger_rows(pool: &PgPool) -> anyhow::Result<(i64, i64)> {
    sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM ledger_transactions), (SELECT COUNT(*) FROM ledger_entries)",
    )
    .fetch_one(pool)
    .await
    .context("counting ledger rows")
}

/// How many `manual_payments` rows one invoice has.
async fn manual_payment_count(pool: &PgPool, invoice: &str) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM manual_payments WHERE invoice_id = $1")
        .bind(invoice)
        .fetch_one(pool)
        .await
        .context("counting the invoice's payment records")
}

/// One raw `manual_payments` row for the constraint cases — written straight
/// past the API and the repository, which is the whole point. Every field
/// defaults to a row that agrees with a 5,000 XAF invoice of merchant B's
/// paid out of band; each case changes the one thing it is about.
struct RawRecord<'a> {
    id: &'a str,
    invoice: &'a str,
    merchant: &'a str,
    livemode: bool,
    method: &'a str,
    reference: Option<&'a str>,
    amount: i64,
    received_after_recording_seconds: i32,
    paid_out_of_band: bool,
}

impl<'a> RawRecord<'a> {
    fn new(id: &'a str, invoice: &'a str) -> Self {
        Self {
            id,
            invoice,
            merchant: MERCHANT_B,
            livemode: false,
            method: "cash",
            reference: None,
            amount: 5000,
            received_after_recording_seconds: 0,
            paid_out_of_band: true,
        }
    }

    async fn insert(&self, pool: &PgPool) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
        sqlx::query(
            "INSERT INTO manual_payments \
                (id, merchant_id, livemode, invoice_id, method, reference, received_at, \
                 amount, currency_code, created_at, paid_out_of_band) \
             VALUES ($1, $2, $3, $4, $5, $6, now() + make_interval(secs => $7::INT), $8, \
                     'XAF', now(), $9)",
        )
        .bind(self.id)
        .bind(self.merchant)
        .bind(self.livemode)
        .bind(self.invoice)
        .bind(self.method)
        .bind(self.reference)
        .bind(self.received_after_recording_seconds)
        .bind(self.amount)
        .bind(self.paid_out_of_band)
        .execute(pool)
        .await
    }
}

/// **The happy path**, for a merchant with **no** configured invoice URLs
/// and no publishable key: `open -> paid`, the amounts, `paid_at`, the flag
/// and the record on the object, `invoice.paid` written exactly once with the
/// record inside it, and **nothing** on the ledger.
///
/// Merchant B is the point of the fixture. `pay`'s hosted path resolves
/// `merchant_clients[].invoices` and needs a publishable key; B has neither,
/// so a fork taken *after* URL resolution would answer this case with the
/// `400` naming both URLs — the decisive mutation is moving the
/// `paid_out_of_band` fork below `forward_urls` in `pay_once`.
#[tokio::test]
async fn paying_out_of_band_needs_no_configured_urls_and_posts_nothing() -> anyhow::Result<()> {
    let harness = harness().await?;
    let invoice = open_invoice(&harness, CLIENT_B).await?;
    let (_, open) = harness
        .get(CLIENT_B, &format!("/v1/invoices/{invoice}"))
        .await?;
    let finalized_at = at(&open, &["status_transitions", "finalized_at"])
        .as_i64()
        .expect("an open invoice was finalized");
    let received_at = finalized_at.to_string();

    let (status, paid) = harness
        .post(
            CLIENT_B,
            &format!("/v1/invoices/{invoice}/pay"),
            &[
                ("paid_out_of_band", "true"),
                ("out_of_band[method]", "bank_transfer"),
                ("out_of_band[reference]", "AFB 2026/0917"),
                ("out_of_band[received_at]", &received_at),
            ],
        )
        .await?;
    assert_eq!(status, 200, "{paid}");

    assert_eq!(field(&paid, "status"), "paid");
    assert_eq!(field(&paid, "amount_due"), 5000);
    assert_eq!(field(&paid, "amount_paid"), 5000);
    assert_eq!(field(&paid, "amount_remaining"), 0);
    assert_eq!(field(&paid, "paid_out_of_band"), true);
    assert_eq!(
        field(&paid, "payment_intent"),
        &Value::Null,
        "no intent was minted"
    );
    assert_eq!(field(&paid, "hosted_invoice_url"), &Value::Null);
    assert!(
        at(&paid, &["status_transitions", "paid_at"])
            .as_i64()
            .is_some(),
        "paid_at is stamped: {paid}"
    );
    let record = field(&paid, "out_of_band_payment").clone();
    let record_id = field(&record, "id").as_str().unwrap_or_default().to_owned();
    assert!(record_id.starts_with("mp_"), "{record}");
    assert_eq!(
        record,
        serde_json::json!({
            "id": record_id,
            "method": "bank_transfer",
            "reference": "AFB 2026/0917",
            "received_at": finalized_at,
        })
    );
    assert_eq!(
        paid.as_object().map(serde_json::Map::len),
        Some(21),
        "the invoice object is the documented twenty-one keys: {paid}"
    );

    // A fresh read renders the same record, through the third read.
    let (_, read) = harness
        .get(CLIENT_B, &format!("/v1/invoices/{invoice}"))
        .await?;
    assert_eq!(field(&read, "out_of_band_payment"), &record);

    // `invoice.paid` exactly once, carrying the record — rendered from the
    // row the transaction wrote, so its body is the object the merchant got.
    let events = harness.events_of("invoice.paid").await?;
    let (object_id, data) = only(&events, "invoice.paid event");
    assert_eq!(object_id, &invoice);
    assert_eq!(field(data, "paid_out_of_band"), true);
    assert_eq!(field(data, "out_of_band_payment"), &record);

    // The stored record: the invoice's own amount, currency and tenant.
    let (amount, currency, merchant): (i64, String, String) = sqlx::query_as(
        "SELECT amount, currency_code, merchant_id FROM manual_payments WHERE invoice_id = $1",
    )
    .bind(&invoice)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(
        (amount, currency.as_str(), merchant.as_str()),
        (5000, "XAF", MERCHANT_B)
    );

    assert_eq!(
        ledger_rows(&harness.pool).await?,
        (0, 0),
        "no money crossed payer_clearing, so nothing is posted"
    );

    harness.shutdown().await;
    Ok(())
}

/// **A Stripe-shaped client sending only `paid_out_of_band=true` succeeds**,
/// and the record says `other` — Stripe's `pay` has the flag and nothing
/// else, so a client written against Stripe knows no `out_of_band[…]` at all.
///
/// Until the review of 2026-09-23 this was a `400` naming
/// `out_of_band[method]`; ADR-0024's D11 (accepted 2026-09-23, because D5 required it)
/// made the method optional. **The decisive mutation:** make the method
/// required again and this case is a `400`.
#[tokio::test]
async fn paid_out_of_band_alone_is_recorded_with_the_method_other() -> anyhow::Result<()> {
    let harness = harness().await?;
    let invoice = open_invoice(&harness, CLIENT_B).await?;
    let (status, paid) = harness
        .post(
            CLIENT_B,
            &format!("/v1/invoices/{invoice}/pay"),
            &[("paid_out_of_band", "true")],
        )
        .await?;
    assert_eq!(status, 200, "{paid}");
    assert_eq!(field(&paid, "status"), "paid");
    assert_eq!(at(&paid, &["out_of_band_payment", "method"]), "other");
    assert_eq!(
        at(&paid, &["out_of_band_payment", "reference"]),
        &Value::Null
    );
    assert_eq!(harness.events_of("invoice.paid").await?.len(), 1);

    harness.shutdown().await;
    Ok(())
}

/// Every malformed out-of-band request is a `400` **naming its parameter**,
/// and none of them writes anything.
#[tokio::test]
async fn each_malformed_out_of_band_request_is_a_four_hundred_naming_its_parameter()
-> anyhow::Result<()> {
    let harness = harness().await?;
    let invoice = open_invoice(&harness, CLIENT_C).await?;
    let path = format!("/v1/invoices/{invoice}/pay");
    let long = "r".repeat(501);
    let future = (time::OffsetDateTime::now_utc().unix_timestamp() + 3600).to_string();
    let flag = ("paid_out_of_band", "true");
    let cash = ("out_of_band[method]", "cash");

    let cases: Vec<(Vec<(&str, &str)>, &str)> = vec![
        (
            vec![flag, ("out_of_band[method]", "barter")],
            "out_of_band[method]",
        ),
        (
            vec![flag, cash, ("out_of_band[refrence]", "typo")],
            "out_of_band",
        ),
        (
            vec![flag, cash, ("out_of_band[reference]", long.as_str())],
            "out_of_band[reference]",
        ),
        (
            vec![flag, cash, ("out_of_band[received_at]", "yesterday")],
            "out_of_band[received_at]",
        ),
        (
            vec![flag, cash, ("out_of_band[received_at]", future.as_str())],
            "out_of_band[received_at]",
        ),
        // Before the invoice was finalized.
        (
            vec![flag, cash, ("out_of_band[received_at]", "1")],
            "out_of_band[received_at]",
        ),
        // Merchant C HAS configured URLs, so these two cannot be the
        // "neither sent nor configured" refusal: they are refused for being
        // SENT with a flag that gives them no meaning.
        (
            vec![flag, cash, ("success_url", SUCCESS_URL)],
            "success_url",
        ),
        (vec![flag, cash, ("cancel_url", CANCEL_URL)], "cancel_url"),
        // `out_of_band[…]` without the flag: a payment described and not
        // asked to be recorded — refused rather than a checkout minted.
        (vec![cash], "out_of_band[method]"),
        (vec![("paid_out_of_band", "yes")], "paid_out_of_band"),
    ];

    for (form, param) in &cases {
        let (status, body) = harness.post(CLIENT_C, &path, form).await?;
        assert_eq!(status, 400, "{form:?}: {body}");
        assert_eq!(
            at(&body, &["error", "param"]).as_str(),
            Some(*param),
            "{form:?}: {body}"
        );
    }

    let (_, still) = harness
        .get(CLIENT_C, &format!("/v1/invoices/{invoice}"))
        .await?;
    assert_eq!(field(&still, "status"), "open");
    assert_eq!(
        field(&still, "payment_intent"),
        &Value::Null,
        "no checkout was minted by any of them"
    );
    assert_eq!(manual_payment_count(&harness.pool, &invoice).await?, 0);
    assert_eq!(harness.events_of("invoice.paid").await?.len(), 0);

    harness.shutdown().await;
    Ok(())
}

/// While an intent that is not `canceled` is attached, an out-of-band payment
/// is the same `409` naming the intent that `void` gives — a merchant cannot
/// record cash while a payer may still be paying through the hosted link —
/// and once the intent is canceled it succeeds, **keeping** the canceled
/// intent on the invoice (the decision `docs/flows/invoices.md` records).
#[tokio::test]
async fn paying_out_of_band_is_refused_while_an_intent_is_live_and_allowed_once_it_is_canceled()
-> anyhow::Result<()> {
    let harness = harness().await?;
    let invoice = open_invoice(&harness, CLIENT_A).await?;
    let path = format!("/v1/invoices/{invoice}/pay");

    let (status, paying) = harness
        .post(
            CLIENT_A,
            &path,
            &[("success_url", SUCCESS_URL), ("cancel_url", CANCEL_URL)],
        )
        .await?;
    assert_eq!(status, 200, "{paying}");
    let intent = field(&paying, "payment_intent")
        .as_str()
        .expect("an intent")
        .to_owned();

    let cash = [
        ("paid_out_of_band", "true"),
        ("out_of_band[method]", "cash"),
    ];
    let (status, body) = harness.post(CLIENT_A, &path, &cash).await?;
    assert_eq!(status, 409, "{body}");
    assert!(
        at(&body, &["error", "message"])
            .as_str()
            .is_some_and(|message| message.contains(&intent)),
        "the refusal names the intent to cancel: {body}"
    );
    assert_eq!(manual_payment_count(&harness.pool, &invoice).await?, 0);

    let (status, body) = harness
        .post(
            CLIENT_A,
            &format!("/v1/payment_intents/{intent}/cancel"),
            &[],
        )
        .await?;
    assert_eq!(status, 200, "{body}");

    let (status, paid) = harness.post(CLIENT_A, &path, &cash).await?;
    assert_eq!(
        status, 200,
        "a canceled intent releases the invoice: {paid}"
    );
    assert_eq!(field(&paid, "status"), "paid");
    assert_eq!(field(&paid, "paid_out_of_band"), true);
    assert_eq!(
        field(&paid, "payment_intent").as_str(),
        Some(intent.as_str()),
        "the abandoned attempt stays on the document, as it does on a voided one"
    );
    // …and so does its session's URL, exactly as on a voided invoice: the
    // renderer derives it from the attached intent and does not ask whether
    // that intent is still payable. Recorded, not endorsed — see
    // `docs/flows/invoices.md` § "Paid out of band", decision 3.
    assert!(field(&paid, "hosted_invoice_url").is_string(), "{paid}");
    assert_eq!(field(&paid, "amount_paid"), 5000);

    harness.shutdown().await;
    Ok(())
}

/// Every status but `open` refuses it with the same `409` naming the status
/// the other transitions give — `draft`, `void`, `uncollectible`, and `paid`
/// (a second out-of-band payment included).
#[tokio::test]
async fn paying_out_of_band_is_refused_from_every_status_but_open() -> anyhow::Result<()> {
    let harness = harness().await?;
    let cash = [
        ("paid_out_of_band", "true"),
        ("out_of_band[method]", "cash"),
    ];

    let draft = harness.draft_with_a_line(CLIENT_B).await?;
    let voided = open_invoice(&harness, CLIENT_B).await?;
    harness
        .post(CLIENT_B, &format!("/v1/invoices/{voided}/void"), &[])
        .await?;
    let written_off = open_invoice(&harness, CLIENT_B).await?;
    harness
        .post(
            CLIENT_B,
            &format!("/v1/invoices/{written_off}/mark_uncollectible"),
            &[],
        )
        .await?;
    let paid = open_invoice(&harness, CLIENT_B).await?;
    let (status, body) = harness
        .post(CLIENT_B, &format!("/v1/invoices/{paid}/pay"), &cash)
        .await?;
    assert_eq!(status, 200, "{body}");

    for (invoice, label) in [
        (&draft, "draft"),
        (&voided, "void"),
        (&written_off, "uncollectible"),
        (&paid, "paid"),
    ] {
        let (status, body) = harness
            .post(CLIENT_B, &format!("/v1/invoices/{invoice}/pay"), &cash)
            .await?;
        assert_eq!(status, 409, "{label}: {body}");
        let expected = format!("`{label}`, not `open`");
        assert!(
            at(&body, &["error", "message"])
                .as_str()
                .is_some_and(|message| message.contains(&expected)),
            "the refusal names the status: {body}"
        );
    }
    assert_eq!(
        harness.events_of("invoice.paid").await?.len(),
        1,
        "only the one that succeeded wrote an event"
    );
    assert_eq!(manual_payment_count(&harness.pool, &paid).await?, 1);

    harness.shutdown().await;
    Ok(())
}

/// An `Idempotency-Key` replays an out-of-band payment's response and records
/// nothing twice; the same key with a different body is the `400`
/// `idempotency_key_in_use`.
#[tokio::test]
async fn an_out_of_band_payment_replays_under_its_idempotency_key() -> anyhow::Result<()> {
    let harness = harness().await?;
    let invoice = open_invoice(&harness, CLIENT_B).await?;
    let path = format!("/v1/invoices/{invoice}/pay");
    let key = fresh_key();
    let form = [
        ("paid_out_of_band", "true"),
        ("out_of_band[method]", "cheque"),
        ("out_of_band[reference]", "Cheque 0042"),
    ];

    let (first_status, first) = harness.post_with_key(CLIENT_B, &path, &form, &key).await?;
    assert_eq!(first_status, 200, "{first}");
    let (again_status, again) = harness.post_with_key(CLIENT_B, &path, &form, &key).await?;
    assert_eq!(again_status, 200, "{again}");
    assert_eq!(first, again, "a replay answers what the original did");

    let (status, body) = harness
        .post_with_key(
            CLIENT_B,
            &path,
            &[
                ("paid_out_of_band", "true"),
                ("out_of_band[method]", "cash"),
            ],
            &key,
        )
        .await?;
    assert_eq!(status, 400, "{body}");
    assert_eq!(at(&body, &["error", "code"]), "idempotency_key_in_use");

    assert_eq!(manual_payment_count(&harness.pool, &invoice).await?, 1);
    assert_eq!(harness.events_of("invoice.paid").await?.len(), 1);

    harness.shutdown().await;
    Ok(())
}

/// A hosted `pay` and an out-of-band `pay` racing on one invoice have
/// **exactly one winner**, five rounds over.
///
/// Either order is legal and each leaves one coherent story: the hosted pay
/// wins (the invoice is `open` with its intent, and there is no record and no
/// `invoice.paid`), or the out-of-band pay wins (the invoice is `paid` with a
/// record, and whatever intent the loser minted is unattached). Both
/// compare-and-swaps carry `NO_LIVE_INTENT` and `status = 'open'`, so under
/// `READ COMMITTED` the loser re-evaluates against the winner's committed row
/// and matches nothing.
#[tokio::test]
async fn a_hosted_pay_and_an_out_of_band_pay_racing_have_exactly_one_winner() -> anyhow::Result<()>
{
    let harness = harness().await?;
    let hosted = [("success_url", SUCCESS_URL), ("cancel_url", CANCEL_URL)];
    let cash = [
        ("paid_out_of_band", "true"),
        ("out_of_band[method]", "cash"),
    ];

    for round in 0..5 {
        let invoice = open_invoice(&harness, CLIENT_A).await?;
        let path = format!("/v1/invoices/{invoice}/pay");
        let (left, right) = tokio::join!(
            harness.post(CLIENT_A, &path, &hosted),
            harness.post(CLIENT_A, &path, &cash),
        );
        let (hosted_status, hosted_body) = left?;
        let (cash_status, cash_body) = right?;
        let mut statuses = [hosted_status.as_u16(), cash_status.as_u16()];
        statuses.sort_unstable();
        assert_eq!(
            statuses,
            [200, 409],
            "round {round}: one winner: {hosted_body} | {cash_body}"
        );

        let (_, now) = harness
            .get(CLIENT_A, &format!("/v1/invoices/{invoice}"))
            .await?;
        let records = manual_payment_count(&harness.pool, &invoice).await?;
        if cash_status == 200 {
            assert_eq!(field(&now, "status"), "paid", "round {round}: {now}");
            assert_eq!(records, 1);
        } else {
            assert_eq!(field(&now, "status"), "open", "round {round}: {now}");
            assert_eq!(field(&now, "paid_out_of_band"), false);
            assert_eq!(
                field(&now, "payment_intent"),
                field(&hosted_body, "payment_intent"),
                "round {round}: the hosted winner's intent is the one attached"
            );
            assert_eq!(records, 0);
        }
    }

    let paid_events = i64::try_from(harness.events_of("invoice.paid").await?.len())?;
    let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM manual_payments")
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(
        paid_events, records,
        "one invoice.paid per record, and none for a hosted winner"
    );

    harness.shutdown().await;
    Ok(())
}

/// Migration `0049`'s multi-column CHECKs, the single-column ones on the new
/// table and its unique index, written **straight past the API** — the drift
/// report cannot see the multi-column ones at all.
///
/// Each refusal is pinned to the constraint that must produce it
/// ([`refused_by`]). **The decisive mutation:** delete any one of them from
/// `0049` and the matching assertion reads `None` or another name.
#[tokio::test]
async fn the_out_of_band_invariants_are_enforced_by_the_database_itself() -> anyhow::Result<()> {
    let harness = harness().await?;
    let open = open_invoice(&harness, CLIENT_B).await?;

    // `paid_out_of_band_means_paid` — an open bill claiming it was settled.
    let refused = sqlx::query("UPDATE invoices SET paid_out_of_band = true WHERE id = $1")
        .bind(&open)
        .execute(&harness.pool)
        .await;
    assert_eq!(
        refused_by(refused),
        Some("paid_out_of_band_means_paid".to_owned())
    );

    // `paid_names_how` — a settled bill with neither an intent nor the flag.
    let refused = sqlx::query(
        "UPDATE invoices SET status = 'paid', amount_paid = amount_due, amount_remaining = 0, \
         paid_at = now() WHERE id = $1",
    )
    .bind(&open)
    .execute(&harness.pool)
    .await;
    assert_eq!(refused_by(refused), Some("paid_names_how".to_owned()));

    // A real out-of-band payment, to build the remaining rows against.
    let paid = open_invoice(&harness, CLIENT_B).await?;
    let (status, body) = harness
        .post(
            CLIENT_B,
            &format!("/v1/invoices/{paid}/pay"),
            &[
                ("paid_out_of_band", "true"),
                ("out_of_band[method]", "cash"),
            ],
        )
        .await?;
    assert_eq!(status, 200, "{body}");

    // `paid_out_of_band_is_never_refunded` — a rail refund on a bill no rail
    // collected.
    let refused = sqlx::query("UPDATE invoices SET amount_refunded = 1 WHERE id = $1")
        .bind(&paid)
        .execute(&harness.pool)
        .await;
    assert_eq!(
        refused_by(refused),
        Some("paid_out_of_band_is_never_refunded".to_owned())
    );

    let pool = &harness.pool;
    // `manual_payments_invoice_id_key` — a second statement about one bill.
    assert_eq!(
        refused_by(RawRecord::new("mp_second", &paid).insert(pool).await),
        Some("manual_payments_invoice_id_key".to_owned())
    );

    // The single-column ones on the record, against an invoice with none.
    // CHECKs are evaluated before the foreign key, so each names itself even
    // though every one of these rows would also fail the key below.
    let other = open_invoice(&harness, CLIENT_B).await?;
    let long = "r".repeat(501);
    for (row, constraint) in [
        (
            RawRecord {
                method: "barter",
                ..RawRecord::new("mp_barter", &other)
            },
            "manual_payments_method_enum_check",
        ),
        (
            RawRecord {
                reference: Some(""),
                ..RawRecord::new("mp_empty", &other)
            },
            "reference_length",
        ),
        (
            RawRecord {
                reference: Some(&long),
                ..RawRecord::new("mp_long", &other)
            },
            "reference_length",
        ),
        (
            RawRecord {
                amount: 0,
                ..RawRecord::new("mp_zero", &other)
            },
            "amount_positive",
        ),
        // `received_before_recorded` — multi-column: money received after
        // vpay recorded it. Thirty seconds of allowance, and not thirty-one.
        (
            RawRecord {
                received_after_recording_seconds: 31,
                ..RawRecord::new("mp_later", &other)
            },
            "received_before_recorded",
        ),
        // `records_an_out_of_band_payment` — the flag column must be `true`.
        (
            RawRecord {
                paid_out_of_band: false,
                ..RawRecord::new("mp_unflagged", &other)
            },
            "records_an_out_of_band_payment",
        ),
    ] {
        assert_eq!(
            refused_by(row.insert(pool).await),
            Some(constraint.to_owned()),
            "{}",
            row.id
        );
    }

    // `manual_payments_agree_with_their_invoice`, the composite foreign key.
    // A row that passes every CHECK — inside the clock allowance, too — is
    // still refused when its invoice is not flagged `paid_out_of_band`. This
    // row was STORABLE until the key existed.
    assert_eq!(
        refused_by(
            RawRecord {
                received_after_recording_seconds: 29,
                ..RawRecord::new("mp_edge", &other)
            }
            .insert(pool)
            .await
        ),
        Some("manual_payments_agree_with_their_invoice".to_owned()),
        "a record for an invoice nobody marked paid out of band is unstorable"
    );

    // …and against a flagged invoice with no record yet, every disagreement
    // on amount, tenant or mode is refused by the same key, and only the
    // agreeing row is storable. The invoice is flagged straight past the API
    // (the flag satisfies `paid_names_how`), which is the state a writer that
    // forgot to insert the record would leave.
    let flagged = open_invoice(&harness, CLIENT_B).await?;
    sqlx::query(
        "UPDATE invoices SET status = 'paid', amount_paid = amount_due, amount_remaining = 0, \
         paid_at = now(), paid_out_of_band = true WHERE id = $1",
    )
    .bind(&flagged)
    .execute(pool)
    .await
    .context("flagging an invoice with no record")?;
    for row in [
        RawRecord {
            amount: 4999,
            ..RawRecord::new("mp_short", &flagged)
        },
        RawRecord {
            merchant: MERCHANT_A,
            ..RawRecord::new("mp_tenant", &flagged)
        },
        RawRecord {
            livemode: true,
            ..RawRecord::new("mp_mode", &flagged)
        },
    ] {
        assert_eq!(
            refused_by(row.insert(pool).await),
            Some("manual_payments_agree_with_their_invoice".to_owned()),
            "{}",
            row.id
        );
    }
    assert_eq!(
        refused_by(RawRecord::new("mp_agrees", &flagged).insert(pool).await),
        None,
        "the record that agrees with its invoice is storable"
    );

    // `NO ACTION` on update: once a record exists, its invoice's referenced
    // columns are frozen.
    let refused = sqlx::query("UPDATE invoices SET livemode = true WHERE id = $1")
        .bind(&flagged)
        .execute(pool)
        .await;
    assert_eq!(
        refused_by(refused),
        Some("manual_payments_agree_with_their_invoice".to_owned())
    );

    harness.shutdown().await;
    Ok(())
}

/// **Erasing the invoice's customer redacts the out-of-band reference
/// everywhere it was copied** — the record, the stored `invoice.paid` body,
/// the stored idempotent response — and afterwards a reference on that
/// customer's invoice is refused while a payment without one still records.
///
/// The reference is classified `payment_reference`, `subject: payer`,
/// `control: redact` (`schemas/privacy-inventory.yaml`): a cheque or transfer
/// reference routinely names the payer, which is what the fixture's literal
/// does.
#[tokio::test]
async fn erasing_the_customer_redacts_the_out_of_band_reference_everywhere() -> anyhow::Result<()> {
    const WHO: &str = "Zzyzx Quibblewort";
    let harness = harness().await?;
    let paid = open_invoice(&harness, CLIENT_B).await?;
    let later = open_invoice(&harness, CLIENT_B).await?;
    let (_, invoice) = harness
        .get(CLIENT_B, &format!("/v1/invoices/{paid}"))
        .await?;
    let customer = field(&invoice, "customer")
        .as_str()
        .expect("a customer")
        .to_owned();
    // `draft_with_a_line` creates a customer per invoice; bind the second
    // invoice to the same payer so the erasure is about both.
    sqlx::query("UPDATE invoices SET customer_id = $2 WHERE id = $1")
        .bind(&later)
        .bind(&customer)
        .execute(&harness.pool)
        .await?;

    let key = fresh_key();
    let reference = format!("Cheque 0042 from {WHO}");
    let form = [
        ("paid_out_of_band", "true"),
        ("out_of_band[method]", "cheque"),
        ("out_of_band[reference]", reference.as_str()),
    ];
    let path = format!("/v1/invoices/{paid}/pay");
    let (status, body) = harness.post_with_key(CLIENT_B, &path, &form, &key).await?;
    assert_eq!(status, 200, "{body}");

    // Two deliveries of the `invoice.paid` event, seeded as the worker would
    // leave them: one still `pending` (its first attempt recorded a digest)
    // and one `succeeded`, both with a merchant endpoint's response that
    // echoed the payload back. The erasure has to clear the live one's digest
    // — the body it would re-render has changed — keep the terminal one's as
    // forensics, and mark both excerpts. Without these rows the two
    // statements that do that could be deleted with this case green.
    let (paid_event,): (String,) =
        sqlx::query_as("SELECT id FROM events WHERE type = 'invoice.paid' AND object_id = $1")
            .bind(&paid)
            .fetch_one(&harness.pool)
            .await
            .context("the invoice.paid event")?;
    for (endpoint, state) in [("whk_live", "pending"), ("whk_done", "succeeded")] {
        sqlx::query(
            "INSERT INTO webhook_deliveries \
                (event_id, endpoint_id, url, state, payload_sha256, response_excerpt) \
             VALUES ($1, $2, 'https://merchant.example.invalid/hook', $3, repeat('a', 64), $4)",
        )
        .bind(&paid_event)
        .bind(endpoint)
        .bind(state)
        .bind(format!("{{\"echo\":\"{reference}\"}}"))
        .execute(&harness.pool)
        .await
        .context("seeding a delivery of invoice.paid")?;
    }

    let everywhere = "SELECT \
         (SELECT COUNT(*) FROM manual_payments WHERE reference LIKE '%' || $1 || '%') \
       + (SELECT COUNT(*) FROM events WHERE data::text LIKE '%' || $1 || '%') \
       + (SELECT COUNT(*) FROM idempotency_keys WHERE response_body::text LIKE '%' || $1 || '%') \
       + (SELECT COUNT(*) FROM webhook_deliveries WHERE response_excerpt LIKE '%' || $1 || '%')";
    let before: i64 = sqlx::query_scalar(everywhere)
        .bind(WHO)
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(
        before, 5,
        "the record, the invoice.paid body, the stored response and both excerpts carry it"
    );

    let (status, body) = harness
        .delete(CLIENT_B, &format!("/v1/customers/{customer}"))
        .await?;
    assert_eq!(status, 200, "{body}");

    let deliveries: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT state, payload_sha256, response_excerpt FROM webhook_deliveries \
         WHERE event_id = $1 ORDER BY endpoint_id",
    )
    .bind(&paid_event)
    .fetch_all(&harness.pool)
    .await?;
    assert_eq!(
        deliveries,
        vec![
            (
                "succeeded".to_owned(),
                Some("a".repeat(64)),
                Some(vpay_db::REDACTED.to_owned())
            ),
            (
                "pending".to_owned(),
                None,
                Some(vpay_db::REDACTED.to_owned())
            ),
        ],
        "the live delivery's digest is cleared so its next attempt re-signs the redacted body; \
         the terminal one's is kept as forensics; both excerpts are marked"
    );

    let after: i64 = sqlx::query_scalar(everywhere)
        .bind(WHO)
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(after, 0, "no copy of the reference survives the erasure");

    let (_, read) = harness
        .get(CLIENT_B, &format!("/v1/invoices/{paid}"))
        .await?;
    assert_eq!(
        at(&read, &["out_of_band_payment", "reference"]).as_str(),
        Some(vpay_db::REDACTED)
    );
    let (_, replayed) = harness.post_with_key(CLIENT_B, &path, &form, &key).await?;
    assert_eq!(
        at(&replayed, &["out_of_band_payment", "reference"]).as_str(),
        Some(vpay_db::REDACTED),
        "a replay after an erasure answers the redacted body"
    );
    let events = harness.events_of("invoice.paid").await?;
    let (_, data) = only(&events, "invoice.paid event");
    assert_eq!(
        at(data, &["out_of_band_payment", "reference"]).as_str(),
        Some(vpay_db::REDACTED)
    );

    // The erased payer's other invoice: a reference would re-attach payer
    // detail, so it is refused — naming the parameter — and nothing is
    // written; without one, the payment records.
    let later_path = format!("/v1/invoices/{later}/pay");
    let (status, body) = harness
        .post(
            CLIENT_B,
            &later_path,
            &[
                ("paid_out_of_band", "true"),
                ("out_of_band[method]", "cash"),
                ("out_of_band[reference]", "Receipt 7"),
            ],
        )
        .await?;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        at(&body, &["error", "param"]).as_str(),
        Some("out_of_band[reference]")
    );
    assert_eq!(manual_payment_count(&harness.pool, &later).await?, 0);
    let (status, body) = harness
        .post(
            CLIENT_B,
            &later_path,
            &[
                ("paid_out_of_band", "true"),
                ("out_of_band[method]", "cash"),
            ],
        )
        .await?;
    assert_eq!(status, 200, "{body}");

    harness.shutdown().await;
    Ok(())
}

/// An erasure and an out-of-band payment with a reference, **racing** on one
/// payer: neither deadlocks, no reference survives either order, and the
/// payment's **stored idempotent response** is redacted too.
///
/// The lock-order argument (`vpay_db::invoices`' `pay_out_of_band_in_tx`):
/// the payment takes `FOR SHARE` on the customer **before** it touches the
/// invoice or `manual_payments`, and the erasure takes `FOR UPDATE` on the
/// customer before it touches anything, so both acquire the customer row
/// first and serialise on it; neither holds a lock the other wants second.
/// This case is the empirical half: a `40P01` would surface as a `500`.
///
/// # It records who won, and refuses to pass vacuously
///
/// Each round sends an `Idempotency-Key`. A `200` means the payment committed
/// first and **wrote** a reference, which the erasure then had to redact —
/// in the record, the event, and in the response the idempotency store kept
/// (whether that store landed before the erasure, which rewrites it, or after,
/// which is the `ResponseSubject::OutOfBandInvoice` late-write path). A `400`
/// naming `out_of_band[reference]` means the erasure committed first. The
/// erasure's start is staggered by a growing delay so the rounds sweep the
/// collision window; rounds run until **both** outcomes have been seen (at
/// least five, at most thirty), and the case fails if no round wrote a
/// reference: a race the erasure always wins would prove nothing about
/// redaction.
#[tokio::test]
async fn an_erasure_racing_an_out_of_band_payment_neither_deadlocks_nor_leaves_the_reference()
-> anyhow::Result<()> {
    let harness = harness().await?;
    let (mut payment_first, mut erasure_first) = (0_u32, 0_u32);

    for round in 0_u32..30 {
        if payment_first > 0 && erasure_first > 0 && round >= 5 {
            break;
        }
        let invoice = open_invoice(&harness, CLIENT_B).await?;
        let (_, read) = harness
            .get(CLIENT_B, &format!("/v1/invoices/{invoice}"))
            .await?;
        let customer = field(&read, "customer")
            .as_str()
            .expect("a customer")
            .to_owned();
        let pay_path = format!("/v1/invoices/{invoice}/pay");
        let erase_path = format!("/v1/customers/{customer}");
        let reference = format!("Transfer from Quibblewort round {round}");
        let form = [
            ("paid_out_of_band", "true"),
            ("out_of_band[method]", "bank_transfer"),
            ("out_of_band[reference]", reference.as_str()),
        ];
        let key = fresh_key();
        let (paying, erasing) = tokio::join!(
            harness.post_with_key(CLIENT_B, &pay_path, &form, &key),
            async {
                // A head start that grows by 3 ms a round, so the rounds sweep
                // the window in which the two collide rather than sampling
                // one point of it. Measured 2026-09-23: with no stagger the
                // erasure won 30 rounds of 30 — the pay request does three
                // reads before its transaction opens — and this case could
                // not see a payment-first round at all.
                tokio::time::sleep(std::time::Duration::from_millis(u64::from(round) * 3)).await;
                harness.delete(CLIENT_B, &erase_path).await
            },
        );
        let (pay_status, pay_body) = paying?;
        let (erase_status, erase_body) = erasing?;
        assert_eq!(erase_status, 200, "round {round}: {erase_body}");

        match pay_status.as_u16() {
            200 => {
                payment_first += 1;
                // The reference WAS written — it is what the payer's
                // statement said — and every copy of it is now the marker.
                let (stored,): (Option<String>,) =
                    sqlx::query_as("SELECT reference FROM manual_payments WHERE invoice_id = $1")
                        .bind(&invoice)
                        .fetch_one(&harness.pool)
                        .await?;
                assert_eq!(
                    stored.as_deref(),
                    Some(vpay_db::REDACTED),
                    "round {round}: the record's reference"
                );
                let (_, replayed) = harness
                    .post_with_key(CLIENT_B, &pay_path, &form, &key)
                    .await?;
                assert_eq!(
                    at(&replayed, &["out_of_band_payment", "reference"]).as_str(),
                    Some(vpay_db::REDACTED),
                    "round {round}: the stored idempotent response replays redacted: {replayed}"
                );
            }
            400 => {
                erasure_first += 1;
                assert_eq!(
                    at(&pay_body, &["error", "param"]).as_str(),
                    Some("out_of_band[reference]"),
                    "round {round}: {pay_body}"
                );
                assert_eq!(manual_payment_count(&harness.pool, &invoice).await?, 0);
            }
            other => panic!("round {round}: neither order produced {other}: {pay_body}"),
        }
    }

    assert!(
        payment_first > 0,
        "no round let the payment commit first ({erasure_first} erasure-first rounds), so no \
         round wrote a reference for the erasure to redact — the case would prove nothing"
    );

    let surviving: i64 = sqlx::query_scalar(
        "SELECT \
           (SELECT COUNT(*) FROM manual_payments WHERE reference LIKE '%Quibblewort%') \
         + (SELECT COUNT(*) FROM events WHERE data::text LIKE '%Quibblewort%') \
         + (SELECT COUNT(*) FROM idempotency_keys \
            WHERE response_body::text LIKE '%Quibblewort%')",
    )
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(
        surviving, 0,
        "whichever won ({payment_first} payment-first, {erasure_first} erasure-first), no \
         reference outlived its payer"
    );

    harness.shutdown().await;
    Ok(())
}

/// The bodiless `POST /v1/invoices/{id}` "touch" on an invoice paid out of
/// band answers the object — reference included — and that stored response
/// is redacted by an erasure: touch, erase, replay the touch → `[redacted]`.
///
/// This is the second body `update` now stores as
/// `ResponseSubject::OutOfBandInvoice`. Over HTTP the touch's store has
/// committed before the erasure starts, so what this proves is the erasure's
/// rewrite of stored invoice responses reaching a touch; the store's own
/// late-write branch is proven by
/// `a_stored_invoice_response_written_after_an_erasure_is_redacted_as_it_lands`.
#[tokio::test]
async fn a_touch_of_an_invoice_paid_out_of_band_replays_redacted_after_an_erasure()
-> anyhow::Result<()> {
    let harness = harness().await?;
    let invoice = open_invoice(&harness, CLIENT_B).await?;
    let (status, body) = harness
        .post(
            CLIENT_B,
            &format!("/v1/invoices/{invoice}/pay"),
            &[
                ("paid_out_of_band", "true"),
                ("out_of_band[reference]", "Cheque from Quibblewort"),
            ],
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let customer = field(&body, "customer")
        .as_str()
        .expect("a customer")
        .to_owned();

    let touch = format!("/v1/invoices/{invoice}");
    let key = fresh_key();
    let (status, touched) = harness.post_with_key(CLIENT_B, &touch, &[], &key).await?;
    assert_eq!(status, 200, "{touched}");
    assert_eq!(
        at(&touched, &["out_of_band_payment", "reference"]).as_str(),
        Some("Cheque from Quibblewort"),
        "the touch answers the object as it stands"
    );

    let (status, body) = harness
        .delete(CLIENT_B, &format!("/v1/customers/{customer}"))
        .await?;
    assert_eq!(status, 200, "{body}");

    let (status, replayed) = harness.post_with_key(CLIENT_B, &touch, &[], &key).await?;
    assert_eq!(status, 200, "{replayed}");
    assert_eq!(
        at(&replayed, &["out_of_band_payment", "reference"]).as_str(),
        Some(vpay_db::REDACTED),
        "the replayed touch answers the redacted body: {replayed}"
    );

    harness.shutdown().await;
    Ok(())
}

/// The idempotency store's **late write**: a response naming an invoice paid
/// out of band, stored under `ResponseSubject::OutOfBandInvoice` **after**
/// its customer was erased, is redacted as it lands — the branch the race
/// above can only hit by chance, driven here through the repository seam in
/// the order the race produces (claim, commit, erasure, store).
///
/// **The decisive mutation:** store it as `ResponseSubject::Verbatim` (what
/// `update` and the out-of-band `pay` stored before 2026-09-23) and the
/// replay carries the payer's reference for 24 hours after their erasure.
#[tokio::test]
async fn a_stored_invoice_response_written_after_an_erasure_is_redacted_as_it_lands()
-> anyhow::Result<()> {
    use vpay_db::{Idempotency, IdempotencyClaim, ResponseSubject, StoredResponse};

    let harness = harness().await?;
    let invoice = open_invoice(&harness, CLIENT_B).await?;
    let (status, paid) = harness
        .post(
            CLIENT_B,
            &format!("/v1/invoices/{invoice}/pay"),
            &[
                ("paid_out_of_band", "true"),
                ("out_of_band[reference]", "Cheque from Quibblewort"),
            ],
        )
        .await?;
    assert_eq!(status, 200, "{paid}");
    let customer = field(&paid, "customer")
        .as_str()
        .expect("a customer")
        .to_owned();

    // A request that has done its work and not yet stored its answer.
    let key = fresh_key();
    let claim = Idempotency::claim(
        harness.repositories.as_ref(),
        MERCHANT_B,
        &key,
        "POST",
        &format!("/v1/invoices/{invoice}"),
        &[7_u8; 32],
    )
    .await?;
    let IdempotencyClaim::Fresh { claim_id } = claim else {
        panic!("a fresh key is a fresh claim: {claim:?}");
    };

    // The erasure lands in between.
    let (status, body) = harness
        .delete(CLIENT_B, &format!("/v1/customers/{customer}"))
        .await?;
    assert_eq!(status, 200, "{body}");

    // And then the store, with the body rendered before the erasure.
    Idempotency::store(
        harness.repositories.as_ref(),
        MERCHANT_B,
        &key,
        claim_id,
        StoredResponse {
            status: 200,
            body: &paid,
            retry: None,
            subject: ResponseSubject::OutOfBandInvoice {
                customer_id: &customer,
            },
        },
    )
    .await?;

    let stored: Value = sqlx::query_scalar(
        "SELECT response_body FROM idempotency_keys WHERE merchant_id = $1 AND idempotency_key = $2",
    )
    .bind(MERCHANT_B)
    .bind(&key)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(
        at(&stored, &["out_of_band_payment", "reference"]).as_str(),
        Some(vpay_db::REDACTED),
        "the late write lost to the erasure: {stored}"
    );

    harness.shutdown().await;
    Ok(())
}

/// Another merchant's `in_…` answers an out-of-band payment with the same
/// `404` as an id that never existed — before the body is looked at.
#[tokio::test]
async fn an_out_of_band_payment_on_another_merchants_invoice_is_the_uniform_not_found()
-> anyhow::Result<()> {
    let harness = harness().await?;
    let theirs = open_invoice(&harness, CLIENT_A).await?;
    let cash = [
        ("paid_out_of_band", "true"),
        ("out_of_band[method]", "cash"),
    ];

    let (their_status, their_body) = harness
        .post(CLIENT_B, &format!("/v1/invoices/{theirs}/pay"), &cash)
        .await?;
    let (missing_status, missing_body) = harness
        .post(
            CLIENT_B,
            &format!("/v1/invoices/{MISSING_INVOICE_ID}/pay"),
            &cash,
        )
        .await?;
    assert_eq!((their_status.as_u16(), missing_status.as_u16()), (404, 404));
    assert_eq!(
        their_body.to_string().replace(&theirs, "<ID>"),
        missing_body.to_string().replace(MISSING_INVOICE_ID, "<ID>"),
    );
    let (_, still) = harness
        .get(CLIENT_A, &format!("/v1/invoices/{theirs}"))
        .await?;
    assert_eq!(field(&still, "status"), "open");

    harness.shutdown().await;
    Ok(())
}
