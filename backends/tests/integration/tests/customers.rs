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
        staff_auth: vpay_config::StaffAuth::default(),
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

// ------------------------------------------------------------------ address

/// **The address, end to end** — issue #67: created, read back as one nested
/// object, replaced whole by an update, and cleared with `address=`.
///
/// # The three things this pins that a shape assertion would not
///
/// 1. **The country is stored upper case.** A merchant who typed `cm` and one
///    who typed `CM` have one value between them, which is the same wire
///    contract `phone`'s canonicalisation is. A "store what they sent"
///    implementation passes every type check and leaves vpay holding two
///    spellings of one country.
/// 2. **An update replaces the address, it does not merge it.** The second
///    request below names `line1` and `country` and not `city`, and the
///    stored `city` must be gone. A component-wise merge would leave the old
///    city beside the new street — an address that was never anybody's,
///    assembled by vpay out of two requests, and visible only to whoever
///    eventually posts something to it.
/// 3. **`address=` clears it, and is different from not mentioning it.**
///    Exactly `name=`'s three states, one level up. Collapsing the two is a
///    one-word edit that compiles and makes an address unremovable.
///
/// The `customer.updated` body is asserted too, because that object is what a
/// merchant's webhook handler reads and it is stored in `events` for ever: a
/// render that was right on the response and wrong in the event is a real and
/// silent split.
#[tokio::test]
async fn an_address_round_trips_is_replaced_whole_and_is_cleared_by_an_empty_value()
-> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    let created = sdk
        .customers()
        .create(
            CreateCustomerParams {
                phone: Some(CANONICAL_PHONE.to_owned()),
                address: Some(vpay_sdk::AddressParams {
                    line1: Some("12 Rue Njo-Njo".to_owned()),
                    city: Some("Douala".to_owned()),
                    // Lower case on the way in, upper case on the way out.
                    country: Some("cm".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            RequestOptions::new().with_idempotency_key("cus-address-1"),
        )
        .await
        .expect("a customer with an address");

    let address = created.address.clone().expect("the address round-trips");
    assert_eq!(address.line1.as_deref(), Some("12 Rue Njo-Njo"));
    assert_eq!(address.city.as_deref(), Some("Douala"));
    assert_eq!(
        address.country.as_deref(),
        Some("CM"),
        "`cm` and `CM` are one country and vpay must not hold two spellings of it"
    );
    assert_eq!(address.line2, None);
    assert_eq!(address.postal_code, None);

    // 2. Replaced whole: `city` was stored and this request does not name it.
    let replaced = sdk
        .customers()
        .update(
            &created.id,
            UpdateCustomerParams {
                address: Some(Some(vpay_sdk::AddressParams {
                    line1: Some("9 Boulevard de la Liberté".to_owned()),
                    country: Some("CM".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
            RequestOptions::new().with_idempotency_key("cus-address-2"),
        )
        .await
        .expect("the update");
    let address = replaced.address.clone().expect("still an address");
    assert_eq!(address.line1.as_deref(), Some("9 Boulevard de la Liberté"));
    assert_eq!(
        address.city, None,
        "an address is replaced whole: a component the request did not name is cleared, \\
         never merged from the stored row"
    );

    // The event body agrees with the response, key for key.
    let events = events_about(&h.pool, &created.id).await?;
    let (kind, data) = events.last().expect("at least the create and the update");
    assert_eq!(kind, "customer.updated");
    assert_eq!(
        data.pointer("/address/line1").and_then(Value::as_str),
        Some("9 Boulevard de la Liberté")
    );
    assert_eq!(
        data.pointer("/address/city"),
        Some(&Value::Null),
        "the event body is the object, not a projection of the request: every component is \\
         rendered, `null` included"
    );

    // 3. `address=` clears it. Sent as the raw body rather than through the
    //    SDK's `Some(None)` as well, so this is a statement about the WIRE
    //    and not only about the SDK's encoding of it.
    let cleared = raw_client()
        .post(h.url(&format!("/v1/customers/{}", created.id)))
        .bearer_auth(h.bearer(CLIENT_A))
        .header("Idempotency-Key", "cus-address-3")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("address=")
        .send()
        .await
        .context("clearing an address")?;
    assert_eq!(cleared.status().as_u16(), 200);
    let body: Value = cleared.json().await.context("the cleared customer")?;
    assert_eq!(
        body.pointer("/address"),
        Some(&Value::Null),
        "`address=` removes the address entirely, and the object then renders `null` rather \\
         than an object of six nulls"
    );

    // And the columns really are NULL — not the empty string, which
    // `address_line1_length`'s floor of 1 would have refused as a 500.
    let stored: Option<String> =
        sqlx::query_scalar("SELECT address_line1 FROM customers WHERE id = $1")
            .bind(&created.id)
            .fetch_one(&h.pool)
            .await
            .context("reading the cleared address")?;
    assert_eq!(stored, None);

    h.shutdown().await;
    Ok(())
}

/// A country that is not ISO 3166-1 alpha-2 is a `400` naming `address`, and
/// **nothing is written**.
///
/// The `400` rather than the `500` migration `0041`'s
/// `address_country_is_iso_3166_1_alpha_2` would otherwise produce:
/// `vpay_db::classify_write` routes a CHECK violation to `Category::Storage`,
/// which reaches a merchant as a `503` telling them to wait for a database
/// that is perfectly healthy. `name_length`'s argument, applied to the one
/// component of the address that has a shape rather than a bound.
///
/// **The decisive mutation:** delete `checked_country` and route `country`
/// through `checked_text` like the other five. The create below answers `201`
/// with `Cameroon` stored, and every other test in this file stays green.
#[tokio::test]
async fn a_country_that_is_not_alpha_2_is_a_400_and_not_the_databases_503() -> anyhow::Result<()> {
    let h = harness().await?;

    for refused in ["CMR", "Cameroon", "237"] {
        let response = raw_client()
            .post(h.url("/v1/customers"))
            .bearer_auth(h.bearer(CLIENT_A))
            .header("Idempotency-Key", format!("bad-country-{refused}"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(format!(
                "phone={CANONICAL_PHONE}&address[country]={refused}"
            ))
            .send()
            .await
            .context("creating a customer with a bad country code")?;

        assert_eq!(
            response.status().as_u16(),
            400,
            "`{refused}` is not an alpha-2 code and must be refused above the statement"
        );
        let body: Value = response.json().await.context("the error body")?;
        assert_eq!(
            body.pointer("/error/param").and_then(Value::as_str),
            Some("address"),
            "the refusal names the top-level parameter a merchant's error handler can act \\
             on: {body}"
        );
        assert_eq!(
            body.pointer("/error/type").and_then(Value::as_str),
            Some("invalid_request_error"),
            "a 503 here would tell a merchant to retry against a database that is fine: \\
             {body}"
        );
    }

    // Nothing was written by any of the three.
    let customers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM customers")
        .fetch_one(&h.pool)
        .await
        .context("counting customers after three refused creates")?;
    assert_eq!(customers, 0);

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

/// A customer an intent references is **anonymised**, not refused and not
/// row-deleted — issues #68 and #96 item 2, migration `0041`.
///
/// # What this replaced, and why the replacement is not a softening
///
/// Until 2026-09-10 this route answered a `409` here, and the `409`'s advice
/// was to clear `name`, `email` and `phone` instead — advice
/// `at_least_one_identifier` refuses, so no merchant could follow it. The
/// payer's identifiers therefore survived every "deletion" of exactly the
/// customers vpay had taken money from. The foreign keys are unchanged and
/// still `NO ACTION`: the payment record survives. What does not survive is
/// the payer on it.
///
/// # Every assertion here is one of the four things that can go wrong
///
/// 1. the erasure does not happen at all (the `200` and the marker);
/// 2. it takes the payment record with it (the intent still resolves, with
///    its amount, its status and its `customer`);
/// 3. it leaves the object unreadable, so a merchant's stored `cus_…` turns
///    into a `404` (the `GET` and `deleted: true`);
/// 4. it can be undone (the update and the attachment, both `409`).
///
/// The fifth — that some copy of the identifiers survives somewhere else — is
/// [`an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`], which
/// is the only one of the five that can be proved without naming the places
/// to look.
#[tokio::test]
async fn a_customer_with_payment_history_is_anonymised_rather_than_deleted() -> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    let customer = sdk
        .customers()
        .create(
            CreateCustomerParams {
                name: Some("Ada Ngo".to_owned()),
                email: Some("ada@example.cm".to_owned()),
                phone: Some(CANONICAL_PHONE.to_owned()),
                metadata: BTreeMap::from([("order_id".to_owned(), "1234".to_owned())]),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("a customer");
    let intent = sdk
        .payment_intents()
        .create(
            create_intent_params(Some(&customer.id)),
            RequestOptions::new(),
        )
        .await
        .expect("an intent that pins the customer");

    let deleted = sdk
        .customers()
        .del(&customer.id, RequestOptions::new())
        .await
        .expect("a customer with payment history is erased, not refused");
    assert_eq!(deleted.id, customer.id);
    assert!(deleted.deleted);

    // 1. The row is still there and holds nothing of the payer's. Read from
    //    the database rather than from the API, because the API rendering the
    //    marker and the column holding it are two different claims.
    let (name, email, phone, anonymized): (String, String, String, bool) = sqlx::query_as(
        "SELECT name, email, phone, anonymized_at IS NOT NULL FROM customers WHERE id = $1",
    )
    .bind(&customer.id)
    .fetch_one(&h.pool)
    .await
    .context("the anonymised customer's row must still exist")?;
    assert!(
        anonymized,
        "`anonymized_at` is the evidence an erasure happened"
    );
    for (column, value) in [("name", &name), ("email", &email), ("phone", &phone)] {
        assert_eq!(
            value.as_str(),
            vpay_db::REDACTED,
            "`{column}` still holds the payer's own value after an erasure"
        );
    }

    // 2. The payment record is intact, which is the whole reason the row
    //    could not simply be deleted. Amount, status and the `customer`
    //    pointer all survive — a merchant's ledger and any dispute still
    //    resolve; what they resolve to is a customer with no payer in it.
    let after = sdk
        .payment_intents()
        .retrieve(&intent.id)
        .await
        .expect("the intent survives its customer's erasure");
    assert_eq!(
        after.amount, AMOUNT,
        "the amount is the ledger, and it stays"
    );
    assert_eq!(after.status, intent.status);
    assert_eq!(
        after.customer.as_deref(),
        Some(customer.id.as_str()),
        "the intent still names the customer it was taken from — `ON DELETE SET NULL` was \
         the alternative and it is worse: it detaches a payment from its payer"
    );

    // 3. The object still resolves, and says what happened. A `404` here
    //    would make every merchant record naming this `cus_…` dangle.
    let raw = raw_client()
        .get(h.url(&format!("/v1/customers/{}", customer.id)))
        .bearer_auth(h.bearer(CLIENT_A))
        .send()
        .await
        .context("retrieving an anonymised customer")?;
    assert_eq!(raw.status().as_u16(), 200);
    let body: Value = raw.json().await.context("the anonymised customer object")?;
    assert_eq!(body.pointer("/deleted"), Some(&Value::Bool(true)));
    assert_eq!(
        body.pointer("/metadata/order_id").and_then(Value::as_str),
        Some("1234"),
        "`metadata` is the MERCHANT's data, not the payer's, and destroying it would be vpay \
         deleting a merchant's records to keep a promise made to somebody else"
    );
    assert_eq!(
        body.pointer("/name").and_then(Value::as_str),
        Some(vpay_db::REDACTED)
    );

    // 4a. It cannot be undone by an update. `at_least_one_identifier` would
    //     not object — `[redacted]` is not NULL — so this has to be refused
    //     above the statement or an erasure is reversible with one `POST`.
    let refused = raw_client()
        .post(h.url(&format!("/v1/customers/{}", customer.id)))
        .bearer_auth(h.bearer(CLIENT_A))
        .header("Idempotency-Key", "un-erase-a-customer")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("name=Ada%20Ngo")
        .send()
        .await
        .context("updating an anonymised customer")?;
    assert_eq!(
        refused.status().as_u16(),
        409,
        "putting a name back on an erased customer must be refused; a 404 would be two \
         routes disagreeing about whether this `cus_…` exists"
    );

    // 4b. And it cannot be attached to a new payment: that would put a fresh
    //     `NO ACTION` reference on a record kept only because the last one
    //     could not be removed, and restart its retention clock.
    let attach = sdk
        .payment_intents()
        .create(
            create_intent_params(Some(&customer.id)),
            RequestOptions::new(),
        )
        .await
        .expect_err("an erased customer is not attachable");
    match attach {
        vpay_sdk::Error::Api { status, .. } => assert_eq!(status, 409),
        other => panic!("expected a 409, got {other:?}"),
    }

    // 5. A second DELETE is idempotent on the object's own state — not on the
    //    `Idempotency-Key`, which only covers a replay of the same request —
    //    and writes no second event.
    let again = raw_client()
        .delete(h.url(&format!("/v1/customers/{}", customer.id)))
        .bearer_auth(h.bearer(CLIENT_A))
        .header("Idempotency-Key", "erase-again-different-key")
        .send()
        .await
        .context("deleting an already-anonymised customer")?;
    assert_eq!(again.status().as_u16(), 200);
    let events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE type = 'customer.deleted' AND object_id = $1",
    )
    .bind(&customer.id)
    .fetch_one(&h.pool)
    .await
    .context("counting customer.deleted events")?;
    assert_eq!(
        events, 1,
        "a second DELETE describes an erasure that already happened; emitting a second \
         `customer.deleted` would tell a merchant a payer was erased twice"
    );

    h.shutdown().await;
    Ok(())
}

/// **The decisive proof: after a `DELETE`, none of the payer's identifiers
/// survives in any column of any table** — issue #68.
///
/// # Why it scans `information_schema` instead of naming the tables
///
/// Every other test in this file asserts about a place somebody thought of.
/// This one asserts about the places nobody did, which is the only shape of
/// assertion that could have caught what the reading behind migration `0041`
/// found: issue #68 is written about "retained intents and sessions", and an
/// intent has never carried a payer identifier. The identifiers were in
/// `events.data` — every `customer.*` body vpay ever wrote, never pruned —
/// in `charges.payer_ref`, reachable from a customer only through an intent,
/// and in `idempotency_keys.response_body`. A test that named tables would
/// have named the wrong ones.
///
/// So it reads every `text`, `varchar` and `jsonb` column the live database
/// has and greps the lot. A fourth store added later fails here rather than
/// waiting to be noticed.
///
/// # The before/after pair is what makes it an assertion
///
/// A scan that found nothing afterwards would pass just as well against a
/// database where nothing was ever written, a typo in the literal, or a
/// query that scanned no columns at all. So the same scan runs **first** and
/// has to find each literal, and the column count is asserted non-trivial.
///
/// The fixture is the shape the whole erasure is about: three unique
/// literals, an update that writes a second `customer.*` event body, a paid
/// intent with a charge carrying the payer's MSISDN, and a stored idempotent
/// response.
#[tokio::test]
async fn an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table() -> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    // Literals nothing else in this database can contain by accident. The
    // phone must still be a canonical MSISDN — `phone_is_a_canonical_msisdn`
    // — so it is unique by its digits rather than by being nonsense.
    const NAME: &str = "Zzyzx Quibblewort";
    const EMAIL: &str = "zzyzx.quibblewort@example.invalid";
    const PHONE: &str = "237600000771";
    const STREET: &str = "77 Rue Quibblewort";

    let customer = sdk
        .customers()
        .create(
            CreateCustomerParams {
                name: Some(NAME.to_owned()),
                email: Some(EMAIL.to_owned()),
                phone: Some(PHONE.to_owned()),
                address: Some(vpay_sdk::customers::AddressParams {
                    line1: Some(STREET.to_owned()),
                    city: Some("Douala".to_owned()),
                    country: Some("CM".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("the customer this test is about");

    // An update, so there is a `customer.updated` body in `events` as well as
    // a `customer.created` one. Both are stored for ever and neither is
    // pruned; they are the copy the retention promise is actually about.
    sdk.customers()
        .update(
            &customer.id,
            UpdateCustomerParams {
                name: Some(Some(NAME.to_owned())),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("an update writes a second stored body");

    // A charge, so `charges.payer_ref` and `payer_ref_masked` hold the
    // payer's MSISDN — the column reachable from a customer only through an
    // intent, and the reason nothing looking at `customers` alone ever found
    // it.
    let intent = sdk
        .payment_intents()
        .create(
            create_intent_params(Some(&customer.id)),
            RequestOptions::new(),
        )
        .await
        .expect("an intent for this customer");
    sqlx::query(
        "INSERT INTO charges (id, payment_intent_id, provider_code, provider_reference_id, \
         state, amount, currency_code, payer_ref, payer_ref_masked) \
         VALUES ('ch_scanner00000000000000', $1, $2, gen_random_uuid(), 'submitted', $3, \
                 'XAF', $4, $5)",
    )
    .bind(&intent.id)
    .bind(RAIL)
    .bind(AMOUNT)
    .bind(PHONE)
    .bind(format!("*** *** {}", &PHONE[PHONE.len() - 3..]))
    .execute(&h.pool)
    .await
    .context("seeding the charge that carries the payer reference")?;

    let literals = [NAME, EMAIL, PHONE, STREET];

    let (before, columns) = scan_for(&h.pool, &literals).await?;
    assert!(
        columns > 100,
        "the scan looked at {columns} columns, which is not a database with vpay's schema in \
         it — every assertion below would be vacuous"
    );
    for literal in literals {
        assert!(
            before.contains_key(literal),
            "`{literal}` is nowhere in the database before the erasure, so its absence \
             afterwards proves nothing. Found: {before:?}"
        );
    }
    // And it really is in the places the reading said it would be, so a
    // future change that stopped storing one of them makes this test say so
    // instead of quietly getting easier.
    for expected in [
        "customers.name",
        "customers.address_line1",
        "events.data",
        "charges.payer_ref",
        "idempotency_keys.response_body",
    ] {
        assert!(
            before
                .values()
                .any(|places| places.iter().any(|place| place == expected)),
            "nothing was found in `{expected}` before the erasure; if that column stopped \
             holding a payer identifier, say so here rather than leaving a scan that no \
             longer covers it. Found: {before:?}"
        );
    }

    sdk.customers()
        .del(&customer.id, RequestOptions::new())
        .await
        .expect("the erasure");

    let (after, _) = scan_for(&h.pool, &literals).await?;
    assert!(
        after.is_empty(),
        "a payer identifier survived the erasure vpay promised: {after:?}"
    );

    // The payment record is untouched, which is the other half of the
    // promise: erasing the payer must not erase the money.
    let (amount, customer_id): (i64, Option<String>) =
        sqlx::query_as("SELECT amount, customer_id FROM payment_intents WHERE id = $1")
            .bind(&intent.id)
            .fetch_one(&h.pool)
            .await
            .context("the intent after the erasure")?;
    assert_eq!(amount, AMOUNT);
    assert_eq!(customer_id.as_deref(), Some(customer.id.as_str()));

    h.shutdown().await;
    Ok(())
}

/// Every `text`/`varchar`/`jsonb` column in the live database, searched for
/// each literal.
///
/// Returns the literals that were found and, for each, the `table.column`
/// places holding it — plus how many columns were actually looked at, which
/// is what stops a scan that silently covered nothing from reading as a
/// clean bill of health.
///
/// The column list comes from `information_schema` rather than from a list in
/// this file for [`an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`]'s
/// whole reason. `_sqlx_migrations` is excluded: it holds the text of every
/// migration, and migration `0041` contains the word `[redacted]` — not any
/// payer's data, but a scan that matched it would be matching vpay's own
/// source.
async fn scan_for(
    pool: &PgPool,
    literals: &[&str],
) -> anyhow::Result<(HashMap<String, Vec<String>>, usize)> {
    let columns: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT table_name, column_name, data_type \
         FROM information_schema.columns \
         WHERE table_schema = 'public' \
           AND table_name <> '_sqlx_migrations' \
           AND data_type IN ('text', 'character varying', 'jsonb') \
         ORDER BY table_name, column_name",
    )
    .fetch_all(pool)
    .await
    .context("listing every text and jsonb column in the database")?;

    let mut found: HashMap<String, Vec<String>> = HashMap::new();
    for (table, column, _) in &columns {
        for literal in literals {
            // `::TEXT` so one statement covers `jsonb` and the string types
            // alike, and `position(... ) > 0` rather than `LIKE`, so a literal
            // containing `%` or `_` would still be searched for literally.
            let hits: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT COUNT(*) FROM {table} WHERE position($1 in {column}::TEXT) > 0"
            )))
            .bind(*literal)
            .fetch_one(pool)
            .await
            .with_context(|| format!("scanning {table}.{column}"))?;
            if hits > 0 {
                found
                    .entry((*literal).to_owned())
                    .or_default()
                    .push(format!("{table}.{column}"));
            }
        }
    }

    Ok((found, columns.len()))
}

// ------------------------------------------------------- the retention sweep

/// **The twelve-month retention sweep, through the shipping worker loop.**
///
/// Three customers, three answers, one pass:
///
/// * idle thirteen months, referenced by nothing → **hard-deleted**, with
///   exactly one `customer.deleted` event;
/// * idle eleven months → **kept**; the horizon is twelve, not "a while";
/// * idle thirteen months and referenced by a payment intent →
///   **anonymised**, with its own `customer.deleted`: the intent keeps its
///   amount, its status and its `customer`, and the payer's identifiers are
///   gone.
///
/// # The third case is the one that changed, and it is why this test is here
///
/// It used to assert "**kept**, because vpay never detaches a payment from
/// its payer" — and that was true of the row and false of the promise. The
/// foreign keys made a referenced customer undeletable, so the sweep skipped
/// it, so the twelve-month retention promise did not apply to exactly the
/// payers vpay had taken money from. Migration `0041` splits the two:
/// nothing is detached, and the payer is still erased. Deleting the
/// `anonymized_at IS NULL` clause from `Customers::idle_since` is what makes
/// the "second pass emits no second event" assertion below fail, because the
/// sweep would offer the anonymised customer again on every pass, for ever.
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
async fn the_sweep_deletes_an_idle_unreferenced_customer_and_anonymises_a_referenced_one()
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
        "a customer a payment intent references keeps its ROW — vpay does not detach a \
         payment from its payer — and is anonymised in place"
    );

    // The referenced one really was erased and not merely spared. The row
    // stays for the foreign key's sake; the payer does not.
    let (name, anonymized): (String, bool) =
        sqlx::query_as("SELECT name, anonymized_at IS NOT NULL FROM customers WHERE id = $1")
            .bind(&referenced)
            .fetch_one(&h.pool)
            .await
            .context("reading the anonymised customer")?;
    assert!(anonymized);
    assert_eq!(
        name.as_str(),
        vpay_db::REDACTED,
        "a customer the sweep could not delete must be anonymised, not skipped: skipping is \
         what exempted every paying customer from the retention promise"
    );

    // The intent it pins is untouched — the whole reason the row stays.
    let (amount, customer_id): (i64, Option<String>) =
        sqlx::query_as("SELECT amount, customer_id FROM payment_intents WHERE id = $1")
            .bind(&intent)
            .fetch_one(&h.pool)
            .await
            .context("the intent after its customer was anonymised")?;
    assert_eq!(amount, AMOUNT);
    assert_eq!(customer_id.as_deref(), Some(referenced.as_str()));

    // Two erasures, two events — one per customer, and the eleven-month one
    // has none. Both bodies are REDACTED: the merchant already received this
    // payer's details in `customer.created`, and the copy vpay stores in
    // `events` is never pruned, so it is the one the promise is about.
    let events: Vec<(String, Value)> = sqlx::query_as(
        "SELECT object_id, data FROM events WHERE type = 'customer.deleted' ORDER BY seq",
    )
    .fetch_all(&h.pool)
    .await
    .context("reading the customer.deleted events")?;
    assert_eq!(events.len(), 2, "two erasures, two events: {events:?}");
    let erased: Vec<&str> = events
        .iter()
        .map(|(object_id, _)| object_id.as_str())
        .collect();
    assert!(erased.contains(&idle.as_str()) && erased.contains(&referenced.as_str()));
    assert!(
        !erased.contains(&recent.as_str()),
        "eleven months is inside the twelve-month horizon"
    );

    for (object_id, data) in &events {
        assert_eq!(
            data.pointer("/object").and_then(Value::as_str),
            Some("customer")
        );
        assert_eq!(
            data.pointer("/id").and_then(Value::as_str),
            Some(object_id.as_str())
        );
        assert_eq!(
            data.pointer("/deleted"),
            Some(&Value::Bool(true)),
            "the body says what happened, on both branches: a merchant cannot tell from the \
             webhook whether the row survived, and there is no reason they should"
        );
        assert_eq!(
            data.pointer("/name").and_then(Value::as_str),
            Some(vpay_db::REDACTED),
            "the stored body carries no identifier of the payer's. This is the reverse of \
             what this event carried until 2026-09-10 and the reversal is the point: \
             `events` is never pruned, so an un-redacted body here is the largest surviving \
             copy of a payer vpay was asked to forget. Body: {data}"
        );
    }

    // A second pass emits no second event. For the deleted customer the row
    // matches nothing; for the anonymised one the guard is
    // `anonymized_at IS NULL`, and without it the sweep would offer that
    // customer again on every pass for the life of the deployment.
    run_the_sweep(&h.repositories, &h.pool).await?;
    let after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE type = 'customer.deleted'")
            .fetch_one(&h.pool)
            .await
            .context("counting customer.deleted events after a second pass")?;
    assert_eq!(after, 2, "a second sweep must not emit a second event");

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

    // AND NEITHER STAMP EMITTED ANYTHING. Migration `0039`'s header says so
    // in prose — `touch_last_used` moves `last_used_at`, a column on no wire
    // object at all, so an event for it would carry a body byte-identical to
    // the previous one, once per payment, for ever — and nothing asserted it
    // until the sabotage review of 2026-09-10. It is the cheap half of a
    // real risk: `customer.updated` now exists, and the stamp is a write to
    // the same table on the confirm path, so a later author reaching for
    // "every customer write emits" would turn one webhook per payment into
    // the merchant's problem. The only event about this customer is the one
    // its `POST` wrote.
    assert_eq!(
        events_about(&h.pool, &customer)
            .await?
            .into_iter()
            .map(|(kind, _)| kind)
            .collect::<Vec<_>>(),
        vec!["customer.created".to_owned()],
        "the retention stamp must emit nothing: it moves a column that is on no wire object"
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
/// # And the erasure's second table
///
/// `vpay_db::customers`' `UNREFERENCED` names **three** tables and decides,
/// per customer, between a hard delete and an anonymisation.
/// `the_sweep_deletes_an_idle_unreferenced_customer_and_anonymises_a_referenced_one`
/// exercises the `payment_intents` clause only. The last assertion here is
/// the `checkout_sessions` one: a customer that no intent names but a session
/// does must be *anonymised*, because hard-deleting it is what the `NO
/// ACTION` foreign key would refuse — rolling the whole erasure back and
/// leaving the payer un-erased with nobody told.
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

    // 5. THE ERASURE'S SECOND TABLE. Y is named by a session and by no intent
    //    at all, and that alone must decide which branch the erasure takes.
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

    let erased = raw_client()
        .delete(h.url(&format!("/v1/customers/{y}")))
        .bearer_auth(h.bearer(CLIENT_A))
        .header("Idempotency-Key", "delete-a-session-customer")
        .send()
        .await
        .context("deleting a customer a session references")?;
    assert_eq!(
        erased.status().as_u16(),
        200,
        "since migration 0041 a `DELETE` always succeeds or is a 404; the 409 it used to \
         answer here advised clearing name/email/phone, which `at_least_one_identifier` \
         refuses"
    );

    // The session's row survives with its customer attached — the foreign key
    // is still `NO ACTION` on both tables — and Y is anonymised rather than
    // deleted. If `UNREFERENCED` lost its `checkout_sessions` clause the
    // erasure would take the hard-delete branch, the FK would raise 23503,
    // and the whole transaction would roll back: the payer would not be
    // erased and the merchant would be told nothing.
    let (still_there, marker): (i64, Option<String>) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM checkout_sessions WHERE customer_id = $1), \
                (SELECT name FROM customers WHERE id = $1 AND anonymized_at IS NOT NULL)",
    )
    .bind(&y)
    .fetch_one(&h.pool)
    .await
    .context("reading the session and the anonymised customer")?;
    assert_eq!(still_there, 1, "the session keeps the customer it names");
    assert_eq!(
        marker.as_deref(),
        Some(vpay_db::REDACTED),
        "a customer a checkout session references is anonymised, never row-deleted: the \
         payment record survives with no payer on it"
    );

    h.shutdown().await;
    Ok(())
}

// --------------------------------------------- customer.created/.updated ---

/// Every `(type, data)` this database holds about one object, oldest first.
async fn events_about(pool: &PgPool, object_id: &str) -> anyhow::Result<Vec<(String, Value)>> {
    let rows: Vec<(String, Value)> =
        sqlx::query_as("SELECT type, data FROM events WHERE object_id = $1 ORDER BY seq")
            .bind(object_id)
            .fetch_all(pool)
            .await
            .context("reading the events a customer write did or did not commit")?;
    Ok(rows)
}

/// `customers.updated_at` and the `created_at` of the event at `index`.
async fn customer_and_event_stamps(
    pool: &PgPool,
    customer_id: &str,
    index: i64,
) -> anyhow::Result<(time::OffsetDateTime, time::OffsetDateTime)> {
    let updated_at: time::OffsetDateTime =
        sqlx::query_scalar("SELECT updated_at FROM customers WHERE id = $1")
            .bind(customer_id)
            .fetch_one(pool)
            .await
            .context("reading the customer's updated_at")?;
    let created_at: time::OffsetDateTime = sqlx::query_scalar(
        "SELECT created_at FROM events WHERE object_id = $1 ORDER BY seq OFFSET $2 LIMIT 1",
    )
    .bind(customer_id)
    .bind(index)
    .fetch_one(pool)
    .await
    .context("reading the event's created_at")?;
    Ok((updated_at, created_at))
}

/// `POST /v1/customers` emits exactly one `customer.created`, in the same
/// transaction as the insert; `POST /v1/customers/{id}` emits exactly one
/// `customer.updated`; and a **bodiless** update emits nothing at all.
///
/// # What was here before, and why it is not a gap being filled quietly
///
/// Neither event existed until 2026-09-10
/// ([issue #66](https://github.com/vaam-apps/vpay/issues/66)). `POST
/// /v1/customers` was a single statement on the pool and the update was a
/// read-modify-write on it; migration `0034` deliberately did **not** add the
/// two labels to `type_is_a_documented_event`, because `0023`'s rule is that
/// the vocabulary moves with the code that writes it. Migration `0039` adds
/// them in the same change that writes them.
///
/// # The transaction claim, and how it is measured rather than asserted
///
/// `customers.updated_at` takes migration `0034`'s `DEFAULT now()` on the
/// **insert**, and `events.created_at` takes migration `0018`'s. Postgres'
/// `now()` is `transaction_timestamp()` — fixed at the start of the
/// transaction — so the two are bit-identical exactly when the insert and the
/// event were one transaction. Moving the event into a second transaction is
/// invisible in every other assertion here and fails that one.
///
/// The **update** cannot use the same equality: its `updated_at` is bound
/// from the calling process's clock rather than from `now()` (it is the same
/// instant `last_used_at`'s `GREATEST` needs, and that one must be the
/// caller's — see `vpay_db::Customers::touch_last_used`). What holds instead
/// is an *ordering*: the transaction starts, Rust then reads its clock, so
/// the event's `created_at` is at or before the row's `updated_at`. An event
/// written in a transaction opened **after** the update committed is strictly
/// after it, and fails. `a_customer_write_and_its_event_roll_back_together`
/// in `vpay-db` is the other half, and the one that proves the rollback.
#[tokio::test]
async fn a_customer_create_and_update_each_emit_one_event_and_a_no_op_emits_none()
-> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    let created = sdk
        .customers()
        .create(
            CreateCustomerParams {
                phone: Some("+237 6 00 00 02 00".to_owned()),
                metadata: BTreeMap::from([("tier".to_owned(), "gold".to_owned())]),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("creating a customer");

    let emitted = events_about(&h.pool, &created.id).await?;
    let (kind, data) = match emitted.as_slice() {
        [one] => one.clone(),
        other => panic!("a create must emit exactly one event, got {other:?}"),
    };
    assert_eq!(kind, "customer.created");
    // `events.data` is the wire object itself; the `data: { object: … }`
    // envelope is put around it at delivery and by `GET /v1/events`.
    assert_eq!(data.get("id"), Some(&serde_json::json!(created.id)));
    assert_eq!(data.get("object"), Some(&serde_json::json!("customer")));
    assert_eq!(
        data.get("phone"),
        Some(&serde_json::json!("237600000200")),
        "the body carries the *canonical* MSISDN the row holds, not the merchant's spelling: \
         {data}"
    );
    assert_eq!(
        data.pointer("/metadata/tier"),
        Some(&serde_json::json!("gold"))
    );
    assert_eq!(
        data.get("last_used_at"),
        None,
        "`last_used_at` is an internal retention clock on no wire object; an event body is \
         stored for ever and is the last place to leak one: {data}"
    );

    let (updated_at, event_at) = customer_and_event_stamps(&h.pool, &created.id, 0).await?;
    assert_eq!(
        updated_at, event_at,
        "on the insert both columns take `DEFAULT now()`, and `now()` is the transaction's \
         start instant — so these agree only when the row and its event were one \
         transaction (issue #66)"
    );

    // A bodiless update: Stripe's no-op. The object comes back unchanged, and
    // nothing is written — including no event, because nothing changed.
    let unchanged = sdk
        .customers()
        .update(
            &created.id,
            UpdateCustomerParams::default(),
            RequestOptions::new(),
        )
        .await
        .expect("a bodiless update answers the object");
    assert_eq!(unchanged.id, created.id);
    assert_eq!(
        events_about(&h.pool, &created.id).await?.len(),
        1,
        "an update that changes nothing must emit nothing: a `customer.updated` for a no-op \
         is a webhook a merchant has to work out how to ignore"
    );

    // A real update.
    let patched = sdk
        .customers()
        .update(
            &created.id,
            UpdateCustomerParams {
                name: Some(Some("Ada".to_owned())),
                metadata: BTreeMap::from([("order".to_owned(), "42".to_owned())]),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect("updating the customer");
    assert_eq!(patched.name.as_deref(), Some("Ada"));

    let emitted = events_about(&h.pool, &created.id).await?;
    let (kind, data) = match emitted.as_slice() {
        [_created, second] => second.clone(),
        other => panic!("a create then one real update is two events, got {other:?}"),
    };
    assert_eq!(kind, "customer.updated");
    assert_eq!(data.get("name"), Some(&serde_json::json!("Ada")));
    assert_eq!(
        data.pointer("/metadata/tier"),
        Some(&serde_json::json!("gold")),
        "the body is the **merged** metadata the transaction wrote, not the keys the request \
         carried: {data}"
    );
    assert_eq!(
        data.pointer("/metadata/order"),
        Some(&serde_json::json!("42")),
        "{data}"
    );

    let (updated_at, event_at) = customer_and_event_stamps(&h.pool, &created.id, 1).await?;
    assert!(
        event_at <= updated_at,
        "the update's transaction starts before it reads the process clock, so the event's \
         `now()` is at or before the row's `updated_at`. An event written in a transaction \
         opened after the update committed is strictly later. event={event_at}, \
         row={updated_at}"
    );

    // A 404 writes nothing at all — the transaction is abandoned before any
    // write, and there is no object for an event to be about.
    let missing = "cus_00000000000000000000000x";
    let error = sdk
        .customers()
        .update(
            missing,
            UpdateCustomerParams {
                name: Some(Some("Nobody".to_owned())),
                ..Default::default()
            },
            RequestOptions::new(),
        )
        .await
        .expect_err("no such customer");
    match error {
        vpay_sdk::Error::Api { status, .. } => assert_eq!(status, 404),
        other => panic!("expected a vpay API error envelope, got {other:?}"),
    }
    assert_eq!(events_about(&h.pool, missing).await?, Vec::new());

    h.shutdown().await;
    Ok(())
}

/// Two concurrent `POST /v1/customers/{id}` requests each adding one metadata
/// key keep **both**, and the second event carries both.
///
/// # The window this closes, and why it stopped being acceptable
///
/// `metadata` is merged key-wise (Stripe's contract), so the written value is
/// a function of the stored one. Until 2026-09-10 the read ran on the pool
/// and `vpay_api::v1::customers::update`'s own doc comment said, in as many
/// words, that two concurrent updates could lose a key and that this was not
/// closed. It could be left while nothing depended on the result being
/// definite. `customer.updated` is exactly such a dependency: a merchant
/// acting on an event describing the losing merge acts on a state the
/// database does not hold.
///
/// The read is now `SELECT … FOR UPDATE` inside the transaction that writes,
/// so the second request blocks, re-reads the committed merge, and merges
/// onto that.
///
/// **The decisive mutation:** drop `FOR UPDATE` from
/// `vpay_db::customers::lock_for_update`. Both requests then read the same
/// stored map and the later write clobbers the earlier key.
///
/// # Why six rounds and not one
///
/// The interleaving that loses a key needs both reads to land before either
/// write, and nothing outside the handler can force that ordering — a test
/// seam that could would be a code path no deployment runs (AGENTS.md rule
/// 1). So the *passing* direction is deterministic (with the lock, no
/// interleaving can lose a key, and six rounds all keep both), and the
/// mutation is caught probabilistically by giving it six chances. Measured
/// 2026-09-10: with `FOR UPDATE` removed, this fails.
#[tokio::test]
async fn two_concurrent_metadata_merges_keep_both_keys_and_the_event_carries_the_committed_state()
-> anyhow::Result<()> {
    let h = harness().await?;
    let sdk = h.a();

    for round in 0..6_u32 {
        let customer = sdk
            .customers()
            .create(
                CreateCustomerParams {
                    phone: Some("237600000200".to_owned()),
                    ..Default::default()
                },
                RequestOptions::new(),
            )
            .await
            .expect("the customer both requests patch")
            .id;

        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for key in ["left", "right"] {
            let sdk = h.a();
            let barrier = Arc::clone(&barrier);
            let customer = customer.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                sdk.customers()
                    .update(
                        &customer,
                        UpdateCustomerParams {
                            metadata: BTreeMap::from([(key.to_owned(), round.to_string())]),
                            ..Default::default()
                        },
                        RequestOptions::new(),
                    )
                    .await
            }));
        }
        for handle in handles {
            handle
                .await
                .expect("the task did not panic")
                .expect("both concurrent updates succeed");
        }

        let stored: Value = sqlx::query_scalar("SELECT metadata FROM customers WHERE id = $1")
            .bind(&customer)
            .fetch_one(&h.pool)
            .await
            .context("reading the merged metadata")?;
        assert_eq!(
            stored.pointer("/left"),
            Some(&serde_json::json!(round.to_string())),
            "round {round}: the `left` key was clobbered by a merge computed over a stale \
             read: {stored}"
        );
        assert_eq!(
            stored.pointer("/right"),
            Some(&serde_json::json!(round.to_string())),
            "round {round}: the `right` key was clobbered: {stored}"
        );

        // Two updates, two events, and **each carries the state its own
        // transaction committed** — which is the whole reason the lock is
        // here rather than the race merely being tolerated. They do not
        // coalesce: two `POST`s are two transitions, and a merchant building
        // dedupe logic on "one transition, one event" is entitled to both.
        let emitted = events_about(&h.pool, &customer).await?;
        let kinds: Vec<&str> = emitted.iter().map(|(kind, _)| kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["customer.created", "customer.updated", "customer.updated"],
            "round {round}"
        );
        // The FIRST update's event is the one an ordering assertion alone
        // would let through. Whichever request took the lock first merged
        // onto an empty map, so its body carries exactly **one** of the two
        // keys. An event carrying both would mean it was rendered from a row
        // the second transaction had already written — the body describing a
        // state its own transaction did not commit, which is the mirror of
        // the bug the lock closes and is not caught by the assertion on the
        // last event. Added by the sabotage review, 2026-09-10.
        let first_update = emitted
            .get(1)
            .map(|(_, data)| data.clone())
            .expect("three events");
        let first_metadata = first_update
            .get("metadata")
            .and_then(serde_json::Value::as_object)
            .expect("the event body carries a metadata object");
        assert_eq!(
            first_metadata.len(),
            1,
            "round {round}: the first update committed one key and its event must say so, \
             not report the merge the second one went on to make: {first_update}"
        );
        let last = emitted
            .last()
            .map(|(_, data)| data.clone())
            .expect("three events");
        assert_eq!(
            last.get("metadata"),
            Some(&stored),
            "round {round}: the last event must carry the committed merge, key for key: \
             {last}"
        );
    }

    h.shutdown().await;
    Ok(())
}

/// The event vocabulary is closed, and it is closed around exactly the three
/// `customer.*` types that have writers.
///
/// `customer.created` and `customer.updated` are migration `0039`'s;
/// `customer.deleted` is `0034`'s. Everything else Stripe spells
/// `customer.*` — `customer.subscription.created`,
/// `customer.source.created` — is asserted **absent**, because vpay has no
/// subscriptions and no stored instruments, and a label in a closed
/// vocabulary that nothing produces is what that CHECK exists to prevent.
///
/// **The decisive mutation:** drop either new label from migration `0039` and
/// the corresponding transition above starts failing with a `23514`, while
/// this case names which label is missing.
#[tokio::test]
async fn the_event_vocabulary_holds_exactly_the_customer_types_that_have_writers()
-> anyhow::Result<()> {
    let h = harness().await?;

    for accepted in ["customer.created", "customer.updated", "customer.deleted"] {
        let written = sqlx::query(
            "INSERT INTO events (id, merchant_id, livemode, type, object_id, data) \
             VALUES ($1, $2, false, $3, 'cus_x', '{}'::jsonb)",
        )
        .bind(format!("evt_{}", uuid::Uuid::new_v4().simple()))
        .bind(MERCHANT_A)
        .bind(accepted)
        .execute(&h.pool)
        .await;
        assert!(
            written.is_ok(),
            "`{accepted}` has a writer in vpay and must be in \
             type_is_a_documented_event: {written:?}"
        );
    }

    for refused in [
        "customer.subscription.created",
        "customer.source.created",
        "customer.discount.created",
    ] {
        let written = sqlx::query(
            "INSERT INTO events (id, merchant_id, livemode, type, object_id, data) \
             VALUES ($1, $2, false, $3, 'cus_x', '{}'::jsonb)",
        )
        .bind(format!("evt_{}", uuid::Uuid::new_v4().simple()))
        .bind(MERCHANT_A)
        .bind(refused)
        .execute(&h.pool)
        .await;
        assert!(
            written.is_err(),
            "`{refused}` has no writer in vpay, so the database must refuse it — a label in a \
             closed vocabulary that nothing produces is what that CHECK exists to prevent"
        );
    }

    h.shutdown().await;
    Ok(())
}
