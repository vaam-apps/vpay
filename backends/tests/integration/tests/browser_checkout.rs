//! `/v1/browser`, end to end: a merchant creates a payment intent with the
//! OAuth SDK, hands the payer a publishable key and a `client_secret`, and a
//! **browser** confirms it — with no bearer token anywhere.
//!
//! ```text
//!   vpay_sdk::Client                     (the artefact a merchant integrates)
//!     -> POST /v1/payment_intents        -> client_secret, on the response
//!   raw reqwest, as a browser would      (the SDK cannot express this)
//!     -> POST /v1/browser/payment_intents/{id}/confirm
//!        key=pk_test_… & client_secret=pi_…_secret_… & payment_method_data[…]
//!        -> charge row + poll job, one transaction
//!        -> MTN adapter -> HTTP -> WireMock container
//!     -> GET /v1/browser/payment_intents/{id}?key=…&client_secret=…
//! ```
//!
//! # Why raw `reqwest` throughout the browser half
//!
//! `vpay-sdk` is a *merchant* SDK: it holds a private key, mints a
//! `private_key_jwt` assertion and sends a bearer token on every request.
//! There is no way to express "no credential except a query parameter"
//! through it, and a browser cannot hold any of what it needs. The browser
//! package that *does* speak this surface — `@vaam-apps/vpay-stripe-js` — is
//! TypeScript and cannot be linked here; its own suite drives a `node:http`
//! stub, and this file is the other half of that claim: the stub's shape
//! against a real server. The bytes asserted below (`key`, `client_secret`,
//! `payment_method_data[type]`, `payment_method_data[<type>][msisdn]`) are
//! exactly what `sdks/stripe-js/src/client.ts` sends.
//!
//! # What this file claims, and why each one needs a whole stack
//!
//! 1. a browser confirm with a valid key and secret reaches the **rail** —
//!    the intent goes `processing` and there is a request in the stub's own
//!    journal;
//! 2. every credential failure — a wrong secret, an unknown publishable key,
//!    another merchant's key — is the identical 404, **byte for byte**. That
//!    is the whole confidentiality property of an unauthenticated surface and
//!    it cannot be asserted from a unit test, because it is a property of
//!    rendered response bodies;
//! 3. the surface offers no `create`, no `list` and no `cancel`;
//! 4. a preflight is allowed from any origin, and the merchant `/v1` nest
//!    answers preflights with **no** CORS header at all;
//! 5. a confirm succeeds with no `Idempotency-Key` — a browser cannot send
//!    one without turning a CORS simple request into a preflighted one — and
//!    a second one is the `409`, not a second charge;
//! 6. `BROWSER_ROUTES` is exactly two entries and neither answers `401`, the
//!    sibling of `payment_intents.rs`'s
//!    `every_registered_v1_path_answers_401_without_a_token`;
//! 7. `client_secret` is on `create` and on both retrieves, and **absent**
//!    from the `/v1` list and from `events.data` — the latter written by the
//!    real settlement transaction, not staged.
//!
//! # No test doubles
//!
//! Real Postgres, a real WireMock rail (the shared
//! `backends/tests/conformance/wiremock` tree), the shipping adapters, the
//! shipping router, the shipping merchant SDK and the shipping worker loop.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::Context as _;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use serde_json::Value;
use sqlx::PgPool;
use testcontainers::{ContainerAsync, GenericImage};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use uuid::Uuid;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, MERCHANT_AUDIENCE, ProviderHost};
use vpay_db::{PaymentIntents, Repositories};
use vpay_sdk::{
    CreatePaymentIntentParams, Credentials, IntentStatus, PaymentMethodType, RequestOptions,
};
use vpay_worker::{Adapters, RailConfigs, RecoveryPolicy};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client_with_publishable_keys,
    migrated_postgres, rail_configs, serve,
};

/// The merchant whose payer drives this surface, and the tenant it acts for.
const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";

/// The second merchant. It exists for one case: a *valid* publishable key,
/// presented against someone else's intent.
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

/// Merchant A's publishable key — what a merchant's checkout page renders
/// into `loadStripe(...)`. `pk_test_` because this deployment is
/// `livemode: false`, which `Config::validate_all` enforces.
const PK_A: &str = "pk_test_acmecameroonsandbox01";

/// Merchant B's. Well-formed, registered, and belonging to the wrong tenant
/// for every intent this suite creates.
const PK_B: &str = "pk_test_betadoualasandbox0001";

/// A key no registration carries. Deliberately well-shaped: a *malformed*
/// key would be refused at boot, so the only unknown key a payer can present
/// is one that could have been real.
const PK_UNKNOWN: &str = "pk_test_neverregisteredhere01";

const PUSH_RAIL: &str = "mtn_momo";
const CURRENCY: &str = "xaf";
const AMOUNT: i64 = 5000;

/// A documentation MSISDN nothing stubs, so a confirm with it falls through
/// to `requesttopay.json`'s catch-all `202`.
const MSISDN: &str = "237670000000";

/// The reference `requesttopay-status.json` answers `FAILED /
/// NOT_ENOUGH_FUNDS` to. Used by the `events.data` case, which needs the
/// **real** settlement transaction to have written an event and does not care
/// which terminal answer produced it.
const DECLINED_REF: Uuid = Uuid::from_u128(0x0f01);

