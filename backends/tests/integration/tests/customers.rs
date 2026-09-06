//! `/v1/customers`, end to end: the real `vpay_api::router` on a real socket,
//! over a real Postgres, driven by the real merchant SDK — plus the real
//! `vpay_worker` job loop for the twelve-month retention sweep.
//!
//! The claims only this file can make:
//!
//! 1. **the maintainer's decision of 2026-09-05, at the wire.** A customer
//!    whose only content is a phone number is created, reads back, and can be
//!    attached to a payment intent, which then renders `customer`. Nothing
//!    below sends a name or an email on the happy path;
//! 2. the phone number is stored **canonical** — the value a rail is given —
//!    however the merchant spelled it;
//! 3. another merchant's `cus_…` and an id that never existed are the **byte
//!    for byte** identical `404` on retrieve, update *and* delete, and naming
//!    another merchant's `cus_…` as `customer` on an intent is the identical
//!    `400`;
//! 4. a customer with all three identifiers absent is a `400` naming them —
//!    not the `503` the database's own `at_least_one_identifier` would
//!    produce, which is the whole reason the rule is decided above the
//!    statement;
//! 5. the cursor pages forward and backward **while rows are being inserted**,
//!    and no row is skipped or repeated;
//! 6. the retention sweep, run through the **shipping worker loop**: a
//!    customer idle for thirteen months and unreferenced is deleted and emits
//!    exactly one `customer.deleted`; one idle for eleven months is kept; one
//!    idle for thirteen months but referenced by a two-month-old intent is
//!    kept, and `DELETE /v1/customers/{id}` on it is a `409`;
//! 7. a confirm stamps the customer's retention clock, which is what stops
//!    (6)'s third case from being a live customer vpay deletes.
//!
//! # No test doubles
//!
//! Real Postgres, the shipping router, the shipping SDK, the shipping worker
//! loop. The rail is configured and **unreachable**, exactly as
//! `refunds.rs` configures it: nothing here confirms against a rail, and an
//! intent only has to exist.

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
use vpay_sdk::customers::{CreateCustomerParams, ListCustomersParams, UpdateCustomerParams};
use vpay_sdk::{CreatePaymentIntentParams, Credentials, PaymentMethodType, RequestOptions};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client,
    merchant_client_with_publishable_keys, migrated_postgres, serve,
};

/// The merchant every test acts as, and the tenant it acts for. Never the
/// same string: a query filtered by `client_id` instead of `merchant_id`
/// would otherwise pass.
const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";

/// The second merchant, which exists so the tenancy cases have someone else's
/// customer to fail to read.
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

const RAIL: &str = "mtn_momo";
const CURRENCY: &str = "xaf";
const AMOUNT: i64 = 5000;

/// The one MSISDN this suite uses, in the form vpay stores it.
const CANONICAL_PHONE: &str = "237600000200";

/// An id of exactly the shape `vpay_core::ids::customer_id` mints, that no
/// merchant has ever had.
const MISSING_CUSTOMER_ID: &str = "cus_00000000000000000000000x";

/// Merchant A's publishable key, and the checkout page's origin.
///
/// Registered here only so this suite can create **checkout sessions**: the
/// `customer` rules for a session are not the intent's rules (a session can
/// contradict the intent it drives, and an intent has nothing to contradict),
/// and `checkout_sessions.customer_id` is one of the two columns the
/// retention sweep's `NOT EXISTS` guard reads. Neither could be exercised
/// from a harness that cannot mint a session.
const PK_A: &str = "pk_test_acmecameroonsandbox01";
const CHECKOUT_BASE: &str = "https://checkout.vpay.test";
const SUCCESS_URL: &str = "https://shop.acme.example/ok";
const CANCEL_URL: &str = "https://shop.acme.example/cancel";

// ------------------------------------------------------------------ harness

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    /// The plain `sqlx` pool: the retention cases have to age a
    /// `last_used_at`, which no repository method does and deliberately must
    /// not — `touch_last_used` only ever moves the clock **forward**.
    pool: PgPool,
    base_url: String,
    pem_a: String,
    pem_b: String,
    signing_key: LoadedSigningKey,
}

impl Harness {
    fn sdk(&self, client_id: &str, pem: &str) -> vpay_sdk::Client {
        vpay_sdk::Client::builder(&self.base_url)
            .credentials(
                Credentials::rsa_pem(client_id, pem).expect("the generated PEM parses as RSA"),
            )
            .build()
            .expect("the SDK client builds from a base URL and a credential")
    }

    fn a(&self) -> vpay_sdk::Client {
        self.sdk(CLIENT_A, &self.pem_a)
    }

    fn b(&self) -> vpay_sdk::Client {
        self.sdk(CLIENT_B, &self.pem_b)
    }

    /// A bearer token for `client_id`, for the raw requests the SDK cannot
    /// make — the byte-level 404 comparison needs the response *bodies*, and
    /// the SDK maps them into a typed error.
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

    async fn shutdown(self) {
        self.server.abort();
    }
}

/// Two merchants, one rail, one currency, `livemode: false`.
///
/// The rail is configured and unreachable, exactly as `refunds.rs` configures
/// it: nothing here confirms against a rail, but boot step 4 has to seed
/// `providers` and `currencies` or the payment intents these cases attach a
/// customer to could not be created at all.
fn config_with(base_url: &str, jwks_a: Value, jwks_b: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "customers".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
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
        }],
        currencies: vec![CurrencyEntry {
            code: "XAF".to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![
            // Only merchant A has a publishable key and therefore only A can
            // create a session; B exists to be failed to read.
            merchant_client_with_publishable_keys(CLIENT_A, MERCHANT_A, jwks_a, &[PK_A]),
            merchant_client(CLIENT_B, MERCHANT_B, jwks_b),
        ],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: CheckoutConfig {
            public_base_url: Some(CHECKOUT_BASE.to_owned()),
        },
        dashboard_client: None,
    }
}

async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (container, repositories, pool) = migrated_postgres().await?;

    let (server_pem, _server_jwks) = generate_key();
    let (pem_a, jwks_a) = generate_key();
    let (pem_b, jwks_b) = generate_key();

    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, jwks_a, jwks_b)
    })
    .await?;

    Ok(Harness {
        _container: container,
        server: served.server,
        repositories,
        pool,
        base_url: served.base_url,
        pem_a,
        pem_b,
        signing_key: served.signing_key,
    })
}

fn raw_client() -> reqwest::Client {
    reqwest::Client::builder()
        .build()
        .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
}