/// The `wiremock/{rail}` root the conformance suite and `compose.yml` both
/// use — referenced, never copied.
fn mappings_dir(rail: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../conformance/wiremock")
        .join(rail)
}

// ------------------------------------------------------------------ metrics

/// This test binary's Prometheus recorder, installed on first use.
///
/// A near-copy of `worker_e2e.rs`'s and `webhooks.rs`'s static of the same
/// name, duplicated rather than shared for the reason both give: each file
/// under `tests/` is its own binary, `metrics::set_global_recorder` succeeds
/// exactly once per process, and under `cargo nextest` that is once per
/// test — which is what makes the exact-count assertion below meaningful
/// rather than a total across whatever else ran first in the process.
static METRICS: LazyLock<PrometheusHandle> = LazyLock::new(|| {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    metrics::set_global_recorder(recorder).expect("this test binary installs exactly one recorder");
    vpay_core::metrics::describe_all();
    handle
});

/// Serves the **shipping** observability router
/// (`vpay_api::observability`, the same function both `main.rs` files call)
/// on an ephemeral port, rendering [`METRICS`] — see `worker_e2e.rs`'s
/// identical helper for why this goes through a real socket rather than
/// `PrometheusHandle::render()` directly: the thing under test is what a
/// Prometheus scrape of a vpay pod would actually receive, including the
/// route being mounted and the handler returning it.
async fn serve_metrics() -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let handle = METRICS.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding the observability listener")?;
    let addr = listener.local_addr().context("reading the bound address")?;
    let task = tokio::spawn(async move {
        let _ = vpay_api::observability::serve(
            listener,
            move || handle.render(),
            std::future::pending::<()>(),
        )
        .await;
    });
    Ok((addr, task))
}

// ------------------------------------------------------------------ harness

struct Harness {
    _postgres: ContainerAsync<PostgresImage>,
    _mtn: ContainerAsync<GenericImage>,
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    /// The plain `sqlx` pool, for the fixtures that read or force schema
    /// state no repository method owns.
    pool: PgPool,
    base_url: String,
    mtn_url: String,
    pem_a: String,
    signing_key: LoadedSigningKey,
    adapters: Arc<Adapters>,
    rails: Arc<RailConfigs>,
}

impl Harness {
    /// Merchant A's own SDK client — the *merchant* half of every test here.
    fn a(&self) -> vpay_sdk::Client {
        vpay_sdk::Client::builder(&self.base_url)
            .credentials(
                Credentials::rsa_pem(CLIENT_A, &self.pem_a).expect("the generated PEM parses"),
            )
            .build()
            .expect("the SDK client builds from a base URL and a credential")
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// A `/v1` bearer token for `client_id`, minted with the server's own
    /// signer — for the raw requests the SDK cannot make.
    fn bearer(&self, client_id: &str) -> String {
        self.signing_key
            .token_manager()
            .issue_client_token_with_extra(
                client_id,
                900,
                Some(vpay_api::SCOPE_PAYMENTS_WRITE.to_owned()),
                Some(MERCHANT_AUDIENCE.to_owned()),
                std::collections::HashMap::new(),
            )
            .expect("the server's own signer mints a merchant token")
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

/// The configuration this deployment runs under: two merchants (one with a
/// publishable key each), one rail pointed at the stub, one currency.
fn config_with(base_url: &str, mtn_url: &str, jwks_a: Value, jwks_b: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "browser-checkout".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
        },
        providers: vec![ProviderHost {
            code: PUSH_RAIL.to_owned(),
            enabled: true,
            host: HostEntry {
                url: mtn_url.to_owned(),
                label: "mtn-wiremock".to_owned(),
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
            merchant_client_with_publishable_keys(CLIENT_A, MERCHANT_A, jwks_a, &[PK_A]),
            merchant_client_with_publishable_keys(CLIENT_B, MERCHANT_B, jwks_b, &[PK_B]),
        ],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard_client: None,
    }
}

async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (postgres, repositories, pool) = migrated_postgres().await?;

    let mtn = vpay_testkit::containers::start_wiremock(&mappings_dir("mtn"))
        .await
        .context("the MTN stub container starts")?;
    let mtn_url = format!(
        "http://127.0.0.1:{}",
        mtn.get_host_port_ipv4(8080)
            .await
            .context("the MTN stub's mapped port")?
    );

    let (server_pem, _server_jwks) = generate_key();
    let (pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();

    let mtn_for_config = mtn_url.clone();
    let (jwks_a_for_server, jwks_b_for_server) = (jwks_a.clone(), jwks_b.clone());
    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(
            base_url,
            &mtn_for_config,
            jwks_a_for_server,
            jwks_b_for_server,
        )
    })
    .await?;
    // Rebuilt from the bound URL rather than smuggled out of the closure, as
    // `confirm_rails.rs` does it: the worker's `RailConfigs` must be the
    // projection of the *same* configuration the server booted with, and two
    // calls to one pure function is the cheapest way to say so.
    let config = config_with(&served.base_url, &mtn_url, jwks_a, jwks_b);

    Ok(Harness {
        _postgres: postgres,
        _mtn: mtn,
        server: served.server,
        repositories,
        pool,
        base_url: served.base_url,
        mtn_url,
        pem_a,
        signing_key: served.signing_key,
        adapters: Arc::new(support::adapters_by_code()),
        rails: Arc::new(rail_configs(&config)),
    })
}

fn create_params() -> CreatePaymentIntentParams {
    CreatePaymentIntentParams {
        amount: AMOUNT,
        currency: CURRENCY.to_owned(),
        payment_method_types: vec![PaymentMethodType::MtnMomo],
        metadata: BTreeMap::new(),
        description: None,
    }
}

// ------------------------------------------------------------------ reading

/// A plain HTTP client with no credential machinery at all — the closest
/// thing in Rust to what a payer's browser is.
fn browser() -> reqwest::Client {
    ensure_crypto_provider_installed();
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("a plain reqwest client builds")
}

/// The merchant half of a checkout: `paymentIntents.create` through the
/// **shipping SDK**, then the `client_secret` read off the raw wire.
///
/// Two calls rather than one, and the split is a real finding rather than
/// test convenience: `vpay_sdk::PaymentIntent` (`sdks/rust/src/model.rs`)
/// predates this field and does not model it, so a merchant integrating in
/// Rust today cannot get the credential out of the SDK either. Using the SDK
/// for the create is what keeps this suite honest about the merchant path;
/// reading the credential from JSON is what a Rust merchant actually has to
/// do. Teaching the SDK the field is `sdks/rust`'s work and is listed in the
/// summary as not done here.
async fn create_intent(h: &Harness) -> anyhow::Result<(String, String)> {
    let intent = h
        .a()
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .map_err(|e| anyhow::anyhow!("creating a payment intent through the SDK: {e}"))?;
    assert_eq!(intent.status, IntentStatus::RequiresPaymentMethod);

    let body: Value = browser()
        .get(h.url(&format!("/v1/payment_intents/{}", intent.id)))
        .bearer_auth(h.bearer(CLIENT_A))
        .send()
        .await
        .context("reading the credential the SDK does not model")?
        .json()
        .await
        .context("the retrieve body is JSON")?;
    let secret = body
        .get("client_secret")
        .and_then(Value::as_str)
        .context("`retrieve` must render the client_secret a merchant's page needs")?
        .to_owned();
    Ok((intent.id, secret))
}

/// A browser confirm: the exact form body `@vaam-apps/vpay-stripe-js` sends, with no
/// `Idempotency-Key` and no `Authorization`.
async fn browser_confirm(
    h: &Harness,
    id: &str,
    key: &str,
    secret: &str,
) -> anyhow::Result<(u16, Value)> {
    let response = browser()
        .post(h.url(&format!("/v1/browser/payment_intents/{id}/confirm")))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "key={key}&client_secret={secret}&payment_method_data[type]={PUSH_RAIL}\
             &payment_method_data[{PUSH_RAIL}][msisdn]={MSISDN}"
        ))
        .send()
        .await
        .context("confirming through the browser surface")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the confirm body is JSON")?;
    Ok((status, body))
}

/// A browser retrieve, with the credential in the query string.
async fn browser_retrieve(
    h: &Harness,
    id: &str,
    key: &str,
    secret: &str,
) -> anyhow::Result<(u16, Value)> {
    // The query string is built by hand rather than through
    // `RequestBuilder::query`: the workspace's reqwest pin does not enable
    // that helper, and every value here is URL-safe by construction — a
    // publishable key is `[A-Za-z0-9]` after its prefix
    // (`ConfigError::MalformedPublishableKey`) and a client secret is
    // `vpay_core::ids`' base32 alphabet, which that module proves
    // `encodeURIComponent` is the identity on. That is the *same* reason
    // `@vaam-apps/vpay-stripe-js` can put them in a query string at all.
    let response = browser()
        .get(h.url(&format!(
            "/v1/browser/payment_intents/{id}?key={key}&client_secret={secret}"
        )))
        .send()
        .await
        .context("retrieving through the browser surface")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the retrieve body is JSON")?;
    Ok((status, body))
}

/// Every request the MTN stub has actually received, from its **own**
/// journal — not from anything vpay recorded about itself.
async fn mtn_journal(mtn_url: &str) -> anyhow::Result<Vec<String>> {
    let body: Value = reqwest::get(format!("{mtn_url}/__admin/requests"))
        .await?
        .json()
        .await?;
    Ok(body
        .get("requests")
        .and_then(Value::as_array)
        .context("WireMock's journal has a `requests` array")?
        .iter()
        .filter_map(|entry| {
            let request = entry.get("request")?;
            Some(format!(
                "{} {}",
                request.get("method")?.as_str()?,
                request.get("url")?.as_str()?
            ))
        })
        .collect())
}

async fn stored_status(pool: &PgPool, id: &str) -> anyhow::Result<String> {
    let (status,): (String,) =
        sqlx::query_as("SELECT status::TEXT FROM payment_intents WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .context("reading the intent's status")?;
    Ok(status)
}

async fn charge_count(pool: &PgPool, id: &str) -> anyhow::Result<i64> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM charges WHERE payment_intent_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .context("counting charges")?;
    Ok(count)
}