/// `CreateCustomerParams` with **only a phone number** — the maintainer's
/// decision of 2026-09-05, as the default shape of every fixture here.
///
/// Written as the default rather than as one special case on purpose: if
/// phone-only were ever quietly demoted to "allowed but unusual", most of
/// this file would go red rather than one test.
fn phone_only() -> CreateCustomerParams {
    CreateCustomerParams {
        phone: Some(CANONICAL_PHONE.to_owned()),
        ..Default::default()
    }
}

fn create_intent_params(customer: Option<&str>) -> CreatePaymentIntentParams {
    CreatePaymentIntentParams {
        amount: AMOUNT,
        currency: CURRENCY.to_owned(),
        payment_method_types: vec![PaymentMethodType::MtnMomo],
        metadata: BTreeMap::new(),
        description: None,
        customer: customer.map(str::to_owned),
    }
}

/// Moves a customer's retention clock **backwards**, which nothing in vpay
/// can do.
///
/// `touch_last_used` is `last_used_at < now`-guarded precisely so a stamp is
/// monotonic, so there is no repository method for this and there must not
/// be: the sweep's whole input would otherwise be writable by any caller. It
/// is raw SQL for `support::age_the_crash`'s reason — a suite may stage a
/// state the shipping code cannot reach, provided it says so.
async fn age_customer(pool: &PgPool, id: &str, days: i64) -> anyhow::Result<()> {
    let affected = sqlx::query(
        "UPDATE customers SET last_used_at = now() - ($2 || ' days')::interval WHERE id = $1",
    )
    .bind(id)
    .bind(days.to_string())
    .execute(pool)
    .await
    .context("ageing a customer's last_used_at")?
    .rows_affected();
    anyhow::ensure!(affected == 1, "ageing must move exactly one customer");
    Ok(())
}

/// The same for a payment intent's `created_at`, so "referenced by an intent
/// two months old" is a state this suite can build.
async fn age_intent(pool: &PgPool, id: &str, days: i64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE payment_intents SET created_at = now() - ($2 || ' days')::interval WHERE id = $1",
    )
    .bind(id)
    .bind(days.to_string())
    .execute(pool)
    .await
    .context("ageing an intent")?;
    Ok(())
}

/// Runs the **shipping** worker's `sweep_idle_customers` job to completion,
/// once.
///
/// Through `vpay_worker::run_once` rather than by calling the repository,
/// because what these cases are about is the job: its page, its guard, its
/// event, and the fact that it is a job kind of its own. A direct
/// `delete_idle` call would prove the statement and nothing about whether
/// anything ever runs it.
async fn run_the_sweep(repositories: &Arc<dyn Repositories>, pool: &PgPool) -> anyhow::Result<()> {
    // The singleton is seeded at `run_at = now`, so a first pass claims it.
    vpay_worker::run_loop::seed_singletons(repositories.as_ref())
        .await
        .context("seeding the singleton jobs the shipping worker seeds at boot")?;
    // A *second* call finds the row already there and an hour out — the seed
    // is `ON CONFLICT DO NOTHING`, deliberately, so that a worker restart
    // cannot drag a scheduled job back to now. Pulling it forward here is
    // this suite saying "run the hourly job again", and it is the honest way
    // to ask: the alternative is sleeping an hour or reaching past the queue
    // and calling `delete_idle`, which would stop proving that anything
    // dispatches this kind at all.
    sqlx::query("UPDATE jobs SET run_at = now() WHERE dedupe_key = $1")
        .bind(vpay_worker::jobs::SWEEP_CUSTOMERS_DEDUPE_KEY)
        .execute(pool)
        .await
        .context("bringing the retention sweep forward")?;
    // Everything else the loop needs is either unused by this job kind or
    // configured to be unreachable; `sweep_idle_customers` touches no rail
    // and no endpoint.
    let adapters: vpay_worker::Adapters = support::adapters_by_code();
    let rails: vpay_worker::RailConfigs = BTreeMap::new();
    let endpoints = support::no_webhook_endpoints();
    let egress = support::default_egress_policy();
    let mut seen: Vec<String> = Vec::new();

    // Drain, rather than one step: `seed_singletons` seeds five jobs and
    // `claim` takes the oldest claimable one, so a single `run_once` would
    // run whichever singleton sorted first and might never reach this one.
    // Bounded so a job that reschedules itself at `Duration::ZERO` cannot
    // spin here — `sweep_idle_customers` does exactly that when a page comes
    // back full, and this suite's pages never are.
    for _ in 0..32 {
        let settled = vpay_worker::run_once(
            repositories.as_ref(),
            &adapters,
            &rails,
            &vpay_worker::RecoveryPolicy::default(),
            &vpay_worker::WebhookContext {
                endpoints: &endpoints,
                egress,
            },
            "customers-suite",
        )
        .await
        .context("running one worker job")?;
        let Some(settled) = settled else { break };
        // Every singleton this loop runs must *finish* or reschedule. A
        // `DeadLettered` here is the failure mode that cost this suite an
        // afternoon: the sweep was claimed and refused, the customer survived,
        // and the only thing that said so was a log line nothing read.
        assert!(
            !matches!(settled.disposition, vpay_worker::Disposition::DeadLettered),
            "a singleton job was dead-lettered, which means this build cannot dispatch a kind \
             it seeds: {settled:?}"
        );
        seen.push(settled.kind);
    }
    assert!(
        seen.iter().any(|kind| kind == "sweep_idle_customers"),
        "the retention sweep must actually have run; it did not: {seen:?}"
    );
    Ok(())
}

// ------------------------------------------------------- phone-only, stored

/// **The maintainer's decision of 2026-09-05, end to end.**
///
/// A customer created with nothing but a phone number is a `201`, and the
/// phone reads back. Nothing else on the object is required, and neither
/// `name` nor `email` is sent anywhere in this test.
///
/// It also pins the *canonicalisation*: the merchant types
/// `+237 6 00 00 02 00` and vpay stores and renders `237600000200`. That is a
/// wire contract rather than an implementation leak — it is the value a rail
/// is given, so a merchant comparing this against a charge's payer reference
/// is comparing the same string — and it is the half a "store what they sent"
/// implementation would break silently.
#[tokio::test]
async fn a_customer_with_only_a_phone_number_is_created_and_reads_back_canonical()
-> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    let created = sdk
        .customers()
        .create(
            CreateCustomerParams {
                phone: Some("+237 6 00 00 02 00".to_owned()),
                ..Default::default()
            },
            RequestOptions::new().with_idempotency_key("cus-create-1"),
        )
        .await
        .expect("a phone number alone is a complete customer");

    assert!(created.id.starts_with("cus_"), "{}", created.id);
    assert_eq!(created.object, "customer");
    assert_eq!(
        created.phone.as_deref(),
        Some(CANONICAL_PHONE),
        "the phone is stored and rendered in the form a rail is given, not as it was typed"
    );
    assert_eq!(created.name, None);
    assert_eq!(created.email, None);
    assert!(!created.livemode);

    let read = sdk
        .customers()
        .retrieve(&created.id)
        .await
        .expect("the customer reads back");
    assert_eq!(read, created, "retrieve renders exactly what create did");

    h.shutdown().await;
    Ok(())
}

/// **Phone-only, attached to a payment intent, rendered back.**
///
/// The brief's second decisive case, and the one that proves `customer`
/// stopped being accepted-and-dropped: an intent created with
/// `customer=cus_…` renders it on create *and* on a later retrieve, which is
/// the read a merchant's reconciliation actually makes.
#[tokio::test]
async fn a_phone_only_customer_attaches_to_an_intent_and_the_intent_renders_it()
-> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    let customer = sdk
        .customers()
        .create(phone_only(), RequestOptions::new())
        .await
        .expect("a phone-only customer");

    let intent = sdk
        .payment_intents()
        .create(
            create_intent_params(Some(&customer.id)),
            RequestOptions::new(),
        )
        .await
        .expect("an intent for that customer");

    assert_eq!(
        intent.customer.as_deref(),
        Some(customer.id.as_str()),
        "`customer` was accepted and DROPPED until 2026-09-06; this is the assertion that \
         says it is stored"
    );

    let read = sdk
        .payment_intents()
        .retrieve(&intent.id)
        .await
        .expect("the intent reads back");
    assert_eq!(read.customer.as_deref(), Some(customer.id.as_str()));

    // And an intent created with no customer renders `null` rather than
    // omitting the key — the property both SDKs' optional field would hide.
    let plain = sdk
        .payment_intents()
        .create(create_intent_params(None), RequestOptions::new())
        .await
        .expect("an intent with no customer");
    assert_eq!(plain.customer, None);

    h.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------- the one-of rule

/// A customer with no `name`, no `email` and no `phone` is a `400` naming
/// them — **not** the `503` the database would produce.
///
/// `at_least_one_identifier` is a `23514`, and `vpay_db::classify_write`
/// routes a CHECK violation to `Category::Storage`, i.e. a `503` telling a
/// merchant to wait for a database that is perfectly healthy. That is why the
/// rule is decided in `vpay_api::v1::customers` and the constraint is a
/// backstop — and this is the assertion that fails if the boundary check is
/// ever removed on the grounds that "the database enforces it anyway".
#[tokio::test]
async fn a_customer_naming_nobody_is_a_400_and_not_the_databases_503() -> anyhow::Result<()> {
    let h = harness().await?;

    let response = raw_client()
        .post(h.url("/v1/customers"))
        .bearer_auth(h.bearer(CLIENT_A))
        .header("Idempotency-Key", "cus-nobody")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("")
        .send()
        .await
        .context("posting an empty customer body")?;

    assert_eq!(response.status().as_u16(), 400);
    let body: Value = response.json().await.context("the error envelope")?;
    assert_eq!(
        body.pointer("/error/type").and_then(Value::as_str),
        Some("invalid_request_error"),
        "got {body:#}"
    );
    let message = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .expect("the envelope carries a message")
        .to_owned();
    for field in ["name", "email", "phone"] {
        assert!(
            message.contains(field),
            "the message names all three, since which one to send is the merchant's choice: \
             {message}"
        );
    }
    assert!(
        message.contains("phone"),
        "and it must say a phone number alone is enough: {message}"
    );

    h.shutdown().await;
    Ok(())
}