// ---------------------------------------------------------------- the tests

/// Claim 1: a payer's browser, holding only what a merchant's page renders,
/// drives a real payment as far as the rail.
///
/// The stub's own journal is what proves "reached the rail": vpay's
/// `provider_requests` row would prove only that vpay believes it sent
/// something.
#[tokio::test]
async fn a_browser_confirm_reaches_the_rail_and_moves_the_intent_to_processing()
-> anyhow::Result<()> {
    let h = harness().await?;
    let (id, secret) = create_intent(&h).await?;

    let before = mtn_journal(&h.mtn_url).await?.len();
    let (status, body) = browser_confirm(&h, &id, PK_A, &secret).await?;

    assert_eq!(status, 200, "{body:#}");
    assert_eq!(
        body.get("status").and_then(Value::as_str),
        Some("processing"),
        "a push rail that accepted the request leaves the intent processing: {body:#}"
    );
    assert_eq!(
        body.get("id").and_then(Value::as_str),
        Some(id.as_str()),
        "{body:#}"
    );
    // The credential comes back, because `@vaam-apps/vpay-stripe-js` types every
    // response on this surface as carrying one — a merchant's page polls with
    // the object it was just handed.
    assert_eq!(
        body.get("client_secret").and_then(Value::as_str),
        Some(secret.as_str()),
        "{body:#}"
    );

    assert_eq!(
        stored_status(&h.pool, &id).await?,
        "processing",
        "the response and the committed row must agree"
    );

    let journal = mtn_journal(&h.mtn_url).await?;
    assert!(
        journal.len() > before,
        "the confirm must have reached the rail's own socket; journal: {journal:?}"
    );
    assert!(
        journal
            .iter()
            .any(|entry| entry.contains("requesttopay") && entry.starts_with("POST")),
        "the rail must have received a requestToPay; journal: {journal:?}"
    );

    // And the payer can poll it back through the same credential.
    let (status, polled) = browser_retrieve(&h, &id, PK_A, &secret).await?;
    assert_eq!(status, 200, "{polled:#}");
    assert_eq!(
        polled.get("status").and_then(Value::as_str),
        Some("processing"),
        "{polled:#}"
    );
    assert_eq!(
        polled.get("client_secret").and_then(Value::as_str),
        Some(secret.as_str()),
        "the polling endpoint renders the credential too: {polled:#}"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 2, and the security property of the whole surface: every credential
/// failure is the **identical** response.
///
/// Byte-for-byte, not merely "all 404": a body that named which half was
/// wrong would separate "this intent exists" from "your secret is wrong",
/// which is the first half of a guessing attack; one that distinguished an
/// unknown key from a foreign one would let anyone enumerate a deployment's
/// merchants.
///
/// **Revert-proof.** Change `browser::authenticate`'s merchant-mismatch arm
/// to `ApiError::Forbidden` — the "more informative" answer someone will
/// eventually reach for — and the `foreign key` case fails on both the status
/// and the body. Change any arm's `RESOURCE` and every case fails on the
/// body.
#[tokio::test]
async fn every_credential_failure_is_the_identical_404() -> anyhow::Result<()> {
    let h = harness().await?;
    let (id, secret) = create_intent(&h).await?;

    // The reference answer: an id that never existed, which is what a payer
    // with a typo'd link sends. Everything below must equal this.
    let missing_id = "pi_00000000000000000000000x";
    let (baseline_status, baseline_body) =
        browser_retrieve(&h, missing_id, PK_A, "pi_x_secret_y").await?;
    assert_eq!(baseline_status, 404, "{baseline_body:#}");
    assert_eq!(
        baseline_body.pointer("/error/type").and_then(Value::as_str),
        Some("invalid_request_error"),
        "{baseline_body:#}"
    );
    assert_eq!(
        baseline_body.pointer("/error/code").and_then(Value::as_str),
        Some("resource_missing"),
        "{baseline_body:#}"
    );
    // The sentence `sdks/stripe-js/src/testing/browser-stub.ts` builds.
    assert_eq!(
        baseline_body
            .pointer("/error/message")
            .and_then(Value::as_str),
        Some(format!("No such payment intent: {missing_id}").as_str()),
        "{baseline_body:#}"
    );
    assert!(
        baseline_body.pointer("/error/param").is_none(),
        "the uniform 404 names no parameter — naming one would say which half was wrong: \
         {baseline_body:#}"
    );

    /// The same body with the id substituted: the id is echoed (Stripe does
    /// the same, and a merchant grepping their logs needs it), so the
    /// comparison has to allow for that and nothing else.
    fn with_id(baseline: &Value, id: &str, missing_id: &str) -> Value {
        let rendered = serde_json::to_string(baseline).expect("the envelope serialises");
        serde_json::from_str(&rendered.replace(missing_id, id)).expect("still JSON")
    }
    let expected = with_id(&baseline_body, &id, missing_id);

    // Every way to be refused, on both routes.
    let cases: Vec<(&str, &str, String)> = vec![
        (
            "the right key, a wrong client_secret",
            PK_A,
            format!("{id}_secret_{}", "0".repeat(32)),
        ),
        (
            "the right key, a client_secret for a different intent",
            PK_A,
            format!("pi_00000000000000000000000x_secret_{}", "a".repeat(32)),
        ),
        (
            "a publishable key no registration carries",
            PK_UNKNOWN,
            secret.clone(),
        ),
        (
            "another merchant's publishable key, valid and registered",
            PK_B,
            secret.clone(),
        ),
        ("no credential at all", "", String::new()),
    ];

    for (what, key, presented) in cases {
        let (status, body) = browser_retrieve(&h, &id, key, &presented).await?;
        assert_eq!(status, 404, "{what}: {body:#}");
        assert_eq!(body, expected, "{what}: the refusal must be identical");

        let (status, body) = browser_confirm(&h, &id, key, &presented).await?;
        assert_eq!(status, 404, "{what} (confirm): {body:#}");
        assert_eq!(
            body, expected,
            "{what} (confirm): the refusal must be identical"
        );
    }

    // And nothing was charged by any of them.
    assert_eq!(
        charge_count(&h.pool, &id).await?,
        0,
        "a refused confirm must leave no charge behind"
    );
    assert_eq!(
        stored_status(&h.pool, &id).await?,
        "requires_payment_method"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 3: the surface offers exactly the two routes it documents.
///
/// `create` and `cancel` are the two a payer must never reach: one would let
/// anyone open payment intents against a merchant's tenant, the other would
/// let a payer void a payment their merchant is relying on. `list` would hand
/// out every intent the tenant has.
#[tokio::test]
async fn the_browser_surface_has_no_create_no_list_and_no_cancel() -> anyhow::Result<()> {
    let h = harness().await?;
    let (id, secret) = create_intent(&h).await?;
    let http = browser();
    // Sent through `RequestBuilder::query`/`::form` rather than a `format!`
    // string, so the credential is carried as a value the request builder
    // encodes — never joined into a URL or body by hand — the same reason
    // `AMOUNT` is stringified once up front rather than inside a `format!`.
    let amount = AMOUNT.to_string();

    for (method, path) in [
        ("POST", "/v1/browser/payment_intents".to_owned()),
        ("GET", "/v1/browser/payment_intents".to_owned()),
        ("POST", format!("/v1/browser/payment_intents/{id}/cancel")),
        ("GET", "/v1/browser".to_owned()),
        ("GET", "/v1/browser/events".to_owned()),
    ] {
        let url = h.url(&path);
        let request = match method {
            "GET" => http
                .get(&url)
                .query(&[("key", PK_A), ("client_secret", secret.as_str())]),
            _ => http.post(&url).form(&[
                ("key", PK_A),
                ("client_secret", secret.as_str()),
                ("amount", amount.as_str()),
                ("currency", CURRENCY),
            ]),
        };
        let response = request
            .send()
            .await
            .with_context(|| format!("{method} {path}"))?;
        assert_eq!(
            response.status().as_u16(),
            404,
            "{method} {path} must not exist on the payer-facing surface"
        );
        let body: Value = response.json().await.context("the 404 body is JSON")?;
        assert_eq!(
            body.pointer("/error/type").and_then(Value::as_str),
            Some("invalid_request_error"),
            "{method} {path}: the browser nest must answer its own envelope, not axum's empty \
             body and not the merchant nest's 401: {body:#}"
        );
    }

    h.shutdown().await;
    Ok(())
}

/// Claim 4: CORS is on the browser nest and on nothing else.
///
/// A payer's page is served from the merchant's own origin, so without the
/// preflight answer the browser never sends the confirm at all. The merchant
/// `/v1` nest must carry no such header: a permissive one there is an
/// invitation to put a bearer token in a page.
#[tokio::test]
async fn the_browser_nest_answers_a_preflight_and_the_merchant_nest_does_not() -> anyhow::Result<()>
{
    let h = harness().await?;
    let http = browser();

    let preflight = |url: String| {
        let http = http.clone();
        async move {
            http.request(reqwest::Method::OPTIONS, url)
                .header("origin", "https://shop.example")
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "content-type")
                .send()
                .await
        }
    };

    let response = preflight(h.url("/v1/browser/payment_intents/pi_x/confirm")).await?;
    let headers = response.headers().clone();
    assert_eq!(
        headers
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("*"),
        "a merchant's checkout can be on any domain they own"
    );
    assert!(
        headers.get("access-control-allow-credentials").is_none(),
        "`*` is only safe with credentials off; nothing here reads a cookie"
    );
    let allowed_methods = headers
        .get("access-control-allow-methods")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_uppercase();
    for method in ["GET", "POST", "OPTIONS"] {
        assert!(
            allowed_methods.contains(method),
            "{method} must be allowed; got {allowed_methods:?}"
        );
    }
    assert!(
        headers
            .get("access-control-allow-headers")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains("content-type"),
        "the form content type is the one header @vaam-apps/vpay-stripe-js sets"
    );
    assert_eq!(
        headers
            .get("access-control-max-age")
            .and_then(|v| v.to_str().ok()),
        Some("600")
    );

    for path in [
        "/v1/payment_intents",
        "/v1/payment_intents/pi_x/confirm",
        "/v1/oauth/token",
    ] {
        let response = preflight(h.url(path)).await?;
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .is_none(),
            "{path} must carry no CORS header: nothing legitimate calls the merchant API from a \
             browser, and a header here would invite a bearer token into a page"
        );
    }

    h.shutdown().await;
    Ok(())
}

/// Claim 5: no `Idempotency-Key`, and a double-submit is the `409` rather
/// than a second charge.
///
/// `/v1`'s POSTs require the header (D7). A browser cannot send it without
/// turning a CORS simple request into a preflighted one, so this surface does
/// not read it — and what protects the payer is the pre-insert check plus
/// `one_charge_per_intent`, which is what was doing the work all along.
///
/// The `409`'s `code` is asserted, not just its status: a payer's page has to
/// tell "already confirmed, go and poll" from "your card was declined", and
/// `@vaam-apps/vpay-stripe-js` surfaces `error.code` for exactly that.
#[tokio::test]
async fn a_browser_confirm_needs_no_idempotency_key_and_a_second_one_is_the_409()
-> anyhow::Result<()> {
    let h = harness().await?;
    let (id, secret) = create_intent(&h).await?;

    let (status, first) = browser_confirm(&h, &id, PK_A, &secret).await?;
    assert_eq!(
        status, 200,
        "a browser cannot send an Idempotency-Key and must not need one: {first:#}"
    );

    let (status, second) = browser_confirm(&h, &id, PK_A, &secret).await?;
    assert_eq!(status, 409, "{second:#}");
    assert_eq!(
        second.pointer("/error/type").and_then(Value::as_str),
        Some("invalid_request_error"),
        "{second:#}"
    );
    assert_eq!(
        charge_count(&h.pool, &id).await?,
        1,
        "one charge per intent, forever — a double-tapping payer must not be charged twice"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 6: every entry in the browser route table is reachable **without a
/// merchant token**, and the same relative path under `/v1` is not.
///
/// The sibling of `payment_intents.rs`'s
/// `every_registered_v1_path_answers_401_without_a_token`, which walks
/// `V1_ROUTES` and asserts the opposite. Both are needed: without this one, a
/// browser route accidentally added to `V1_ROUTES` would start answering 401
/// to every payer and the other test would happily agree it should.
///
/// # This test was changed in Step 9, deliberately
///
/// It pinned the table at two entries from Step 5c until 2026-09-04. Step 9
/// added three `GET`s about a **checkout session**
/// (`vpay_api::browser::checkout_sessions`), so the count moves — and the
/// property is re-stated rather than relaxed: exactly one entry answers a
/// non-read method, and it is the confirm that has always been here. vpay's
/// own checkout page confirms through *that* route, so Step 9 added no second
/// way to move money.
///
/// The unit-level half of the same pin, with the exhaustive ordered list, is
/// `the_browser_surface_offers_two_read_and_confirm_routes_and_nothing_else`
/// in `vpay-api`'s own tests; that one's doc comment carries the full
/// argument.
#[tokio::test]
async fn every_browser_route_is_reachable_without_a_merchant_token() -> anyhow::Result<()> {
    let h = harness().await?;
    let http = browser();

    assert_eq!(
        vpay_api::BROWSER_ROUTES.len(),
        5,
        "the payer-facing surface is two payment-intent routes and three checkout reads — \
         nothing may be added here without deciding what a payer may do with it"
    );
    let writes: Vec<&str> = vpay_api::BROWSER_ROUTES
        .iter()
        .filter(|route| route.methods.iter().any(|method| *method != "GET"))
        .map(|route| route.path)
        .collect();
    assert_eq!(
        writes,
        vec!["/payment_intents/{id}/confirm"],
        "a payer may confirm one intent and do nothing else on this surface"
    );

    /// Fills a route pattern's `{id}` with an id of the right *kind*, so a
    /// `cs_` route is not probed with a `pi_` id — which would be refused for
    /// the right answer and the wrong reason.
    fn concrete(path: &str) -> String {
        if path.starts_with("/checkout/sessions") {
            path.replace("{id}", "cs_00000000000000000000000x")
        } else {
            path.replace("{id}", "pi_00000000000000000000000x")
        }
    }

    let mut checked = 0_usize;
    for route in vpay_api::BROWSER_ROUTES {
        let path = concrete(route.path);
        for method in route.methods {
            let url = h.url(&format!("/v1/browser{path}"));
            let request = match *method {
                "GET" => http.get(&url),
                "POST" => http
                    .post(&url)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(""),
                other => panic!("this test does not know how to send {other}"),
            };
            let response = request
                .send()
                .await
                .with_context(|| format!("{method} {url}"))?;
            let status = response.status().as_u16();
            assert_ne!(
                status, 401,
                "{method} /v1/browser{path} must not demand a token a payer can never hold"
            );

            if route.path == "/checkout/origins" {
                // The one route with no object to be missing: it answers a
                // *tenant's* origins, so an absent key gets the same empty
                // list a registered tenant with none gets. That is the
                // fail-closed answer and the non-enumerable one — a 404 here
                // would tell a caller a key was unrecognised.
                assert_eq!(status, 200, "{method} /v1/browser{path}");
                assert_eq!(
                    response.json::<Value>().await?,
                    serde_json::json!({ "origins": [] }),
                    "no key means no origins, not an error a prober could learn from"
                );
            } else {
                // The uniform 404, decided by this surface's own credential
                // gate rather than by any authentication layer.
                assert_eq!(status, 404, "{method} /v1/browser{path}");
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 5, "five routes, one method each");

    // The contrast that makes the separation real rather than nominal: the
    // *same* relative path is behind the token boundary under `/v1` and
    // outside it under `/v1/browser`. If the browser routes were ever folded
    // into `V1_ROUTES`, the second answer here would become 401 — and
    // `payment_intents.rs`'s boundary walk would insist that was correct.
    //
    // Two of the five paths (`/checkout/sessions/{id}/return` and
    // `/checkout/origins`) are mounted nowhere under `/v1`. They still answer
    // 401 there, and that is the point: the `/v1` nest's authentication layer
    // runs before axum matches a route, so *every* path under that prefix is
    // behind the boundary — which is exactly why the browser nest has to be
    // mounted outside it rather than as a path within it.
    for route in vpay_api::BROWSER_ROUTES {
        let path = concrete(route.path);
        for method in route.methods {
            // The route's *own* verb: a `GET` at the confirm path would be
            // axum's bare 405 on the browser nest and would prove nothing.
            let build = |url: String| match *method {
                "GET" => http.get(url),
                "POST" => http
                    .post(url)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(""),
                other => panic!("this test does not know how to send {other}"),
            };
            let merchant = build(h.url(&format!("/v1{path}"))).send().await?;
            let payer = build(h.url(&format!("/v1/browser{path}"))).send().await?;
            assert_eq!(
                merchant.status().as_u16(),
                401,
                "{method} /v1{path} is the merchant surface and must demand a token"
            );
            assert_ne!(
                payer.status().as_u16(),
                401,
                "{method} /v1/browser{path} must be decided by this surface's own credential gate"
            );
        }
    }

    h.shutdown().await;
    Ok(())
}

/// Claim 7: the credential reaches the two responses a merchant's page needs
/// and **no others**.
///
/// The list is the case that matters most in practice: one page would hand a
/// merchant's integration the live browser credential for every intent on it,
/// and a list response is the one most likely to be logged wholesale.
#[tokio::test]
async fn the_client_secret_is_on_create_and_retrieve_and_never_on_the_list() -> anyhow::Result<()> {
    let h = harness().await?;
    let (id, secret) = create_intent(&h).await?;

    assert!(
        secret.starts_with(&format!("{id}_secret_")),
        "the credential must be the id plus the separator plus the suffix"
    );

    // The merchant's own retrieve carries it too, so a merchant who lost the
    // create response can recover it without creating a second intent.
    let retrieved: Value = browser()
        .get(h.url(&format!("/v1/payment_intents/{id}")))
        .bearer_auth(h.bearer(CLIENT_A))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        retrieved.get("client_secret").and_then(Value::as_str),
        Some(secret.as_str()),
        "{retrieved:#}"
    );

    let listed: Value = browser()
        .get(h.url("/v1/payment_intents"))
        .bearer_auth(h.bearer(CLIENT_A))
        .send()
        .await?
        .json()
        .await?;
    let data = listed
        .get("data")
        .and_then(Value::as_array)
        .context("the list envelope carries `data`")?;
    assert!(!data.is_empty(), "the list must not be vacuously clean");
    for item in data {
        assert!(
            item.get("client_secret").is_none(),
            "a list page must not carry a payer credential: {item:#}"
        );
        assert_eq!(
            item.as_object().map(serde_json::Map::len),
            Some(12),
            "the listed object is the twelve documented keys: {item:#}"
        );
    }
    // Not merely absent as a key — absent as a *value* anywhere in the page.
    let rendered = serde_json::to_string(&listed)?;
    assert!(
        !rendered.contains(&secret),
        "the credential must not appear anywhere in a list page"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 7, the half that would be worst to get wrong: no `client_secret`
/// reaches `events.data`.
///
/// An event body is signed, delivered at-least-once to a merchant's endpoint
/// over the public internet, and stored in `events` forever. The event here
/// is written by the **real** settlement transaction
/// (`vpay_db::settlement::apply_failed`, through the shipping worker loop),
/// not staged: `intent_snapshot` renders it through the same
/// `PaymentIntentObject` a `GET` returns, so a field added to that type would
/// land here by construction and this is what would notice.
///
/// The charge is pointed at the rail's decline reference before the poll for
/// `worker_e2e.rs`'s reason: the confirm mints its own reference and a seam
/// to fix it from a test would be a code path that exists only outside
/// production. Which terminal answer produced the event does not matter — the
/// question is what is *in* it.
#[tokio::test]
async fn no_event_body_carries_a_client_secret() -> anyhow::Result<()> {
    let h = harness().await?;
    let (id, secret) = create_intent(&h).await?;

    let (status, body) = browser_confirm(&h, &id, PK_A, &secret).await?;
    assert_eq!(status, 200, "{body:#}");

    sqlx::query("UPDATE charges SET provider_reference_id = $2 WHERE payment_intent_id = $1")
        .bind(&id)
        .bind(DECLINED_REF)
        .execute(&h.pool)
        .await
        .context("pointing the charge at the rail's decline stub")?;

    let endpoints = support::no_webhook_endpoints();
    let egress = support::default_egress_policy();
    let settled = vpay_worker::run_once(
        h.repositories.as_ref(),
        &h.adapters,
        &h.rails,
        &RecoveryPolicy::default(),
        &vpay_worker::WebhookContext {
            endpoints: &endpoints,
            egress,
        },
        "browser-checkout-suite",
    )
    .await?
    .context("the browser confirm enqueued no poll job")?;
    assert_eq!(settled.disposition, vpay_worker::Disposition::Finished);

    let rows: Vec<(String, Value)> =
        sqlx::query_as("SELECT type::TEXT, data FROM events WHERE object_id = $1 ORDER BY seq")
            .bind(&id)
            .fetch_all(&h.pool)
            .await
            .context("reading the events the settlement wrote")?;
    assert!(
        !rows.is_empty(),
        "the settlement must have written an event, or this test proves nothing"
    );
    for (event_type, data) in &rows {
        assert!(
            data.get("client_secret").is_none(),
            "{event_type}: a payer credential must never be in a webhook body: {data:#}"
        );
        assert_eq!(
            data.as_object().map(serde_json::Map::len),
            Some(12),
            "{event_type}: events.data is the twelve documented keys: {data:#}"
        );
        assert!(
            !serde_json::to_string(data)?.contains(&secret),
            "{event_type}: the credential must not appear anywhere in the snapshot"
        );
    }

    h.shutdown().await;
    Ok(())
}

/// The credential must not reach an operator's log either, and the row is
/// what carries it everywhere inside this process.
///
/// A unit test in `vpay-db` pins the `Debug` impl in isolation; this one
/// asserts it against a row that came **back out of Postgres**, so a column
/// that was added to `COLUMNS` without being added to the redaction would
/// fail here rather than in a fixture nobody re-derives.
#[tokio::test]
async fn a_stored_rows_debug_output_never_carries_the_client_secret() -> anyhow::Result<()> {
    let h = harness().await?;
    let (id, secret) = create_intent(&h).await?;

    let row = PaymentIntents::get_by_id(h.repositories.as_ref(), &id)
        .await?
        .context("the created intent is stored")?;

    let suffix = secret
        .split_once("_secret_")
        .map(|(_, suffix)| suffix.to_owned())
        .context("the credential carries the separator")?;
    assert_eq!(
        row.client_secret_suffix, suffix,
        "the stored half must be exactly what `create` rendered"
    );

    let formatted = format!("{row:?}");
    assert!(
        !formatted.contains(&suffix),
        "the stored suffix must not reach a Debug output"
    );
    assert!(
        !formatted.contains(&secret),
        "the joined credential must not reach a Debug output"
    );
    assert!(
        formatted.contains("redacted"),
        "the redaction must be visible, not a silently dropped field"
    );
    // And the row is still useful to whoever is reading the log.
    assert!(
        formatted.contains(&id),
        "the row's own id must still be visible in Debug output"
    );
    assert!(
        formatted.contains(MERCHANT_A),
        "the merchant id must still be visible in Debug output"
    );

    h.shutdown().await;
    Ok(())
}

/// **The browser nest is a fourth counted group.** `crate::router`'s
/// `track_http_metrics` is mounted inside `/v1/browser` for the same reason
/// it is mounted inside `/v1/oauth` and `/v1`: a browser request's route
/// *pattern* — `/v1/browser/payment_intents/{id}`, not the concrete
/// `pi_…` id a payer's page put in the URL — only exists once this nest has
/// matched, so counting it on the outer router (before the nests) would
/// either miss it entirely or label it `unmatched`.
///
/// Driven through a real `GET` with a real key and secret, so the assertion
/// pins the whole path — the nest, the middleware, the nest's own
/// `track_http_metrics` layer, and the handler answering `200` — rather than
/// a unit test's synthetic recorder. `vpay-api`'s own `lib.rs` unit tests
/// cover the same middleware against `/healthz`, `/v1/oauth` and `/v1`; this
/// is the sibling that needs a real database and therefore lives here.
#[tokio::test]
async fn a_browser_get_is_counted_under_its_own_route_pattern() -> anyhow::Result<()> {
    let h = harness().await?;
    let (metrics_addr, metrics_task) = serve_metrics().await?;

    let (id, secret) = create_intent(&h).await?;
    let (status, _body) = browser_retrieve(&h, &id, PK_A, &secret).await?;
    assert_eq!(status, 200, "a valid key and secret must retrieve cleanly");

    let scrape = reqwest::get(format!("http://{metrics_addr}/metrics"))
        .await
        .context("scraping /metrics off the observability listener")?
        .text()
        .await
        .context("reading the scrape body")?;
    assert!(
        scrape.contains(
            r#"vpay_http_requests_total{route="/v1/browser/payment_intents/{id}",method="GET",status="200"} 1"#
        ),
        "the browser GET must be counted under its route pattern:\n{scrape}"
    );
    assert!(
        !scrape.contains(&id),
        "the concrete payment intent id must never become a label value: {scrape}"
    );

    metrics_task.abort();
    h.shutdown().await;
    Ok(())
}