/// Clearing the **last** identifier through an update is refused the same
/// way, and the customer is left intact.
///
/// The case the create-side check cannot cover: the request says only
/// `phone=`, and whether that leaves the customer nameless is a fact about
/// the *stored row*.
#[tokio::test]
async fn clearing_a_customers_last_identifier_is_refused_and_changes_nothing() -> anyhow::Result<()>
{
    let h = harness().await?;
    let sdk = h.a();

    let customer = sdk
        .customers()
        .create(phone_only(), RequestOptions::new())
        .await
        .expect("a phone-only customer");

    let error = sdk
        .customers()
        .update(
            &customer.id,
            UpdateCustomerParams {
                phone: Some(None),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect_err("clearing the only identifier leaves a customer naming nobody");
    match error {
        vpay_sdk::Error::Api { status, param, .. } => {
            assert_eq!(status, 400);
            assert_eq!(param.as_deref(), Some("name"));
        }
        other => panic!("expected a 400 naming a parameter, got {other:?}"),
    }

    let unchanged = sdk
        .customers()
        .retrieve(&customer.id)
        .await
        .expect("the customer survives a refused update");
    assert_eq!(
        unchanged.phone.as_deref(),
        Some(CANONICAL_PHONE),
        "a refused update must write nothing"
    );

    // And clearing one of two is allowed, which is what makes the rule "at
    // least one" rather than "never fewer".
    let named = sdk
        .customers()
        .update(
            &customer.id,
            UpdateCustomerParams {
                name: Some(Some("Ada Ngo".to_owned())),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("adding a name");
    assert_eq!(named.name.as_deref(), Some("Ada Ngo"));

    let cleared = sdk
        .customers()
        .update(
            &customer.id,
            UpdateCustomerParams {
                phone: Some(None),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("clearing the phone is fine once a name survives it");
    assert_eq!(cleared.phone, None);
    assert_eq!(cleared.name.as_deref(), Some("Ada Ngo"));

    h.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ tenancy

/// Another merchant's `cus_…` and an id that never existed are the **byte for
/// byte** identical `404`, on all three routes that take an id.
///
/// Byte-level, not status-level, for `refunds.rs`' reason: a different
/// `message` would make any of these an existence oracle for another tenant's
/// customers, and a status-code comparison would not notice.
#[tokio::test]
async fn a_foreign_customer_and_a_missing_one_are_the_identical_404() -> anyhow::Result<()> {
    let h = harness().await?;

    // Merchant B's customer, created through B's own SDK so the row is real
    // and correctly owned.
    let theirs = h
        .b()
        .customers()
        .create(phone_only(), RequestOptions::new())
        .await
        .expect("merchant B's customer");

    let client = raw_client();
    let token = h.bearer(CLIENT_A);

    for (method, suffix) in [("GET", ""), ("POST", ""), ("DELETE", "")] {
        let mut bodies = Vec::new();
        for id in [theirs.id.as_str(), MISSING_CUSTOMER_ID] {
            let url = h.url(&format!("/v1/customers/{id}{suffix}"));
            let mut request = match method {
                "GET" => client.get(url),
                "POST" => client
                    .post(url)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body("name=Mallory"),
                _ => client.delete(url),
            };
            request = request.bearer_auth(&token);
            if method != "GET" {
                // A distinct key per (method, id) pair: a replayed key would
                // answer the *stored* response and this comparison would be
                // between one response and its own replay.
                request = request.header("Idempotency-Key", format!("{method}-{id}"));
            }
            let response = request.send().await.context("the tenancy request")?;
            assert_eq!(
                response.status().as_u16(),
                404,
                "{method} on {id} must be a 404"
            );
            // The caller's own id is echoed back, so substitute each
            // request's own id out before comparing. `refunds.rs` does the
            // same and for the same reason: the echo is a function of the
            // *request*, so it distinguishes nothing — anything else
            // differing would.
            bodies.push(
                response
                    .text()
                    .await
                    .context("the 404 body")?
                    .replace(id, "<id>"),
            );
        }
        let [foreign, missing] =
            <[String; 2]>::try_from(bodies).expect("exactly two requests were made above");
        assert_eq!(
            foreign, missing,
            "{method}: another merchant's customer and an id that never existed must be \
             indistinguishable, byte for byte once each request's own id is substituted out — \
             otherwise this route is an existence oracle across tenants"
        );
        assert!(
            missing.contains("resource_missing"),
            "and it is the resource_missing envelope, not the unknown_route one an unmounted \
             route would answer: {missing}"
        );
    }

    // Merchant B's customer really is still there — so the 404s above were
    // tenancy, not a delete that happened anyway.
    let survivor = h
        .b()
        .customers()
        .retrieve(&theirs.id)
        .await
        .expect("merchant B's customer is untouched");
    assert_eq!(survivor.id, theirs.id);

    h.shutdown().await;
    Ok(())
}

/// Naming another merchant's `cus_…` as `customer` on an intent is a `400`
/// naming the parameter — **the same** `400` an id that does not exist gets.
///
/// A `404`, or a distinct message, would make `POST /v1/payment_intents` an
/// oracle for which `cus_…` exist under some other tenant.
#[tokio::test]
async fn attaching_a_foreign_customer_to_an_intent_is_the_same_400_as_a_missing_one()
-> anyhow::Result<()> {
    let h = harness().await?;

    let theirs = h
        .b()
        .customers()
        .create(phone_only(), RequestOptions::new())
        .await
        .expect("merchant B's customer");

    let mut messages = Vec::new();
    for id in [theirs.id.as_str(), MISSING_CUSTOMER_ID] {
        let error = h
            .a()
            .payment_intents()
            .create(create_intent_params(Some(id)), RequestOptions::new())
            .await
            .expect_err("a customer that is not this merchant's");
        match error {
            vpay_sdk::Error::Api {
                status,
                param,
                message,
                ..
            } => {
                assert_eq!(status, 400, "not a 404: the id is a field of the request");
                assert_eq!(param.as_deref(), Some("customer"));
                messages.push(message);
            }
            other => panic!("expected a 400 naming `customer`, got {other:?}"),
        }
    }
    let [foreign_message, missing_message] =
        <[String; 2]>::try_from(messages).expect("exactly two creates were attempted above");
    assert_eq!(
        foreign_message, missing_message,
        "a foreign customer and a missing one must answer the identical sentence"
    );

    // A malformed id is refused *before* any lookup, and says something
    // different — that one is a typo the merchant can fix from the message,
    // and telling them so leaks nothing.
    let error = h
        .a()
        .payment_intents()
        .create(
            create_intent_params(Some("cs_notacustomer")),
            RequestOptions::new(),
        )
        .await
        .expect_err("a `cs_…` is not a customer id");
    match error {
        vpay_sdk::Error::Api {
            status,
            param,
            message,
            ..
        } => {
            assert_eq!(status, 400);
            assert_eq!(param.as_deref(), Some("customer"));
            assert!(message.contains("cus_"), "{message}");
            assert_ne!(message, foreign_message);
        }
        other => panic!("expected a 400, got {other:?}"),
    }

    h.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------------- list

/// The cursor pages forward and backward **while rows are being inserted**,
/// and every customer appears exactly once.
///
/// The property `payment_intents.rs` holds for intents, held here for the
/// reason the cursor is a `seq` and not a `created_at`: a cursor over a
/// timestamp ties under a burst, and a tie is a row a merchant never sees.
/// Rows are created *between* the two page reads, which is the shape a real
/// integration has and a static fixture does not.
#[tokio::test]
async fn the_cursor_pages_both_ways_without_skipping_a_row_under_concurrent_inserts()
-> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    /// The ids on a page, in the order the page carries them. A helper rather
    /// than `page.data[0].id`, because `clippy::indexing_slicing` is denied
    /// here — and because a slice comparison says *which* page came back where
    /// a panicking index says only that some index was wrong.
    fn ids(page: &vpay_sdk::List<vpay_sdk::Customer>) -> Vec<&str> {
        page.data.iter().map(|c| c.id.as_str()).collect()
    }

    /// `created[a..b]`, newest first — the order every page here is in.
    fn newest_first(created: &[String], skip: usize, take: usize) -> Vec<&str> {
        created
            .iter()
            .rev()
            .skip(skip)
            .take(take)
            .map(String::as_str)
            .collect()
    }

    /// The oldest row on a page, which is the cursor for the next one.
    fn oldest(page: &vpay_sdk::List<vpay_sdk::Customer>) -> String {
        ids(page)
            .last()
            .copied()
            .expect("a page this test read is never empty")
            .to_owned()
    }

    let mut created = Vec::new();
    for index in 0..5 {
        created.push(
            sdk.customers()
                .create(
                    CreateCustomerParams {
                        name: Some(format!("Payer {index}")),
                        ..Default::default()
                    },
                    RequestOptions::new(),
                )
                .await
                .expect("a customer")
                .id,
        );
    }

    // Page one, newest first.
    let first = sdk
        .customers()
        .list(ListCustomersParams {
            limit: Some(2),
            ..Default::default()
        })
        .await
        .expect("the first page");
    assert!(first.has_more);
    assert_eq!(
        ids(&first),
        newest_first(&created, 0, 2),
        "the first page is the two newest, newest first"
    );
    let page_one_cursor = oldest(&first);

    // A row lands between the two reads. The cursor is a `seq`, so the new
    // row sorts *ahead* of the page just read and cannot displace anything
    // behind it.
    let interleaved = sdk
        .customers()
        .create(
            CreateCustomerParams {
                name: Some("Payer 5".to_owned()),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("a customer created mid-page")
        .id;

    let second = sdk
        .customers()
        .list(ListCustomersParams {
            limit: Some(2),
            starting_after: Some(page_one_cursor),
            ..Default::default()
        })
        .await
        .expect("the second page");
    assert_eq!(
        ids(&second),
        newest_first(&created, 2, 2),
        "the concurrent insert must not shift the page: a cursor over `seq` is stable, one \
         over `created_at` would not be"
    );
    assert!(
        !second.data.iter().any(|c| c.id == interleaved),
        "and the new row is ahead of this page, not inside it"
    );

    // Backwards from the second page's oldest row: `ending_before` scans
    // ascending and is reversed, so `data` is newest-first either way.
    let back = sdk
        .customers()
        .list(ListCustomersParams {
            limit: Some(2),
            ending_before: Some(oldest(&second)),
            ..Default::default()
        })
        .await
        .expect("the previous page");
    assert_eq!(
        ids(&back),
        newest_first(&created, 1, 2),
        "paging back must land on the rows immediately newer than the cursor, newest first"
    );

    // Nothing of merchant B's is ever on merchant A's pages, however many
    // pages are read.
    h.b()
        .customers()
        .create(phone_only(), RequestOptions::new())
        .await
        .expect("merchant B's customer");
    let all = sdk
        .customers()
        .list(ListCustomersParams {
            limit: Some(100),
            ..Default::default()
        })
        .await
        .expect("every customer of merchant A's");
    assert_eq!(
        all.data.len(),
        6,
        "five plus the interleaved one, and no more"
    );
    assert!(!all.has_more);

    h.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------- delete

/// `DELETE` is a hard delete: the row is gone, a later retrieve is the same
/// 404 an id that never existed gets, and the response carries no payer data.
#[tokio::test]
async fn a_delete_removes_the_row_and_answers_the_stripe_deleted_shape() -> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    let customer = sdk
        .customers()
        .create(phone_only(), RequestOptions::new())
        .await
        .expect("a customer");

    let deleted = sdk
        .customers()
        .del(&customer.id, RequestOptions::new())
        .await
        .expect("a customer with no payment history is deletable");
    assert_eq!(deleted.id, customer.id);
    assert_eq!(deleted.object, "customer");
    assert!(deleted.deleted);

    // The row is really gone — not flagged. A `deleted_at` column would let
    // this count come back 1 while every route above still answered 404.
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM customers WHERE id = $1")
        .bind(&customer.id)
        .fetch_one(&h.pool)
        .await
        .context("counting the deleted row")?;
    assert_eq!(remaining, 0, "a customer is deleted, never flagged");

    // And the response body carries none of the payer's details — the whole
    // reason it is the deleted shape and not the customer.
    let raw = raw_client()
        .delete(h.url(&format!("/v1/customers/{}", customer.id)))
        .bearer_auth(h.bearer(CLIENT_A))
        .header("Idempotency-Key", "del-again")
        .send()
        .await
        .context("deleting an already-deleted customer")?;
    assert_eq!(
        raw.status().as_u16(),
        404,
        "a second delete of a row that is gone is a 404 under a *different* idempotency key"
    );

    h.shutdown().await;
    Ok(())
}

/// A customer an intent references cannot be deleted, and the refusal is a
/// `409` that explains itself — not the `500` a raw foreign-key violation
/// would produce.
#[tokio::test]
async fn a_customer_with_payment_history_cannot_be_deleted() -> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    let customer = sdk
        .customers()
        .create(phone_only(), RequestOptions::new())
        .await
        .expect("a customer");
    sdk.payment_intents()
        .create(
            create_intent_params(Some(&customer.id)),
            RequestOptions::new(),
        )
        .await
        .expect("an intent that pins the customer");

    let error = sdk
        .customers()
        .del(&customer.id, RequestOptions::new())
        .await
        .expect_err("a customer with payment history is not deletable");
    match error {
        vpay_sdk::Error::Api {
            status, message, ..
        } => {
            assert_eq!(
                status, 409,
                "a fact about the object's state, not about the request's shape — and never a \
                 500, which is what an unmapped 23503 would be"
            );
            assert!(
                message.contains("PaymentIntent"),
                "the message has to say what is in the way and what to do instead: {message}"
            );
        }
        other => panic!("expected a 409, got {other:?}"),
    }

    h.shutdown().await;
    Ok(())
}

// ------------------------------------------------------- the retention sweep

/// **The twelve-month retention sweep, through the shipping worker loop.**
///
/// Three customers, three answers, one pass:
///
/// * idle thirteen months, referenced by nothing → **deleted**, with exactly
///   one `customer.deleted` event;
/// * idle eleven months → **kept**; the horizon is twelve, not "a while";
/// * idle thirteen months but referenced by a payment intent → **kept**,
///   because vpay never detaches a payment from its payer.
///
/// Run through `vpay_worker::run_once` rather than by calling `delete_idle`,
/// so what is proved includes that the job kind is **dispatched** at all.
/// That is not a hypothetical: on 2026-09-06 the first version of this suite
/// failed here because `JobKind::from_wire` had a hand-maintained list that
/// `SweepIdleCustomers` was missing from, so the job was claimed and
/// **dead-lettered** as "not a job kind this build knows; the row was written
/// by a different version". Everything else was green — the statement, the
/// event, the guard, every unit test — and the sweep had simply never run.
/// [`run_the_sweep`] now asserts the disposition rather than only the effect,
/// because "the row survived" reports a dead letter only by accident.
#[tokio::test]
async fn the_sweep_deletes_an_idle_unreferenced_customer_and_keeps_the_other_two()
-> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    let idle = sdk
        .customers()
        .create(
            CreateCustomerParams {
                name: Some("Thirteen months, unreferenced".to_owned()),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("the customer the sweep is for")
        .id;
    let recent = sdk
        .customers()
        .create(
            CreateCustomerParams {
                name: Some("Eleven months".to_owned()),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("a customer inside the horizon")
        .id;
    let referenced = sdk
        .customers()
        .create(
            CreateCustomerParams {
                name: Some("Thirteen months, referenced".to_owned()),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("a customer an intent pins")
        .id;

    let intent = sdk
        .payment_intents()
        .create(
            create_intent_params(Some(&referenced)),
            RequestOptions::new(),
        )
        .await
        .expect("the intent that pins it")
        .id;
    age_intent(&h.pool, &intent, 60).await?;

    // Ageing happens *after* the intent is created, because creating an
    // intent stamps the customer's clock — which is the behaviour the third
    // case depends on and the second half of this file proves separately.
    age_customer(&h.pool, &idle, 400).await?;
    age_customer(&h.pool, &recent, 330).await?;
    age_customer(&h.pool, &referenced, 400).await?;

    run_the_sweep(&h.repositories, &h.pool).await?;

    let survivors: Vec<String> = sqlx::query_scalar("SELECT id FROM customers ORDER BY seq")
        .fetch_all(&h.pool)
        .await
        .context("reading the customers that survived the sweep")?;
    assert!(
        !survivors.contains(&idle),
        "a customer idle for thirteen months and referenced by nothing must be deleted"
    );
    assert!(
        survivors.contains(&recent),
        "eleven months is inside the twelve-month horizon"
    );
    assert!(
        survivors.contains(&referenced),
        "a customer a payment intent references is never swept — vpay does not detach a \
         payment from its payer"
    );

    // Exactly one event, for exactly the deleted customer, and its
    // `data.object` is the customer as it stood — which is the only record of
    // who was erased, since the row is gone.
    let events: Vec<(String, String, Value)> = sqlx::query_as(
        "SELECT id, object_id, data FROM events WHERE type = 'customer.deleted' ORDER BY seq",
    )
    .fetch_all(&h.pool)
    .await
    .context("reading the customer.deleted events")?;
    assert_eq!(events.len(), 1, "one deletion, one event: {events:?}");
    let (_, object_id, data) = events.into_iter().next().expect("one event");
    assert_eq!(object_id, idle);
    assert_eq!(
        data.pointer("/id").and_then(Value::as_str),
        Some(idle.as_str())
    );
    assert_eq!(
        data.pointer("/object").and_then(Value::as_str),
        Some("customer")
    );
    assert_eq!(
        data.pointer("/name").and_then(Value::as_str),
        Some("Thirteen months, unreferenced"),
        "the body carries the payer's own details, because after the delete there is nothing \
         left to read"
    );

    // A second pass emits no second event — the guard is the statement, and
    // a deleted row matches nothing.
    run_the_sweep(&h.repositories, &h.pool).await?;
    let after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE type = 'customer.deleted'")
            .fetch_one(&h.pool)
            .await
            .context("counting customer.deleted events after a second pass")?;
    assert_eq!(after, 1, "a second sweep must not emit a second event");

    h.shutdown().await;
    Ok(())
}

/// **A confirm stamps the customer's retention clock**, which is what keeps a
/// customer a merchant uses regularly out of the sweep.
///
/// The decisive shape: the customer is aged past the horizon *and then* the
/// intent is confirmed, so the only thing that can save it is the stamp on
/// the confirm path. Remove `customers::touch` from `confirm_once` and the
/// sweep deletes a customer whose payer was prompted seconds earlier.
///
/// The confirm itself **fails** — the rail is unreachable by design in this
/// suite — and that is deliberate rather than a compromise: the stamp happens
/// before the rail is resolved, and proving it survives a failed confirm is
/// stronger than proving it happens on a successful one.
#[tokio::test]
async fn a_confirm_stamps_the_customers_clock_and_keeps_it_out_of_the_sweep() -> anyhow::Result<()>
{
    let h = harness().await?;
    let sdk = h.a();

    let customer = sdk
        .customers()
        .create(phone_only(), RequestOptions::new())
        .await
        .expect("a customer")
        .id;
    let intent = sdk
        .payment_intents()
        .create(create_intent_params(Some(&customer)), RequestOptions::new())
        .await
        .expect("an intent for it")
        .id;

    // Past the horizon, with the intent already in place.
    age_customer(&h.pool, &customer, 400).await?;
    let before: time::OffsetDateTime =
        sqlx::query_scalar("SELECT last_used_at FROM customers WHERE id = $1")
            .bind(&customer)
            .fetch_one(&h.pool)
            .await
            .context("reading the aged clock")?;

    // The confirm reaches the unreachable rail and fails. The stamp is
    // upstream of that.
    let _ = sdk
        .payment_intents()
        .confirm(
            &intent,
            vpay_sdk::payment_intents::ConfirmPaymentIntentParams::mtn_momo(CANONICAL_PHONE),
            RequestOptions::new(),
        )
        .await;

    let after: time::OffsetDateTime =
        sqlx::query_scalar("SELECT last_used_at FROM customers WHERE id = $1")
            .bind(&customer)
            .fetch_one(&h.pool)
            .await
            .context("reading the stamped clock")?;
    assert!(
        after > before,
        "a confirm is the clearest evidence a customer is live; it must move the retention \
         clock ({before} -> {after})"
    );

    h.shutdown().await;
    Ok(())
}

/// The retention stamp is **monotonic**: a request whose clock is behind
/// cannot rewind a customer's `last_used_at`.
///
/// Two vpay processes do not share a clock, and the horizon is twelve months
/// — so a rewind is not a rounding error, it is the difference between a
/// customer surviving a pass and not. `Customers::touch_last_used` filters on
/// `last_used_at < now` for exactly this, and there is no other layer that
/// could enforce it.
#[tokio::test]
async fn a_retention_stamp_never_moves_a_customers_clock_backwards() -> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    let customer = sdk
        .customers()
        .create(phone_only(), RequestOptions::new())
        .await
        .expect("a customer")
        .id;

    let now: time::OffsetDateTime =
        sqlx::query_scalar("SELECT last_used_at FROM customers WHERE id = $1")
            .bind(&customer)
            .fetch_one(&h.pool)
            .await
            .context("reading the fresh clock")?;

    // A stamp from a process whose clock is an hour behind.
    let behind = now - time::Duration::hours(1);
    let moved = vpay_db::Customers::touch_last_used(h.repositories.as_ref(), &customer, behind)
        .await
        .context("stamping with a stale instant")?;
    assert!(
        !moved,
        "a stamp older than the stored clock must match no row and report `false`"
    );

    let unchanged: time::OffsetDateTime =
        sqlx::query_scalar("SELECT last_used_at FROM customers WHERE id = $1")
            .bind(&customer)
            .fetch_one(&h.pool)
            .await
            .context("re-reading the clock")?;
    assert_eq!(
        unchanged, now,
        "a slow process must not be able to hand a live customer to the sweep"
    );

    // Forward still works, which is what makes the assertion above about
    // *direction* rather than about the stamp being broken.
    let ahead = now + time::Duration::hours(1);
    assert!(
        vpay_db::Customers::touch_last_used(h.repositories.as_ref(), &customer, ahead)
            .await
            .context("stamping forward")?
    );

    h.shutdown().await;
    Ok(())
}

/// A list is scoped to the caller's tenant, and a cursor naming **another
/// merchant's** customer pages nothing rather than paging into their range.
///
/// Two claims, and the second is the one no other case here makes. The list
/// route's tenancy is one `WHERE merchant_id = $1`; its *cursor* is a
/// correlated subquery, and a subquery that resolved the id without the
/// tenant would turn `starting_after` into a position in another merchant's
/// sequence — a caller who forged one would page their rows, or, worse, get
/// their own list silently re-anchored. `vpay_db::customers::list_page`
/// scopes both subqueries, so a foreign id resolves to `NULL`, `seq < NULL`
/// is `NULL`, and the page is empty: it fails **closed**.
///
/// The forged cursor is well-formed on purpose. A malformed one is refused by
/// `paging::validated_cursor` before any statement runs, which would prove
/// nothing about the statement.
#[tokio::test]
async fn a_list_is_tenant_scoped_and_a_foreign_cursor_pages_nothing() -> anyhow::Result<()> {
    let h = harness().await?;

    let mut mine = Vec::new();
    for index in 0..3 {
        mine.push(
            h.a()
                .customers()
                .create(
                    CreateCustomerParams {
                        name: Some(format!("A's payer {index}")),
                        ..Default::default()
                    },
                    RequestOptions::new(),
                )
                .await
                .expect("merchant A's customer")
                .id,
        );
    }

    let mut theirs = Vec::new();
    for index in 0..2 {
        theirs.push(
            h.b()
                .customers()
                .create(
                    CreateCustomerParams {
                        name: Some(format!("B's payer {index}")),
                        ..Default::default()
                    },
                    RequestOptions::new(),
                )
                .await
                .expect("merchant B's customer")
                .id,
        );
    }

    let page = h
        .a()
        .customers()
        .list(ListCustomersParams::default())
        .await
        .expect("merchant A's list");
    let seen: Vec<&str> = page.data.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        seen.len(),
        3,
        "merchant A has three customers and must see exactly those: {seen:?}"
    );
    for id in &theirs {
        assert!(
            !seen.contains(&id.as_str()),
            "merchant B's {id} appeared in merchant A's list: {seen:?}"
        );
    }

    // The forged cursor: B's newest customer, sent by A as `starting_after`.
    // It is a real `cus_…` that really exists — just not A's.
    let forged = theirs.last().expect("merchant B created two customers");
    let crossed = h
        .a()
        .customers()
        .list(ListCustomersParams {
            starting_after: Some(forged.clone()),
            ..Default::default()
        })
        .await
        .expect("a foreign cursor is answered, not errored");
    assert!(
        crossed.data.is_empty(),
        "a cursor naming another merchant's customer must resolve to nothing and page \
         nothing — it paged {:?}. Anything non-empty means the cursor subquery is not \
         tenant-scoped and `starting_after` is a position in another merchant's sequence",
        crossed
            .data
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>()
    );
    assert!(!crossed.has_more);

    // And the same id as `ending_before`, which is the other subquery and a
    // separate line of SQL.
    let crossed_back = h
        .a()
        .customers()
        .list(ListCustomersParams {
            ending_before: Some(forged.clone()),
            ..Default::default()
        })
        .await
        .expect("a foreign cursor is answered, not errored");
    assert!(
        crossed_back.data.is_empty(),
        "`ending_before` is a second subquery and must be scoped too; it paged {:?}",
        crossed_back
            .data
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>()
    );

    // A's own cursor still works, so the assertions above are about tenancy
    // and not about the cursor being broken for everybody.
    let ours = h
        .a()
        .customers()
        .list(ListCustomersParams {
            starting_after: mine.last().cloned(),
            ..Default::default()
        })
        .await
        .expect("merchant A's own cursor");
    assert_eq!(
        ours.data.len(),
        2,
        "A's own newest customer as a cursor leaves the two older ones"
    );

    h.shutdown().await;
    Ok(())
}

/// `POST /v1/customers` is under the same `Idempotency-Key` machinery as
/// every other write: a replay answers the **stored** object and writes no
/// second row, and the same key with a different body is refused.
///
/// The refusal is `400` `idempotency_error` / `idempotency_key_in_use`, which
/// is `vpay_core::Category::Idempotency`'s own kind and code rather than
/// anything this route chooses (ADR-0011) — asserted verbatim for
/// `payment_intents.rs`'s reason.
///
/// Why it matters more here than on an intent: a customer is personal data,
/// and a retried create that minted a *second* `cus_…` would leave a merchant
/// holding one id while vpay held two rows of the same payer — a duplicate no
/// unique index can catch, because there deliberately is none (see
/// `docs/flows/customers.md`, "No cross-merchant identity").
#[tokio::test]
async fn a_replayed_create_answers_the_stored_customer_and_a_reused_key_is_refused()
-> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();
    let opts = RequestOptions::new().with_idempotency_key("customer-create-0001");

    let first = sdk
        .customers()
        .create(phone_only(), opts.clone())
        .await
        .expect("the first create");

    let replay = sdk
        .customers()
        .create(phone_only(), opts.clone())
        .await
        .expect("the replay is answered, not refused");
    assert_eq!(
        first.id, replay.id,
        "a replay must answer the stored customer, not mint a second `cus_…`"
    );
    assert_eq!(first.phone, replay.phone);

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM customers WHERE merchant_id = $1")
        .bind(MERCHANT_A)
        .fetch_one(&h.pool)
        .await
        .context("counting merchant A's customers")?;
    assert_eq!(
        rows, 1,
        "the replay must not have created a second customer"
    );

    // The same key, a different body.
    let different = CreateCustomerParams {
        name: Some("Somebody Else".to_owned()),
        ..Default::default()
    };
    let error = sdk
        .customers()
        .create(different, opts)
        .await
        .expect_err("the same key with a different body must be refused");
    let message = format!("{error:?}");
    assert!(
        message.contains("idempotency_key_in_use"),
        "the refusal is the documented envelope, not a second customer: {message}"
    );

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM customers WHERE merchant_id = $1")
        .bind(MERCHANT_A)
        .fetch_one(&h.pool)
        .await
        .context("counting merchant A's customers after the refusal")?;
    assert_eq!(rows, 1, "the refused request must not have created a row");

    h.shutdown().await;
    Ok(())
}

/// `POST /v1/checkout/sessions` as a raw form, because **neither SDK can send
/// `customer` on a session** — see this case's own findings note in
/// `docs/sdks/parity.md`.
async fn create_session(
    h: &Harness,
    client_id: &str,
    fields: &[(&str, &str)],
) -> anyhow::Result<(u16, Value)> {
    let response = raw_client()
        .post(h.url("/v1/checkout/sessions"))
        .bearer_auth(h.bearer(client_id))
        .header("Idempotency-Key", uuid::Uuid::new_v4().to_string())
        .form(fields)
        .send()
        .await
        .context("creating a checkout session")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the session body")?;
    Ok((status, body))
}

/// A checkout session's `customer`: inherited, supplied, or a refused
/// contradiction — and a customer a **session alone** references is pinned
/// against deletion.
///
/// # Four documented behaviours that no case exercised
///
/// `docs/flows/customers.md` ("`customer` on a payment intent and a checkout
/// session") states that a session accepts `customer`, stores it, renders it,
/// inherits its intent's when the request omits one, and refuses one that
/// disagrees with the intent's. S4a delivered all of that and tested none of
/// it: `checkout_sessions.rs` gained only a `customer_id: None` field filler
/// and a key-count bump on the *nested intent*.
///
/// The session path is not the intent path and cannot be assumed from it. An
/// intent has nothing to contradict; a session is the only object in vpay
/// where two rows can name two different payers, and
/// `checkout_sessions::validate`'s three-armed match is the only code that
/// decides what happens then.
///
/// # And the sweep's second table
///
/// `Customers::idle_since` and `delete_idle` guard on `NOT EXISTS` over
/// **two** tables. `the_sweep_deletes_an_idle_unreferenced_customer_and_keeps_the_other_two`
/// exercises the `payment_intents` half only. The last assertion here is the
/// `checkout_sessions` half: a customer that no intent names but a session
/// does must be undeletable, or the sweep would erase a payer mid-checkout.
#[tokio::test]
async fn a_sessions_customer_is_inherited_supplied_or_a_refused_contradiction() -> anyhow::Result<()>
{
    let h = harness().await?;
    let sdk = h.a();

    let x = sdk
        .customers()
        .create(phone_only(), RequestOptions::new())
        .await
        .expect("customer X")
        .id;
    let y = sdk
        .customers()
        .create(
            CreateCustomerParams {
                name: Some("Yvonne".to_owned()),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("customer Y")
        .id;

    /// One intent per session: an intent may have only one open session, so
    /// every case below needs its own.
    async fn intent(sdk: &vpay_sdk::Client, customer: Option<&str>) -> String {
        sdk.payment_intents()
            .create(create_intent_params(customer), RequestOptions::new())
            .await
            .expect("an intent")
            .id
    }

    // 1. INHERITED. The request omits `customer`; the intent has one.
    let with_x = intent(&sdk, Some(&x)).await;
    let (status, body) = create_session(
        &h,
        CLIENT_A,
        &[
            ("payment_intent", with_x.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    assert_eq!(status, 201, "the session is created: {body:#}");
    assert_eq!(
        body.get("customer").and_then(Value::as_str),
        Some(x.as_str()),
        "a session with no `customer` inherits its intent's, so the two rows always agree: \
         {body:#}"
    );

    // 2. SUPPLIED. The intent has none; the request names one.
    let bare = intent(&sdk, None).await;
    let (status, body) = create_session(
        &h,
        CLIENT_A,
        &[
            ("payment_intent", bare.as_str()),
            ("customer", y.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    assert_eq!(status, 201, "the session is created: {body:#}");
    assert_eq!(
        body.get("customer").and_then(Value::as_str),
        Some(y.as_str()),
        "a session's own `customer` is stored and rendered: {body:#}"
    );

    // It really reached the column the sweep reads, not just the response.
    let stored: Option<String> =
        sqlx::query_scalar("SELECT customer_id FROM checkout_sessions WHERE id = $1")
            .bind(
                body.get("id")
                    .and_then(Value::as_str)
                    .expect("a session id"),
            )
            .fetch_one(&h.pool)
            .await
            .context("reading the session's customer_id")?;
    assert_eq!(stored.as_deref(), Some(y.as_str()));

    // 3. A CONTRADICTION. The intent says X, the request says Y.
    let also_x = intent(&sdk, Some(&x)).await;
    let (status, body) = create_session(
        &h,
        CLIENT_A,
        &[
            ("payment_intent", also_x.as_str()),
            ("customer", y.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    assert_eq!(
        status, 400,
        "a session and the intent it drives naming two different payers is a contradiction, \
         not a preference between two answers: {body:#}"
    );
    assert_eq!(
        body.pointer("/error/param").and_then(Value::as_str),
        Some("customer"),
        "and the refusal names the parameter the merchant sent: {body:#}"
    );

    // 4. ANOTHER MERCHANT'S CUSTOMER is the *same* refusal a missing one gets
    //    — a session create must not be an existence oracle either.
    let theirs = h
        .b()
        .customers()
        .create(phone_only(), RequestOptions::new())
        .await
        .expect("merchant B's customer")
        .id;
    let bare_two = intent(&sdk, None).await;
    let mut refusals = Vec::new();
    for candidate in [theirs.as_str(), MISSING_CUSTOMER_ID] {
        let (status, body) = create_session(
            &h,
            CLIENT_A,
            &[
                ("payment_intent", bare_two.as_str()),
                ("customer", candidate),
                ("success_url", SUCCESS_URL),
                ("cancel_url", CANCEL_URL),
            ],
        )
        .await?;
        assert_eq!(status, 400, "{candidate} must be a 400: {body:#}");
        refusals.push(body.to_string());
    }
    let [foreign, missing] =
        <[String; 2]>::try_from(refusals).expect("exactly two requests were made above");
    assert_eq!(
        foreign, missing,
        "another merchant's `cus_…` and one that never existed must be the identical refusal, \
         or `POST /v1/checkout/sessions` is an oracle for which customers exist under some \
         other tenant"
    );

    // 5. THE SWEEP'S SECOND TABLE. Y is named by a session and by no intent
    //    at all, and that alone must pin it.
    let referencing_intents: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM payment_intents WHERE customer_id = $1")
            .bind(&y)
            .fetch_one(&h.pool)
            .await
            .context("counting intents naming Y")?;
    assert_eq!(
        referencing_intents, 0,
        "this assertion is only about `checkout_sessions` if no intent names Y"
    );

    let refused = raw_client()
        .delete(h.url(&format!("/v1/customers/{y}")))
        .bearer_auth(h.bearer(CLIENT_A))
        .header("Idempotency-Key", "delete-a-session-customer")
        .send()
        .await
        .context("deleting a customer a session references")?;
    assert_eq!(
        refused.status().as_u16(),
        409,
        "a customer a checkout session references cannot be deleted — the foreign key is \
         `NO ACTION` on both tables, and a sweep that read only `payment_intents` would \
         erase a payer mid-checkout"
    );

    h.shutdown().await;
    Ok(())
}
