//! Checkout Sessions, end to end: a merchant creates one from its server, a
//! payer's browser drives it, a rail settles the payment, and the session
//! moves with the intent.
//!
//! ```text
//!   raw reqwest + a merchant bearer token   (the merchant's own server)
//!     -> POST /v1/payment_intents           -> pi_…
//!     -> POST /v1/checkout/sessions         -> cs_…, url = {base}/c/cs_…#cs_…_secret_…
//!   raw reqwest, as a browser would         (vpay's own checkout page)
//!     -> GET /v1/browser/checkout/origins?key=pk_…
//!     -> GET /v1/browser/checkout/sessions/cs_…?key&client_secret
//!            -> the session, payment_intent expanded WITH its client_secret
//!     -> POST /v1/browser/payment_intents/pi_…/confirm   (the existing route)
//!            -> charge + poll job -> MTN adapter -> HTTP -> WireMock
//!     -> GET /v1/browser/checkout/sessions/cs_…/return?key&t=…
//!            -> the session, payment_intent expanded WITHOUT its secret
//!   vpay_worker::run_once                   (the shipping loop)
//!     -> settlement transaction -> session `complete`/`paid`, in one commit
//! ```
//!
//! # What this file claims
//!
//! 1. `create` answers `201` with the `client_secret` and a `url` whose
//!    credential is in the **fragment**, and `retrieve` answers the same —
//!    while the **list** carries neither;
//! 2. every credential failure on the two browser reads is the identical
//!    404, byte for byte, including the tenancy case;
//! 3. every URL rule the wire contract states is enforced at `create`, with
//!    the parameter named;
//! 4. the session read renders the **intent's** `client_secret` and the
//!    return read does not — the escalation D6 exists to prevent;
//! 5. the origins route answers by publishable key alone, and an unknown key
//!    is indistinguishable from a registered one with no origins;
//! 6. the **real** settlement transaction flips the session in the same
//!    commit as the intent — proven by running the shipping worker loop, not
//!    by staging a row;
//! 7. `expire` is a compare-and-swap, and a live charge refuses it;
//! 8. one open session per intent, enforced whether or not the pre-check
//!    sees it;
//! 9. a deployment with no `checkout.public_base_url` refuses to create a
//!    session rather than minting a `url` that resolves to nothing.
//!
//! # No test doubles
//!
//! Real Postgres, a real WireMock rail (the shared
//! `backends/tests/conformance/wiremock` tree), the shipping adapters, the
//! shipping router and the shipping worker loop.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use serde_json::Value;
use sqlx::PgPool;
use testcontainers::{ContainerAsync, GenericImage};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_config::{
    CheckoutConfig, Config, CurrencyEntry, Deployment, HostEntry, MERCHANT_AUDIENCE, ProviderHost,
};
use vpay_db::Repositories;
use vpay_worker::{Adapters, RailConfigs, RecoveryPolicy};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client_with_checkout,
    merchant_client_with_display_name, migrated_postgres, rail_configs, serve,
};

/// The merchant whose payer drives this surface, and the tenant it acts for.
const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";

/// The second merchant, for the one case that needs a *valid* publishable key
/// presented against someone else's session.
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

const PK_A: &str = "pk_test_acmecameroonsandbox01";
/// Merchant A's **second** registered key. Listed after `PK_A` in the
/// registration and sorting after it too, so the "first configured key"
/// default and an alphabetical accident are told apart by
/// `a_session_pins_the_tenants_first_key_unless_the_merchant_names_another`
/// rather than by luck.
const PK_A2: &str = "pk_test_acmecameroonsandbox02";
const PK_B: &str = "pk_test_betadoualasandbox0001";

/// Well-shaped and registered nowhere. Deliberately well-shaped: a malformed
/// key is refused at boot, so the only unknown key a payer can present is one
/// that could have been real.
const PK_UNKNOWN: &str = "pk_test_neverregisteredhere01";

/// Merchant A's embedding origins — the answer the checkout app's
/// `middleware.ts` turns into `frame-ancestors`.
const ORIGIN_A: &str = "https://shop.acme.example";
const ORIGIN_A2: &str = "https://checkout.acme.example:8443";

/// Where this deployment says its checkout app lives. Not the API's own
/// `public_base_url`: they are two deployables, and a suite that conflated
/// them would not notice a `url` built from the wrong one.
const CHECKOUT_BASE: &str = "https://checkout.vpay.test";

/// What merchant A's payers are told they are paying. Deliberately nothing
/// like [`MERCHANT_A`], so an assertion on it cannot pass by the fallback.
const DISPLAY_NAME_A: &str = "Boutique Acme Cameroun";

const PUSH_RAIL: &str = "mtn_momo";
const CURRENCY: &str = "xaf";
const AMOUNT: i64 = 5000;

/// A documentation MSISDN nothing stubs, so a confirm with it falls through
/// to `requesttopay.json`'s catch-all `202` — and the status stub's own
/// catch-all then answers `SUCCESSFUL`, which is what the settlement case
/// needs.
const MSISDN: &str = "237670000000";

const SUCCESS_URL: &str = "https://shop.acme.example/ok?sid={CHECKOUT_SESSION_ID}";
const CANCEL_URL: &str = "https://shop.acme.example/cancel";
const RETURN_URL: &str = "https://shop.acme.example/embedded/done";

fn mappings_dir(rail: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../conformance/wiremock")
        .join(rail)
}

// ------------------------------------------------------------------ harness

struct Harness {
    _postgres: ContainerAsync<PostgresImage>,
    _mtn: ContainerAsync<GenericImage>,
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    pool: PgPool,
    base_url: String,
    signing_key: LoadedSigningKey,
    adapters: Arc<Adapters>,
    rails: Arc<RailConfigs>,
}

impl Harness {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// A `/v1` bearer token for `client_id`, minted with the server's own
    /// signer.
    ///
    /// Raw HTTP rather than `vpay-sdk` throughout this suite, unlike
    /// `browser_checkout.rs`'s merchant half: the Rust SDK models no
    /// `checkout.sessions` resource yet (that is lane 5's work), so there is
    /// nothing of it to exercise here. Stated rather than left implicit —
    /// this file proves the *server*, and lane 5's own tests prove the SDK
    /// speaks to it.
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
/// publishable key and two origins, one with a key and none), one rail
/// pointed at the stub, one currency, and a checkout app.
fn config_with(
    base_url: &str,
    mtn_url: &str,
    jwks_a: Value,
    jwks_b: Value,
    checkout_base: Option<&str>,
    merchant_b_has_a_key: bool,
) -> Config {
    Config {
        deployment: Deployment {
            name: "checkout-sessions".to_owned(),
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
            merchant_client_with_display_name(
                CLIENT_A,
                MERCHANT_A,
                jwks_a,
                &[PK_A, PK_A2],
                // Only when checkout is configured: a merchant with origins
                // and no `checkout.public_base_url` is refused at boot, which
                // is exactly what `a_deployment_without_a_checkout_app_…`
                // relies on being able to avoid.
                if checkout_base.is_some() {
                    &[ORIGIN_A, ORIGIN_A2]
                } else {
                    &[]
                },
                // Merchant A configures a name; merchant B below does not, so
                // both halves of `ResourceConfig::merchant_display_name` are
                // exercised by the same deployment.
                DISPLAY_NAME_A,
            ),
            // Registered, keyed, and with **no** origins — the fail-closed
            // shape, and the case that makes "an unknown key is
            // indistinguishable from an empty one" a real comparison rather
            // than a tautology.
            merchant_client_with_checkout(
                CLIENT_B,
                MERCHANT_B,
                jwks_b,
                if merchant_b_has_a_key { &[PK_B] } else { &[] },
                &[],
            ),
        ],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: CheckoutConfig {
            public_base_url: checkout_base.map(str::to_owned),
        },
        dashboard_client: None,
    }
}

async fn harness() -> anyhow::Result<Harness> {
    harness_with_checkout(Some(CHECKOUT_BASE)).await
}

/// The same deployment, with merchant B holding **no** publishable key.
///
/// A separate constructor rather than a parameter on every call, for
/// `merchant_client_with_checkout`'s reason: exactly one test needs a keyless
/// tenant, and the shape every other test wants is the one where both
/// merchants are reachable.
async fn harness_with_keyless_merchant_b() -> anyhow::Result<Harness> {
    harness_with(Some(CHECKOUT_BASE), false).await
}

async fn harness_with_checkout(checkout_base: Option<&'static str>) -> anyhow::Result<Harness> {
    harness_with(checkout_base, true).await
}

async fn harness_with(
    checkout_base: Option<&'static str>,
    merchant_b_has_a_key: bool,
) -> anyhow::Result<Harness> {
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
    let (_pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();

    let mtn_for_config = mtn_url.clone();
    let (jwks_a_for_server, jwks_b_for_server) = (jwks_a.clone(), jwks_b.clone());
    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(
            base_url,
            &mtn_for_config,
            jwks_a_for_server,
            jwks_b_for_server,
            checkout_base,
            merchant_b_has_a_key,
        )
    })
    .await?;
    // Rebuilt from the bound URL rather than smuggled out of the closure, as
    // `browser_checkout.rs` does it: the worker's `RailConfigs` must be the
    // projection of the *same* configuration the server booted with.
    let config = config_with(
        &served.base_url,
        &mtn_url,
        jwks_a,
        jwks_b,
        checkout_base,
        merchant_b_has_a_key,
    );

    Ok(Harness {
        _postgres: postgres,
        _mtn: mtn,
        server: served.server,
        repositories,
        pool,
        base_url: served.base_url,
        signing_key: served.signing_key,
        adapters: Arc::new(support::adapters_by_code()),
        rails: Arc::new(rail_configs(&config)),
    })
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

/// `POST /v1/payment_intents` as merchant A's own server does it.
async fn create_intent(h: &Harness) -> anyhow::Result<String> {
    create_intent_for(h, CLIENT_A).await
}

/// The same, for whichever tenant's credential is named — the two-merchant
/// cases need an intent that belongs to the *other* one, and a session cannot
/// be created against an intent the caller does not own.
async fn create_intent_for(h: &Harness, client_id: &str) -> anyhow::Result<String> {
    // A raw body rather than `RequestBuilder::form`: the array key
    // `payment_method_types[0]` is the wire shape both merchant SDKs send
    // (`sdks/rust/src/form.rs`), and `form` percent-encodes the brackets,
    // which `vpay_api::form` reads as a scalar field named
    // `payment_method_types[0]` rather than as the first element of a list.
    // Every value here is URL-safe by construction.
    let body: Value = browser()
        .post(h.url("/v1/payment_intents"))
        .bearer_auth(h.bearer(client_id))
        .header("idempotency-key", uuid::Uuid::new_v4().to_string())
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "amount={AMOUNT}&currency={CURRENCY}&payment_method_types[0]={PUSH_RAIL}"
        ))
        .send()
        .await
        .context("creating a payment intent")?
        .json()
        .await
        .context("the create body is JSON")?;
    body.get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("the create must answer an intent id: {body:#}"))
}

/// An intent and the `client_secret` `create` rendered for it (D2), for the
/// one case that confirms an intent **no session was ever created for** — the
/// only place a browser credential does not come out of a session read.
async fn create_intent_with_secret(h: &Harness) -> anyhow::Result<(String, String)> {
    let body: Value = browser()
        .post(h.url("/v1/payment_intents"))
        .bearer_auth(h.bearer(CLIENT_A))
        .header("idempotency-key", uuid::Uuid::new_v4().to_string())
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "amount={AMOUNT}&currency={CURRENCY}&payment_method_types[0]={PUSH_RAIL}"
        ))
        .send()
        .await
        .context("creating a payment intent")?
        .json()
        .await
        .context("the create body is JSON")?;
    Ok((field(&body, "id")?, field(&body, "client_secret")?))
}

/// `POST /v1/checkout/sessions`, with whatever fields the caller names.
async fn create_session(
    h: &Harness,
    client_id: &str,
    fields: &[(&str, &str)],
) -> anyhow::Result<(u16, Value)> {
    let response = browser()
        .post(h.url("/v1/checkout/sessions"))
        .bearer_auth(h.bearer(client_id))
        .header("idempotency-key", uuid::Uuid::new_v4().to_string())
        .form(fields)
        .send()
        .await
        .context("creating a checkout session")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    Ok((status, body))
}

/// A hosted session on a fresh intent, and the three values every test needs.
struct Session {
    id: String,
    secret: String,
    intent_id: String,
    url: String,
}

async fn hosted_session(h: &Harness) -> anyhow::Result<Session> {
    let intent_id = create_intent(h).await?;
    let (status, body) = create_session(
        h,
        CLIENT_A,
        &[
            ("payment_intent", intent_id.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    anyhow::ensure!(status == 201, "creating a hosted session: {body:#}");
    Ok(Session {
        id: field(&body, "id")?,
        secret: field(&body, "client_secret")?,
        intent_id,
        url: field(&body, "url")?,
    })
}

fn field(body: &Value, key: &str) -> anyhow::Result<String> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("`{key}` must be a string: {body:#}"))
}

/// The **return token**, read straight out of Postgres.
///
/// It is deliberately on no response — vpay builds the one URL that carries
/// it and hands that to the *rail* (lane 2), so a merchant never sees it and
/// there is nothing on the wire for a test to read. Going to the column is
/// what a payer arriving from Orange effectively has, without this suite
/// having to stand up a redirect rail.
async fn return_token(pool: &PgPool, session_id: &str) -> anyhow::Result<String> {
    let (token,): (String,) =
        sqlx::query_as("SELECT return_token FROM checkout_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(pool)
            .await
            .context("reading the session's return token")?;
    Ok(token)
}

async fn stored_session(pool: &PgPool, id: &str) -> anyhow::Result<(String, String)> {
    let row: (String, String) =
        sqlx::query_as("SELECT status, payment_status FROM checkout_sessions WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .context("reading the session")?;
    Ok(row)
}

async fn session_read(
    h: &Harness,
    id: &str,
    key: &str,
    secret: &str,
) -> anyhow::Result<(u16, Value)> {
    let response = browser()
        .get(h.url(&format!("/v1/browser/checkout/sessions/{id}")))
        .query(&[("key", key), ("client_secret", secret)])
        .send()
        .await
        .context("the browser session read")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    Ok((status, body))
}

async fn return_read(
    h: &Harness,
    id: &str,
    key: &str,
    token: &str,
) -> anyhow::Result<(u16, Value)> {
    let response = browser()
        .get(h.url(&format!("/v1/browser/checkout/sessions/{id}/return")))
        .query(&[("key", key), ("t", token)])
        .send()
        .await
        .context("the browser return read")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    Ok((status, body))
}

/// A browser confirm through the route that already existed — the point being
/// that Step 9 added no second way to move money.
async fn browser_confirm(h: &Harness, intent_id: &str, intent_secret: &str) -> anyhow::Result<u16> {
    Ok(browser_confirm_response(h, intent_id, intent_secret)
        .await?
        .0)
}

/// The same call, with the envelope kept.
///
/// A separate function rather than a wider [`browser_confirm`], because most
/// cases here only ever assert the status and threading a body they ignore
/// through every one of them would bury the two that read a `code`.
async fn browser_confirm_response(
    h: &Harness,
    intent_id: &str,
    intent_secret: &str,
) -> anyhow::Result<(u16, Value)> {
    let response = browser()
        .post(h.url(&format!("/v1/browser/payment_intents/{intent_id}/confirm")))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "key={PK_A}&client_secret={intent_secret}&payment_method_data[type]={PUSH_RAIL}\
             &payment_method_data[{PUSH_RAIL}][msisdn]={MSISDN}"
        ))
        .send()
        .await
        .context("confirming through the browser surface")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    Ok((status, body))
}

/// `POST /v1/payment_intents/{id}/confirm` as the **merchant's** own server
/// does it, under an `Idempotency-Key` the caller chooses.
///
/// The caller chooses the key so a case can send the *same* one twice and
/// read the replay, which is the half of this surface the browser one does
/// not have (`docs/flows/browser-checkout.md`, "No `Idempotency-Key`").
///
/// A raw body rather than `RequestBuilder::form`, for
/// [`create_intent_for`]'s reason: `form` percent-encodes the brackets, and
/// `vpay_api::form` then reads `payment_method_data[type]` as a scalar field
/// rather than as structure.
async fn merchant_confirm(
    h: &Harness,
    intent_id: &str,
    idempotency_key: &str,
) -> anyhow::Result<(u16, Value)> {
    let response = browser()
        .post(h.url(&format!("/v1/payment_intents/{intent_id}/confirm")))
        .bearer_auth(h.bearer(CLIENT_A))
        .header("idempotency-key", idempotency_key)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "payment_method_data[type]={PUSH_RAIL}\
             &payment_method_data[{PUSH_RAIL}][msisdn]={MSISDN}"
        ))
        .send()
        .await
        .context("confirming through the merchant surface")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    Ok((status, body))
}

/// `GET /v1/payment_intents/{id}` through the shipping router, as the
/// merchant that owns it.
async fn retrieve_intent(h: &Harness, intent_id: &str) -> anyhow::Result<(u16, Value)> {
    let response = browser()
        .get(h.url(&format!("/v1/payment_intents/{intent_id}")))
        .bearer_auth(h.bearer(CLIENT_A))
        .send()
        .await
        .context("retrieving a payment intent")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    Ok((status, body))
}

/// Everything a confirm writes before it reaches a rail, counted: the charge
/// rows for this intent, the `provider_requests` rows hanging off them, and
/// the poll jobs in the whole deployment.
///
/// The third is counted deployment-wide on purpose. A poll job is keyed on a
/// **charge** id (`vpay_worker::jobs::poll_dedupe_key`), so scoping the count
/// to this intent would need a charge to scope by — and the claim being made
/// is that there is no charge. Every case that asserts against this creates
/// exactly one intent and never completes a confirm, so a non-zero count is
/// unambiguous.
async fn write_footprint(pool: &PgPool, intent_id: &str) -> anyhow::Result<(i64, i64, i64)> {
    let (charges,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM charges WHERE payment_intent_id = $1")
            .bind(intent_id)
            .fetch_one(pool)
            .await
            .context("counting the charges of an intent")?;
    let (requests,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM provider_requests r \
         JOIN charges c ON c.id = r.charge_id \
         WHERE c.payment_intent_id = $1",
    )
    .bind(intent_id)
    .fetch_one(pool)
    .await
    .context("counting the provider requests of an intent")?;
    let (jobs,): (i64,) = sqlx::query_as("SELECT count(*) FROM jobs WHERE kind = 'poll_charge'")
        .fetch_one(pool)
        .await
        .context("counting the poll jobs")?;
    Ok((charges, requests, jobs))
}

/// `status`, `payment_status` and `updated_at` — the third being what proves
/// a *read* decided something rather than a write.
async fn stored_session_stamp(
    pool: &PgPool,
    id: &str,
) -> anyhow::Result<(String, String, time::OffsetDateTime)> {
    let row: (String, String, time::OffsetDateTime) = sqlx::query_as(
        "SELECT status, payment_status, updated_at FROM checkout_sessions WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .context("reading the session's stamp")?;
    Ok(row)
}

/// `POST /v1/checkout/sessions/{id}/expire` as the merchant's own server.
async fn expire_session(h: &Harness, id: &str) -> anyhow::Result<(u16, Value)> {
    let response = browser()
        .post(h.url(&format!("/v1/checkout/sessions/{id}/expire")))
        .bearer_auth(h.bearer(CLIENT_A))
        .header("idempotency-key", uuid::Uuid::new_v4().to_string())
        .header("content-type", "application/x-www-form-urlencoded")
        .body("")
        .send()
        .await
        .context("expiring a checkout session")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    Ok((status, body))
}

/// The `error.code` of an envelope.
///
/// The body itself is deliberately **not** in the failure message. A browser
/// confirm and every `/v1` checkout-session response render a live
/// `client_secret` (`SecretRendering::Include`, and the hosted `url` carries
/// the session secret in its fragment), so interpolating one into an
/// assertion message would print a credential into CI's logs the one moment
/// the assertion fails — the same `rust/cleartext-logging` fix
/// `docs/status.md` records for `vpay-config`'s `Debug` test.
fn error_code(body: &Value) -> anyhow::Result<String> {
    body.pointer("/error/code")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .context("the body must be the error envelope")
}

/// [`error_code`] for a failure *message*: the envelope's code, or a
/// placeholder when the body is not an envelope. Never the body.
fn error_code_or_none(body: &Value) -> &str {
    body.pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("<not an error envelope>")
}

/// Runs the shipping worker loop until it has no more work, so the settlement
/// this suite asserts is the one a deployment actually performs.
async fn drain_worker(h: &Harness) -> anyhow::Result<usize> {
    let endpoints = support::no_webhook_endpoints();
    let egress = support::default_egress_policy();
    let mut ran = 0_usize;
    for _ in 0..10 {
        support::make_every_job_runnable(&h.pool).await?;
        let outcome = vpay_worker::run_once(
            h.repositories.as_ref(),
            &h.adapters,
            &h.rails,
            &RecoveryPolicy::default(),
            &vpay_worker::WebhookContext {
                endpoints: &endpoints,
                egress,
            },
            "checkout-sessions-suite",
        )
        .await?;
        match outcome {
            Some(claimed) => {
                ran += 1;
                if claimed.disposition == vpay_worker::Disposition::Finished {
                    break;
                }
            }
            None => break,
        }
    }
    Ok(ran)
}

// ---------------------------------------------------------------- the tests

/// Claim 1: `create` answers the credential and a `url` whose secret is in
/// the **fragment**; `retrieve` answers the same; the list answers neither.
///
/// The fragment is the whole of D6 for the hosted mode: a `?` there would put
/// a live payer credential into the checkout app's access log, its `Referer`
/// headers and every proxy in between. Asserted as "the credential does not
/// appear before the `#`" rather than as a string match on the whole URL, so
/// a future base URL that happened to contain the secret's characters could
/// not make it pass.
#[tokio::test]
async fn a_hosted_session_answers_a_url_whose_secret_is_in_the_fragment() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;

    assert!(
        session
            .secret
            .starts_with(&format!("{}_secret_", session.id)),
        "the credential is the session id, the separator and the suffix (secret never printed; \
         it is {} chars for session {})",
        session.secret.len(),
        session.id
    );
    assert_eq!(
        session.url,
        format!(
            "{CHECKOUT_BASE}/c/{}?key={PK_A}#{}",
            session.id, session.secret
        ),
        "the publishable key is a query parameter — the page reads it server-side to set \
         frame-ancestors, where a fragment never arrives — and the credential is in the fragment"
    );
    let (before_fragment, fragment) = session
        .url
        .split_once('#')
        .context("a hosted url carries a fragment")?;
    assert_eq!(fragment, session.secret);
    assert!(
        !before_fragment.contains(&session.secret),
        "the credential must not appear before the fragment: {}",
        session.url
    );
    assert!(
        before_fragment.ends_with(&format!("?key={PK_A}")),
        "the tenant's first configured key, with nothing else in the query string: {}",
        session.url
    );
    assert!(
        !fragment.contains('?') && !fragment.contains('&'),
        "the fragment is the credential and nothing else: {}",
        session.url
    );

    // `retrieve` recovers both, so a merchant who lost the create response
    // does not have to create a second session.
    let retrieved: Value = browser()
        .get(h.url(&format!("/v1/checkout/sessions/{}", session.id)))
        .bearer_auth(h.bearer(CLIENT_A))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(field(&retrieved, "client_secret")?, session.secret);
    assert_eq!(field(&retrieved, "url")?, session.url);
    assert_eq!(field(&retrieved, "object")?, "checkout.session");
    assert_eq!(field(&retrieved, "status")?, "open");
    assert_eq!(field(&retrieved, "payment_status")?, "unpaid");
    // The merchant surface renders the intent as an id, never expanded.
    assert_eq!(field(&retrieved, "payment_intent")?, session.intent_id);
    // D5: echoed back exactly as written, never substituted.
    assert_eq!(field(&retrieved, "success_url")?, SUCCESS_URL);

    // The list carries neither the credential nor the url that carries it.
    let listed: Value = browser()
        .get(h.url("/v1/checkout/sessions"))
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
            item.get("url"),
            Some(&Value::Null),
            "…nor the url that carries the same credential in its fragment: {item:#}"
        );
    }
    let rendered = serde_json::to_string(&listed)?;
    assert!(
        !rendered.contains(&session.secret),
        "the credential must not appear anywhere in a list page"
    );

    // …and the `payment_intent` filter narrows it.
    let filtered: Value = browser()
        .get(h.url("/v1/checkout/sessions"))
        .bearer_auth(h.bearer(CLIENT_A))
        .query(&[("payment_intent", session.intent_id.as_str())])
        .send()
        .await?
        .json()
        .await?;
    let filtered_data = filtered
        .get("data")
        .and_then(Value::as_array)
        .context("`data`")?;
    assert_eq!(filtered_data.len(), 1, "{filtered:#}");

    h.shutdown().await;
    Ok(())
}

/// Claim 2, and the security property of the whole browser surface: every
/// credential failure is the **identical** response.
///
/// Byte-for-byte, not merely "all 404", and the tenancy case
/// (merchant B's valid, registered key against merchant A's session) is the
/// one that makes it a tenancy check rather than a formality.
///
/// **Revert-proof.** Delete the `session.merchant_id != merchant_id` arm from
/// `browser::checkout_sessions::authenticate` and the "another merchant's
/// publishable key" row starts answering `200` with merchant A's session in
/// it — this test fails on the status and on the body.
#[tokio::test]
async fn every_credential_failure_on_the_checkout_surface_is_the_identical_404()
-> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    let token = return_token(&h.pool, &session.id).await?;

    // The reference answer: an id that never existed.
    let missing = "cs_00000000000000000000000x";
    let (status, baseline) = session_read(&h, missing, PK_A, "cs_x_secret_y").await?;
    assert_eq!(status, 404, "{baseline:#}");
    assert_eq!(
        baseline.pointer("/error/type").and_then(Value::as_str),
        Some("invalid_request_error"),
        "{baseline:#}"
    );
    assert_eq!(
        baseline.pointer("/error/code").and_then(Value::as_str),
        Some("resource_missing"),
        "{baseline:#}"
    );
    assert_eq!(
        baseline.pointer("/error/message").and_then(Value::as_str),
        Some(format!("No such checkout session: {missing}").as_str()),
        "{baseline:#}"
    );
    assert!(
        baseline.pointer("/error/param").is_none(),
        "the uniform 404 names no parameter — naming one would say which half was wrong: \
         {baseline:#}"
    );

    // The same body with the id substituted: the id is echoed, so the
    // comparison allows for that and nothing else.
    let rendered = serde_json::to_string(&baseline)?;
    let expected: Value = serde_json::from_str(&rendered.replace(missing, &session.id))?;

    let wrong_secret = format!("{}_secret_{}", session.id, "0".repeat(32));
    let cases: Vec<(&str, &str, String)> = vec![
        ("the right key, a wrong client_secret", PK_A, wrong_secret),
        (
            "the right key, a secret for a different session",
            PK_A,
            format!("cs_00000000000000000000000x_secret_{}", "a".repeat(32)),
        ),
        (
            "a publishable key no registration carries",
            PK_UNKNOWN,
            session.secret.clone(),
        ),
        (
            "another merchant's publishable key, valid and registered",
            PK_B,
            session.secret.clone(),
        ),
        ("no credential at all", "", String::new()),
        // The escalation D6 exists to prevent, from the other direction: the
        // *return token* must not open the session read, which is the route
        // that hands over the intent's own credential.
        (
            "the session's own return token, on the session read",
            PK_A,
            token.clone(),
        ),
    ];

    for (what, key, presented) in cases {
        let (status, body) = session_read(&h, &session.id, key, &presented).await?;
        assert_eq!(status, 404, "{what}: {body:#}");
        assert_eq!(body, expected, "{what}: the refusal must be identical");
    }

    // The return read refuses the same six shapes, with the same body — and
    // notably refuses the *session secret*, which is the stronger credential.
    let return_cases: Vec<(&str, &str, String)> = vec![
        ("a wrong return token", PK_A, "0".repeat(32)),
        (
            "a publishable key no registration carries",
            PK_UNKNOWN,
            token.clone(),
        ),
        ("another merchant's publishable key", PK_B, token.clone()),
        ("no credential at all", "", String::new()),
        (
            "the session's client_secret, where a return token is expected",
            PK_A,
            session.secret.clone(),
        ),
    ];
    for (what, key, presented) in return_cases {
        let (status, body) = return_read(&h, &session.id, key, &presented).await?;
        assert_eq!(status, 404, "{what} (return): {body:#}");
        assert_eq!(
            body, expected,
            "{what} (return): the refusal must be identical"
        );
    }

    // The envelope is the same shape the payment-intent browser surface
    // answers with, differing only in the noun a payer reads. The two
    // spellings are separate on purpose (`checkout session` vs
    // `payment intent`), so this compares the machine-readable half.
    let intent_404: Value = browser()
        .get(h.url("/v1/browser/payment_intents/pi_00000000000000000000000x"))
        .query(&[("key", PK_A), ("client_secret", "pi_x_secret_y")])
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        intent_404.pointer("/error/type"),
        baseline.pointer("/error/type")
    );
    assert_eq!(
        intent_404.pointer("/error/code"),
        baseline.pointer("/error/code")
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 3: every URL rule the wire contract states, enforced at `create`,
/// with the parameter named.
///
/// One table, because a *missing* rule shows up as a missing row. The
/// `refused` rows are the ones worth having: silently dropping a `return_url`
/// sent with `ui_mode: hosted` would pass every "the session was created"
/// test and would then forward a payer somewhere the merchant did not choose.
#[tokio::test]
async fn every_url_rule_is_enforced_at_create_and_names_its_parameter() -> anyhow::Result<()> {
    let h = harness().await?;

    /// One refused create: what it is, the fields beyond `payment_intent`,
    /// and the `param` the answer must name.
    ///
    /// A named alias only so the table below is not a `type_complexity`
    /// denial under the workspace's `-D warnings`; the tuple is the whole
    /// meaning, exactly as `vpay_core::ids`' `Generator` alias is.
    type UrlCase<'a> = (&'static str, Vec<(&'static str, &'a str)>, &'static str);

    let too_long = format!("https://shop.acme.example/{}", "a".repeat(2_048));
    let cases: Vec<UrlCase<'_>> = vec![
        (
            "hosted with no success_url",
            vec![("cancel_url", CANCEL_URL)],
            "success_url",
        ),
        (
            "hosted with no cancel_url",
            vec![("success_url", SUCCESS_URL)],
            "cancel_url",
        ),
        (
            "hosted with a return_url",
            vec![
                ("success_url", SUCCESS_URL),
                ("cancel_url", CANCEL_URL),
                ("return_url", RETURN_URL),
            ],
            "return_url",
        ),
        (
            "embedded with no return_url",
            vec![("ui_mode", "embedded")],
            "return_url",
        ),
        (
            "embedded with a success_url",
            vec![
                ("ui_mode", "embedded"),
                ("return_url", RETURN_URL),
                ("success_url", SUCCESS_URL),
            ],
            "success_url",
        ),
        (
            "embedded with a cancel_url",
            vec![
                ("ui_mode", "embedded"),
                ("return_url", RETURN_URL),
                ("cancel_url", CANCEL_URL),
            ],
            "cancel_url",
        ),
        (
            "a javascript: success_url",
            vec![
                ("success_url", "javascript:alert(1)"),
                ("cancel_url", CANCEL_URL),
            ],
            "success_url",
        ),
        (
            "a data: cancel_url",
            vec![
                ("success_url", SUCCESS_URL),
                ("cancel_url", "data:text/html,<script>alert(1)</script>"),
            ],
            "cancel_url",
        ),
        (
            "a scheme-relative success_url",
            vec![
                ("success_url", "//shop.acme.example/ok"),
                ("cancel_url", CANCEL_URL),
            ],
            "success_url",
        ),
        (
            "a success_url over 2048 characters",
            vec![
                ("success_url", too_long.as_str()),
                ("cancel_url", CANCEL_URL),
            ],
            "success_url",
        ),
        (
            "an unknown ui_mode",
            vec![
                ("ui_mode", "iframe"),
                ("success_url", SUCCESS_URL),
                ("cancel_url", CANCEL_URL),
            ],
            "ui_mode",
        ),
    ];

    for (what, extra, expected_param) in cases {
        // A fresh intent per case, so a refusal cannot be the one-open-
        // session rule firing on a leftover from the case before.
        let intent_id = create_intent(&h).await?;
        let mut fields = vec![("payment_intent", intent_id.as_str())];
        fields.extend(extra);

        let (status, body) = create_session(&h, CLIENT_A, &fields).await?;
        assert_eq!(status, 400, "{what}: {body:#}");
        assert_eq!(
            body.pointer("/error/param").and_then(Value::as_str),
            Some(expected_param),
            "{what}: the refusal must name the field the merchant sent: {body:#}"
        );
        assert_eq!(
            body.pointer("/error/type").and_then(Value::as_str),
            Some("invalid_request_error"),
            "{what}: {body:#}"
        );
    }

    // The `payment_intent` rules, on the same table.
    for (what, payment_intent) in [
        ("a missing payment_intent", ""),
        (
            "a session id where an intent id belongs",
            "cs_0123456789abcdefghjkmnpq",
        ),
        (
            "an intent that does not exist",
            "pi_0123456789abcdefghjkmnpq",
        ),
    ] {
        let (status, body) = create_session(
            &h,
            CLIENT_A,
            &[
                ("payment_intent", payment_intent),
                ("success_url", SUCCESS_URL),
                ("cancel_url", CANCEL_URL),
            ],
        )
        .await?;
        assert_eq!(status, 400, "{what}: {body:#}");
        assert_eq!(
            body.pointer("/error/param").and_then(Value::as_str),
            Some("payment_intent"),
            "{what}: {body:#}"
        );
    }

    // Another merchant's intent answers **identically** to one that does not
    // exist — the tenancy property, on a body parameter rather than a path.
    let intent_id = create_intent(&h).await?;
    let (foreign_status, foreign) = create_session(
        &h,
        CLIENT_B,
        &[
            ("payment_intent", intent_id.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    let (missing_status, missing) = create_session(
        &h,
        CLIENT_B,
        &[
            ("payment_intent", "pi_0123456789abcdefghjkmnpq"),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    assert_eq!(foreign_status, missing_status, "{foreign:#}");
    assert_eq!(
        foreign, missing,
        "another merchant's intent must be indistinguishable from one that does not exist"
    );

    // And a `POST` with no `Idempotency-Key` is refused before any of it —
    // D7 applies to this resource exactly as to every other `/v1` POST.
    let intent_id = create_intent(&h).await?;
    let response = browser()
        .post(h.url("/v1/checkout/sessions"))
        .bearer_auth(h.bearer(CLIENT_A))
        .form(&[
            ("payment_intent", intent_id.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ])
        .send()
        .await?;
    assert_eq!(
        response.status().as_u16(),
        400,
        "a /v1 POST without an Idempotency-Key is refused"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 4, and the one that would be worst to get wrong: the session read
/// hands over the **intent's** `client_secret` and the return read does not.
///
/// The escalation this closes: `return_token` (a query-string value, written
/// to access logs) → the session read → the intent's `client_secret` →
/// `confirm`. Every hop of it is refused, and the two that matter are
/// asserted here — the return token cannot open the session read (that case
/// is in `every_credential_failure_…`), and the return read renders no
/// credential.
///
/// **Revert-proof.** Change `retrieve_for_return` to build
/// `ExpandableIntent::ExpandedWithSecret` and the last three assertions fail.
#[tokio::test]
async fn the_session_read_carries_the_intents_secret_and_the_return_read_does_not()
-> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    let token = return_token(&h.pool, &session.id).await?;

    // The page's first call: the session, and the intent expanded with its
    // own credential.
    let (status, body) = session_read(&h, &session.id, PK_A, &session.secret).await?;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(field(&body, "id")?, session.id);
    assert_eq!(field(&body, "object")?, "checkout.session");
    // The url is **not** echoed on the browser surface: it carries the
    // session secret in its fragment, and the return read below must not be
    // able to recover it.
    assert_eq!(
        body.get("url"),
        Some(&Value::Null),
        "a browser read must not render the payer link: {body:#}"
    );

    let intent = body
        .get("payment_intent")
        .and_then(Value::as_object)
        .context("`payment_intent` is expanded on the browser reads")?;
    let intent_secret = intent
        .get("client_secret")
        .and_then(Value::as_str)
        .context("the session read must render the intent's client_secret")?
        .to_owned();
    assert!(intent_secret.starts_with(&format!("{}_secret_", session.intent_id)));
    // The fields the page cannot paint without, in one round trip.
    assert_eq!(intent.get("amount"), Some(&Value::from(AMOUNT)));
    assert_eq!(intent.get("currency"), Some(&Value::from(CURRENCY)));
    assert_eq!(
        intent.get("payment_method_types"),
        Some(&serde_json::json!([PUSH_RAIL]))
    );
    assert_eq!(
        intent.get("status"),
        Some(&Value::from("requires_payment_method"))
    );
    assert!(intent.contains_key("next_action"), "{body:#}");
    assert!(intent.contains_key("last_payment_error"), "{body:#}");

    // …and that credential really does drive the confirm route that already
    // existed. Step 9 added no second way to move money.
    assert_eq!(
        browser_confirm(&h, &session.intent_id, &intent_secret).await?,
        200
    );

    // The return page's call: the same expanded intent, and no credential.
    let (status, body) = return_read(&h, &session.id, PK_A, &token).await?;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(field(&body, "id")?, session.id);
    assert_eq!(
        body.get("url"),
        Some(&Value::Null),
        "the return read must not render the payer link either: {body:#}"
    );
    let intent = body
        .get("payment_intent")
        .and_then(Value::as_object)
        .context("`payment_intent` is expanded here too")?;
    assert!(
        !intent.contains_key("client_secret"),
        "the return read must not render the intent's credential: {body:#}"
    );
    assert_eq!(intent.len(), 12, "the twelve documented keys: {body:#}");
    // The outcome the return page renders is there.
    assert_eq!(intent.get("status"), Some(&Value::from("processing")));

    // Not merely absent as a key — absent as a *value* anywhere in the body,
    // and the session's own credential is absent too.
    let rendered = serde_json::to_string(&body)?;
    assert!(
        !rendered.contains(&intent_secret),
        "the intent's credential must not appear anywhere in the return read"
    );
    assert!(
        !rendered.contains(&session.secret),
        "…nor the session's, which the return token must not be exchangeable for"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 5: the origins route answers by publishable key alone, and an
/// unknown key is indistinguishable from a registered one with no origins.
///
/// The second half is the confidentiality property: an answer that told them
/// apart would let anyone enumerate which merchants a deployment serves by
/// trying keys — the same reason every other refusal on this surface is the
/// uniform 404. It is also the fail-closed answer: no origins means no
/// embedding.
#[tokio::test]
async fn the_origins_route_answers_by_key_alone_and_cannot_be_enumerated() -> anyhow::Result<()> {
    let h = harness().await?;

    let read = |key: &'static str| {
        let url = h.url("/v1/browser/checkout/origins");
        async move {
            let response = browser().get(url).query(&[("key", key)]).send().await?;
            let status = response.status().as_u16();
            let body: Value = response.json().await?;
            Ok::<_, anyhow::Error>((status, body))
        }
    };

    // No `client_secret` anywhere in the request, and a `200`.
    let (status, body) = read(PK_A).await?;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(
        body,
        serde_json::json!({ "origins": [ORIGIN_A, ORIGIN_A2] }),
        "the tenant's origins, in configuration order"
    );

    // A registered merchant with none, and a key nobody registered: the same
    // answer, byte for byte.
    let (empty_status, empty) = read(PK_B).await?;
    let (unknown_status, unknown) = read(PK_UNKNOWN).await?;
    assert_eq!(empty_status, 200);
    assert_eq!(unknown_status, 200);
    assert_eq!(empty, serde_json::json!({ "origins": [] }));
    assert_eq!(
        unknown, empty,
        "an unknown key must be indistinguishable from a registered one with no origins"
    );

    // No key at all is the same again — and, like the two above, is
    // fail-closed rather than an error a caller could learn from.
    let response = browser()
        .get(h.url("/v1/browser/checkout/origins"))
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.json::<Value>().await?, empty);

    h.shutdown().await;
    Ok(())
}

/// Which publishable key a session pins, end to end — the default, the
/// merchant's choice, an unregistered key, and a tenant with none.
///
/// The key is not decoration: it is `?key=` on the hosted `url`, on the
/// embedded iframe the SDK builds, and on the **return page** — the one URL a
/// *rail* holds a copy of. Pinning it on the row is what makes that URL
/// survive a key rotation, so this also asserts the stored column and the
/// helper both callers of the return URL go through.
#[tokio::test]
async fn a_session_pins_the_tenants_first_key_unless_the_merchant_names_another()
-> anyhow::Result<()> {
    let h = harness().await?;

    // The default: the *first configured* key. `PK_A2` sorts after `PK_A`
    // and is second in the registration, so an implementation that iterated
    // a `BTreeMap` would still pick `PK_A` here — which is why the "chose
    // the second" case below is the one that separates them.
    let session = hosted_session(&h).await?;
    assert!(
        session.url.contains(&format!("?key={PK_A}")),
        "{}",
        session.url
    );

    // The merchant names their second key.
    let intent_id = create_intent(&h).await?;
    let (status, body) = create_session(
        &h,
        CLIENT_A,
        &[
            ("payment_intent", intent_id.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
            ("publishable_key", PK_A2),
        ],
    )
    .await?;
    assert_eq!(status, 201, "{body:#}");
    let chosen_id = field(&body, "id")?;
    assert!(
        field(&body, "url")?.contains(&format!("?key={PK_A2}")),
        "the merchant's chosen key must be the one in the link: {body:#}"
    );

    // Stored on the row, and the return-page helper reads it from there —
    // the whole reason it is a column rather than a render-time lookup.
    let row = h
        .repositories
        .get_by_id_unscoped(&chosen_id)
        .await?
        .context("the session is stored")?;
    assert_eq!(row.publishable_key, PK_A2);
    assert_eq!(
        row.return_page_url(CHECKOUT_BASE),
        format!(
            "{CHECKOUT_BASE}/c/{chosen_id}/return?t={}&key={PK_A2}",
            row.return_token
        )
    );
    // …and that URL really does authenticate the return read, which is the
    // claim lane 2 hands to a rail.
    let built = row.return_page_url(CHECKOUT_BASE);
    let query = built.split_once('?').map(|(_, q)| q).context("a query")?;
    let mut token = None;
    let mut key = None;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("t", value)) => token = Some(value.to_owned()),
            Some(("key", value)) => key = Some(value.to_owned()),
            _ => {}
        }
    }
    let (token, key) = (token.context("t")?, key.context("key")?);
    let (status, body) = return_read(&h, &chosen_id, &key, &token).await?;
    assert_eq!(
        status, 200,
        "the URL vpay hands the rail must be one the return route accepts: {body:#}"
    );

    // A key that is not this merchant's: a `400` naming the parameter. The
    // merchant is authenticated and asking about their own registration, so
    // there is nothing to hide from them — unlike the payer-facing surface,
    // where an unknown key is the uniform 404.
    let intent_id = create_intent(&h).await?;
    let (status, body) = create_session(
        &h,
        CLIENT_A,
        &[
            ("payment_intent", intent_id.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
            ("publishable_key", PK_B),
        ],
    )
    .await?;
    assert_eq!(
        status, 400,
        "another merchant's key must be refused: {body:#}"
    );
    assert_eq!(
        body.pointer("/error/param").and_then(Value::as_str),
        Some("publishable_key"),
        "{body:#}"
    );
    // Nothing was written.
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM checkout_sessions WHERE payment_intent_id = $1")
            .bind(&intent_id)
            .fetch_one(&h.pool)
            .await?;
    assert_eq!(count, 0);

    h.shutdown().await;
    Ok(())
}

/// A tenant with **no** publishable key cannot have a checkout link built:
/// there would be no `?key=` to put in it, and the page it led to would
/// answer the uniform 404 to every payer.
///
/// Answered with the same `checkout_not_configured` code a missing
/// `checkout.public_base_url` gets and a **different sentence** — the fact is
/// the same from the merchant's side ("this vpay cannot do hosted checkout
/// for me"), and only the sentence tells whoever they forward it to which
/// line of YAML to add.
///
/// Merchant B is the tenant with a key and no origins elsewhere in this
/// suite; here the deployment is rebuilt with B holding no key at all, so
/// nothing else about the fixture has to move.
#[tokio::test]
async fn a_tenant_with_no_publishable_key_cannot_have_a_checkout_link_built() -> anyhow::Result<()>
{
    let h = harness_with_keyless_merchant_b().await?;

    // Merchant B creates its own intent, so the refusal is about the key and
    // not about tenancy.
    let amount = AMOUNT.to_string();
    let body: Value = browser()
        .post(h.url("/v1/payment_intents"))
        .bearer_auth(h.bearer(CLIENT_B))
        .header("idempotency-key", uuid::Uuid::new_v4().to_string())
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "amount={amount}&currency={CURRENCY}&payment_method_types[0]={PUSH_RAIL}"
        ))
        .send()
        .await?
        .json()
        .await?;
    let intent_id = field(&body, "id")?;

    let (status, body) = create_session(
        &h,
        CLIENT_B,
        &[
            ("payment_intent", intent_id.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    assert_eq!(status, 500, "{body:#}");
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("checkout_not_configured"),
        "{body:#}"
    );
    let message = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .context("a message")?;
    assert!(
        message.contains("publishable_keys"),
        "the message must name the line to add, not the deployment-wide block: {message}"
    );
    assert!(
        !message.contains("checkout.public_base_url"),
        "…and must not name a key that is configured on this deployment: {message}"
    );

    // Merchant A, on the same deployment, is unaffected — this is a property
    // of one registration, not of the deployment.
    let session = hosted_session(&h).await?;
    assert!(session.url.contains(&format!("?key={PK_A}")));

    h.shutdown().await;
    Ok(())
}

/// Claim 6: the **real** settlement transaction moves the session with the
/// intent, in one commit.
///
/// Driven through the shipping worker loop — `vpay_worker::run_once`, the
/// same function `vpay-worker-bin`'s loop calls — against the real WireMock
/// rail, so what is asserted is the write a deployment actually performs. A
/// staged `UPDATE` would prove nothing about whether anything calls it.
///
/// **Revert-proof.** Delete the `flip_session` call from
/// `vpay_db::settlement::apply_succeeded` and this fails on the last two
/// assertions: the intent reaches `succeeded` and the session stays
/// `open`/`unpaid`, which is exactly the disagreement `payment_status` was
/// denormalised to make impossible.
#[tokio::test]
async fn the_settlement_transaction_flips_the_session_with_the_intent() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;

    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("open".to_owned(), "unpaid".to_owned())
    );

    let (status, body) = session_read(&h, &session.id, PK_A, &session.secret).await?;
    assert_eq!(status, 200, "{body:#}");
    let intent_secret = body
        .pointer("/payment_intent/client_secret")
        .and_then(Value::as_str)
        .context("the session read renders the intent's credential")?
        .to_owned();

    assert_eq!(
        browser_confirm(&h, &session.intent_id, &intent_secret).await?,
        200
    );
    // Still open: a confirm is not a settlement.
    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("open".to_owned(), "unpaid".to_owned()),
        "a submitted charge must not move the session; only the settlement does"
    );

    let ran = drain_worker(&h).await?;
    assert!(ran > 0, "the confirm must have enqueued a poll job");

    let (intent_status,): (String,) =
        sqlx::query_as("SELECT status::TEXT FROM payment_intents WHERE id = $1")
            .bind(&session.intent_id)
            .fetch_one(&h.pool)
            .await?;
    assert_eq!(
        intent_status, "succeeded",
        "the stub's default status answer is SUCCESSFUL, so the poll must settle the charge"
    );
    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("complete".to_owned(), "paid".to_owned()),
        "the session must have moved in the same transaction as the intent"
    );

    // …and the payer's own page sees it, through the credential it already
    // holds.
    let (status, body) = session_read(&h, &session.id, PK_A, &session.secret).await?;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(field(&body, "status")?, "complete");
    assert_eq!(field(&body, "payment_status")?, "paid");

    h.shutdown().await;
    Ok(())
}

/// The other half of claim 6: a rail decline leaves the session `expired`
/// carrying `payment_status: failed` (D10 — there is no `failed` session
/// status).
///
/// The charge is pointed at the stub's decline reference before the poll, for
/// `browser_checkout.rs`'s reason: the confirm mints its own reference, and a
/// seam to fix it from a test would be a code path that exists only outside
/// production.
#[tokio::test]
async fn a_declined_payment_leaves_the_session_expired_and_failed() -> anyhow::Result<()> {
    /// The reference `requesttopay-status.json` answers `FAILED /
    /// NOT_ENOUGH_FUNDS` to.
    const DECLINED_REF: uuid::Uuid = uuid::Uuid::from_u128(0x0f01);

    let h = harness().await?;
    let session = hosted_session(&h).await?;

    let (_status, body) = session_read(&h, &session.id, PK_A, &session.secret).await?;
    let intent_secret = body
        .pointer("/payment_intent/client_secret")
        .and_then(Value::as_str)
        .context("the intent credential")?
        .to_owned();
    assert_eq!(
        browser_confirm(&h, &session.intent_id, &intent_secret).await?,
        200
    );

    sqlx::query("UPDATE charges SET provider_reference_id = $2 WHERE payment_intent_id = $1")
        .bind(&session.intent_id)
        .bind(DECLINED_REF)
        .execute(&h.pool)
        .await
        .context("pointing the charge at the rail's decline stub")?;

    drain_worker(&h).await?;

    let (intent_status,): (String,) =
        sqlx::query_as("SELECT status::TEXT FROM payment_intents WHERE id = $1")
            .bind(&session.intent_id)
            .fetch_one(&h.pool)
            .await?;
    assert_eq!(
        intent_status, "requires_payment_method",
        "a decline returns the intent, carrying last_payment_error"
    );
    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("expired".to_owned(), "failed".to_owned()),
        "D10: a session whose intent failed terminally is reported as expired with \
         payment_status: failed"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 7: `expire` is a compare-and-swap, a second one is a `409`, and a
/// session whose intent has a live charge cannot be expired at all.
///
/// The live-charge guard is the one that matters: expiring there would tell a
/// merchant the checkout was abandoned while the rail may still take the
/// payment, and the settlement transaction would then contradict it by
/// flipping the same row to `complete`/`paid`.
///
/// **Revert-proof.** Delete the `NOT EXISTS` clause from
/// `vpay_db::checkout_sessions`' `expire` and the live-charge case answers
/// `200` instead of `409`.
#[tokio::test]
async fn expiring_a_session_is_a_compare_and_swap_and_a_live_charge_refuses_it()
-> anyhow::Result<()> {
    let h = harness().await?;

    let expire = |id: String| {
        let url = h.url(&format!("/v1/checkout/sessions/{id}/expire"));
        let token = h.bearer(CLIENT_A);
        async move {
            let response = browser()
                .post(url)
                .bearer_auth(token)
                .header("idempotency-key", uuid::Uuid::new_v4().to_string())
                .header("content-type", "application/x-www-form-urlencoded")
                .body("")
                .send()
                .await?;
            let status = response.status().as_u16();
            let body: Value = response.json().await?;
            Ok::<_, anyhow::Error>((status, body))
        }
    };

    // The ordinary case.
    let session = hosted_session(&h).await?;
    let (status, body) = expire(session.id.clone()).await?;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(field(&body, "status")?, "expired");
    assert_eq!(
        field(&body, "payment_status")?,
        "unpaid",
        "expiring must not rewrite what the money did"
    );
    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("expired".to_owned(), "unpaid".to_owned())
    );

    // A second one is the 409, not a silent 200.
    let (status, body) = expire(session.id.clone()).await?;
    assert_eq!(status, 409, "{body:#}");
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("invalid_state"),
        "{body:#}"
    );

    // Another merchant's session is the 404, not the 409 — the tenancy
    // property, on the write path.
    let session_b = hosted_session(&h).await?;
    let response = browser()
        .post(h.url(&format!("/v1/checkout/sessions/{}/expire", session_b.id)))
        .bearer_auth(h.bearer(CLIENT_B))
        .header("idempotency-key", uuid::Uuid::new_v4().to_string())
        .header("content-type", "application/x-www-form-urlencoded")
        .body("")
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 404);
    assert_eq!(
        stored_session(&h.pool, &session_b.id).await?.0,
        "open",
        "a foreign expire must not have moved the row"
    );

    // A live charge refuses it.
    let (_status, body) = session_read(&h, &session_b.id, PK_A, &session_b.secret).await?;
    let intent_secret = body
        .pointer("/payment_intent/client_secret")
        .and_then(Value::as_str)
        .context("the intent credential")?
        .to_owned();
    assert_eq!(
        browser_confirm(&h, &session_b.intent_id, &intent_secret).await?,
        200
    );

    let (status, body) = expire(session_b.id.clone()).await?;
    assert_eq!(
        status, 409,
        "a session with a live charge must not expire: {body:#}"
    );
    assert!(
        body.pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains(&session_b.intent_id)),
        "the refusal must tell the merchant which intent to poll: {body:#}"
    );
    assert_eq!(
        stored_session(&h.pool, &session_b.id).await?.0,
        "open",
        "the guard must have refused the write, not merely the response"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 8: one open session per intent, and the index — not the pre-check —
/// is what enforces it.
///
/// The second half is the part a `SELECT`-then-`INSERT` would get wrong under
/// concurrency, so it is asserted directly against the constraint: the same
/// insert is attempted twice through the repository, and the second is a
/// `UniqueViolation` naming `checkout_sessions_one_open_per_intent`.
///
/// **Revert-proof.** Drop the partial unique index from migration `0028` and
/// the second half fails — the write succeeds and the intent has two live
/// payer links.
#[tokio::test]
async fn an_intent_may_have_only_one_open_session() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;

    // Through the API: the pre-check, which names the session in the way.
    let (status, body) = create_session(
        &h,
        CLIENT_A,
        &[
            ("payment_intent", session.intent_id.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    assert_eq!(status, 409, "{body:#}");
    assert!(
        body.pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains(&session.id)),
        "the refusal must name the session that is in the way: {body:#}"
    );

    // Through the repository, past the pre-check entirely: the index is what
    // actually refuses, which is what makes the rule hold under concurrency.
    let new = vpay_db::NewCheckoutSession {
        id: vpay_core::ids::checkout_session_id(),
        merchant_id: MERCHANT_A.to_owned(),
        payment_intent_id: session.intent_id.clone(),
        livemode: false,
        ui_mode: "hosted".to_owned(),
        success_url: Some(CANCEL_URL.to_owned()),
        cancel_url: Some(CANCEL_URL.to_owned()),
        return_url: None,
        publishable_key: PK_A.to_owned(),
        client_secret_suffix: vpay_core::ids::client_secret_suffix(),
        return_token: vpay_core::ids::return_token(),
        expires_at: time::OffsetDateTime::now_utc() + time::Duration::hours(24),
        created_at: time::OffsetDateTime::now_utc(),
    };
    let error = h
        .repositories
        .create(&new)
        .await
        .expect_err("a second open session on one intent must be refused by the index");
    assert!(
        matches!(
            &error,
            vpay_db::DbError::UniqueViolation { constraint, .. }
                if constraint == "checkout_sessions_one_open_per_intent"
        ),
        "the index, not some other constraint: {error:?}"
    );

    // And once the first is expired, a second is allowed — the index is
    // partial on purpose.
    browser()
        .post(h.url(&format!("/v1/checkout/sessions/{}/expire", session.id)))
        .bearer_auth(h.bearer(CLIENT_A))
        .header("idempotency-key", uuid::Uuid::new_v4().to_string())
        .header("content-type", "application/x-www-form-urlencoded")
        .body("")
        .send()
        .await?;
    let (status, body) = create_session(
        &h,
        CLIENT_A,
        &[
            ("payment_intent", session.intent_id.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    assert_eq!(
        status, 201,
        "an expired session blocks nothing — the index is partial: {body:#}"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 9: a deployment that serves no checkout page refuses to create a
/// session rather than minting a `url` that resolves to nothing.
///
/// This is AGENTS.md's second rule applied to a configuration gap: the
/// plausible-looking alternative — answering `201` with `url: null` — would
/// leave a merchant holding a session no payer can ever reach, with nothing
/// in the response saying so.
///
/// The `code` is what an SDK branches on, so it is asserted rather than the
/// status: the status is `500` and not the `503` the Step 9 plan asked for,
/// deliberately and for the reason `ApiError::CheckoutNotConfigured`'s own
/// doc gives — ADR-0011 derives the status from the category, and the only
/// category that answers `503` would also tell the merchant to retry a
/// request that cannot succeed until someone deploys.
#[tokio::test]
async fn a_deployment_without_a_checkout_app_refuses_to_create_a_session() -> anyhow::Result<()> {
    let h = harness_with_checkout(None).await?;
    let intent_id = create_intent(&h).await?;

    let (status, body) = create_session(
        &h,
        CLIENT_A,
        &[
            ("payment_intent", intent_id.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;

    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("checkout_not_configured"),
        "{body:#}"
    );
    assert_eq!(status, 500, "{body:#}");
    assert!(
        body.pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("checkout.public_base_url")),
        "the message must name the key an operator has to set: {body:#}"
    );

    // Nothing was written: a refused create must leave no session behind.
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM checkout_sessions")
        .fetch_one(&h.pool)
        .await?;
    assert_eq!(count, 0);

    // The deployment still has a browser surface for payment intents — this
    // is a missing *capability*, not a broken deployment.
    let response = browser()
        .get(h.url("/v1/browser/checkout/origins"))
        .query(&[("key", PK_A)])
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        response.json::<Value>().await?,
        serde_json::json!({ "origins": [] }),
        "no checkout app means no embedding, and the honest answer is an empty list"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 11: both browser reads tell the payer **who they are paying**, from
/// configuration — and fall back to the tenant id for a merchant that
/// configured no name.
///
/// The page requires the member: `isSessionEnvelope`
/// (`frontends/apps/checkout/src/lib/api.ts`) refuses a session envelope
/// without `merchant.name` (until lane 3b made it optional), so before this landed vpay's own checkout page
/// rendered `error.unexpected` for every session it read. The field name is
/// asserted as a literal here because it is a wire contract shared with a
/// TypeScript app that cannot be type-checked against this crate.
///
/// Merchant A configures `display_name`; merchant B does not. Both are read
/// through the shipping routes in the same deployment, so the fallback is
/// exercised as a *response* rather than as a map lookup.
#[tokio::test]
async fn both_browser_reads_carry_the_merchants_display_name() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    let token = return_token(&h.pool, &session.id).await?;

    let (status, body) = session_read(&h, &session.id, PK_A, &session.secret).await?;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(
        body.pointer("/merchant/name").and_then(Value::as_str),
        Some(DISPLAY_NAME_A),
        "the session read must tell the page who the payer is paying: {body:#}"
    );

    let (status, body) = return_read(&h, &session.id, PK_A, &token).await?;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(
        body.pointer("/merchant/name").and_then(Value::as_str),
        Some(DISPLAY_NAME_A),
        "the return page shows it too — `{{merchant}} has been told you paid`: {body:#}"
    );

    // Merchant B configures none. The fallback is the tenant id: an internal
    // label, but a true one, and the alternative is a page that cannot paint.
    let intent_b = create_intent_for(&h, CLIENT_B).await?;
    let (status, created) = create_session(
        &h,
        CLIENT_B,
        &[
            ("payment_intent", intent_b.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    anyhow::ensure!(status == 201, "creating merchant B's session: {created:#}");

    let (status, body) = session_read(
        &h,
        &field(&created, "id")?,
        PK_B,
        &field(&created, "client_secret")?,
    )
    .await?;
    assert_eq!(status, 200, "{body:#}");
    assert!(
        body.get("merchant").is_none(),
        "a merchant with no display_name renders no `merchant` member — never its tenant id: {body:#}"
    );
    assert!(
        !body.to_string().contains(MERCHANT_B),
        "the tenant id must not appear anywhere in what a payer is shown: {body:#}"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 12: both browser reads stop answering at `expires_at`, whatever the
/// session's `status` — and the **read** is what enforces it, not the sweep.
///
/// The `return_token` is written down by design: it travels in a query
/// string, so it is in the rail's own storage, in whatever the rail logs, and
/// in the checkout app's access logs. `expires_at` (D10's 24 hours) is the
/// bound on how long that copy is worth anything, and before this it bounded
/// nothing — a token read the session forever.
///
/// The sweep cannot be what enforces it, and this test is written so that it
/// is not: no worker runs here at all. The sweep leaves a session with a live
/// charge `open` on purpose, it runs at most once an hour, and a deployment
/// whose worker is down would keep answering these reads for the length of
/// the outage.
///
/// The refusal is the *same* uniform 404 as every other one on this surface,
/// asserted byte for byte against an unknown id rather than by status alone —
/// a distinguishable "expired" answer would tell a holder of a stale token
/// that the session existed.
///
/// **Revert-proof.** Delete the `expires_at` check from `authenticate` and
/// both halves fail with `200`.
#[tokio::test]
async fn both_browser_reads_stop_at_the_horizon_whatever_the_status() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    let token = return_token(&h.pool, &session.id).await?;

    // The refusal to compare against: the *same* id with the wrong
    // credential, taken while the session is still live. The 404 echoes the
    // id the caller spelled (`ApiError::NotFound`), so a body from a
    // different id would differ for a reason that has nothing to do with
    // this test — and comparing "expired" against "wrong credential" is the
    // sharper claim anyway: a holder of a stale token must not be able to
    // tell that the session ever existed.
    let (_status, refused_body) =
        session_read(&h, &session.id, PK_A, "cs_wrong_0000000000000000000").await?;
    let (_status, refused_return_body) = return_read(&h, &session.id, PK_A, "wrongtoken").await?;

    // Both reads work right up to the horizon.
    let (status, _body) = session_read(&h, &session.id, PK_A, &session.secret).await?;
    assert_eq!(status, 200);
    let (status, _body) = return_read(&h, &session.id, PK_A, &token).await?;
    assert_eq!(status, 200);

    // 24 hours later. Only `expires_at` moves; `status` stays `open`, which
    // is what a deployment whose sweep has not run yet actually looks like.
    sqlx::query(
        "UPDATE checkout_sessions SET expires_at = now() - INTERVAL '1 second' WHERE id = $1",
    )
    .bind(&session.id)
    .execute(&h.pool)
    .await
    .context("moving the session past its horizon")?;
    assert_eq!(
        stored_session(&h.pool, &session.id).await?.0,
        "open",
        "the row must still say `open`, or this proves the sweep rather than the read"
    );

    for (label, (status, body), refusal) in [
        (
            "session read",
            session_read(&h, &session.id, PK_A, &session.secret).await?,
            &refused_body,
        ),
        (
            "return read",
            return_read(&h, &session.id, PK_A, &token).await?,
            &refused_return_body,
        ),
    ] {
        assert_eq!(status, 404, "{label}: {body:#}");
        assert_eq!(
            &body, refusal,
            "{label}: a correct credential for an expired session must answer exactly what a \
             wrong one does"
        );
    }

    h.shutdown().await;
    Ok(())
}

/// Claim 13: the session read hands over the **intent's** `client_secret`
/// only while the session is `open`.
///
/// That credential exists so vpay's page can drive
/// `POST /v1/browser/payment_intents/{id}/confirm`. Once the session is
/// `complete` there is nothing left to confirm, and re-issuing it on every
/// subsequent read would keep a live intent credential in circulation for a
/// checkout that is over — reachable by anyone who has the session secret,
/// which is in the URL the payer was sent.
///
/// The page loses nothing: it read the secret on its first call and polls
/// with the copy it holds.
///
/// The session is driven to `complete` by the **shipping settlement
/// transaction** — the real worker loop over a real rail — rather than by an
/// `UPDATE`, so what is being read is a state a deployment actually produces.
///
/// **Revert-proof.** Make the read render `ExpandedWithSecret`
/// unconditionally and the last assertion fails.
#[tokio::test]
async fn the_session_read_stops_handing_out_the_intents_secret_once_it_is_settled()
-> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;

    let (_status, body) = session_read(&h, &session.id, PK_A, &session.secret).await?;
    let intent_secret = body
        .pointer("/payment_intent/client_secret")
        .and_then(Value::as_str)
        .context("an open session hands the page the intent's credential")?
        .to_owned();

    assert_eq!(
        browser_confirm(&h, &session.intent_id, &intent_secret).await?,
        200
    );
    drain_worker(&h).await?;
    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("complete".to_owned(), "paid".to_owned()),
        "the settlement transaction must have finished the session"
    );

    let (status, body) = session_read(&h, &session.id, PK_A, &session.secret).await?;
    assert_eq!(
        status, 200,
        "a finished session is still readable — the page has an outcome to paint: {body:#}"
    );
    assert_eq!(
        body.pointer("/payment_intent/status")
            .and_then(Value::as_str),
        Some("succeeded"),
        "and the outcome is what it is for: {body:#}"
    );
    assert_eq!(
        body.pointer("/payment_intent/client_secret"),
        None,
        "a settled session must not re-issue the intent's credential: {body:#}"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 10: the worker's hourly housekeeping sweep expires a session past
/// its horizon — and leaves alone one whose intent has a live charge.
///
/// `expires_at` was written at create and read by **nothing** until Step 9's
/// lane 1b: a session past D10's 24 hours reported `status: open` until a
/// merchant expired it by hand or the intent settled, so `status` could not
/// tell "still payable" from "abandoned yesterday". This runs the shipping
/// `vpay_worker::run_once` over the shipping `sweep_expired` job — seeded by
/// the shipping `seed_singletons` — rather than calling the repository
/// method, because what is being claimed is that a *deployment* expires
/// sessions, not that a statement does.
///
/// The horizon is moved by rewriting `expires_at`, which is the one thing a
/// test can do that a deployment cannot wait 24 hours for. Nothing else is
/// staged: both sessions were created through `POST /v1/checkout/sessions`
/// and the live charge was opened through the browser confirm the page uses.
///
/// The poll job of that charge is pushed an hour out first. Without it the
/// drain would settle the charge before the sweep ran, the session would be
/// `complete`, and the live-charge guard would have had nothing to guard —
/// the case would pass while proving nothing.
///
/// **Revert-proof.** Delete the `NOT EXISTS` live-charge clause from
/// `expire_due` and the second half fails: the paying session expires while
/// a rail is still holding the payment, and the settlement transaction then
/// contradicts it.
#[tokio::test]
async fn the_housekeeping_sweep_expires_a_stale_session_and_spares_a_paying_one()
-> anyhow::Result<()> {
    let h = harness().await?;

    let abandoned = hosted_session(&h).await?;
    let paying = hosted_session(&h).await?;

    // A payer on the second session confirms: a real charge, in `submitting`,
    // against a rail that may still take the money.
    let (_status, body) = session_read(&h, &paying.id, PK_A, &paying.secret).await?;
    let intent_secret = body
        .pointer("/payment_intent/client_secret")
        .and_then(Value::as_str)
        .context("the intent credential the page confirms with")?
        .to_owned();
    assert_eq!(
        browser_confirm(&h, &paying.intent_id, &intent_secret).await?,
        200
    );

    // See this test's own comment: the sweep, not the poll ladder, is what is
    // under test, and the poll would settle the charge out from under it.
    sqlx::query("UPDATE jobs SET run_at = now() + INTERVAL '1 hour' WHERE kind = 'poll_charge'")
        .execute(&h.pool)
        .await
        .context("deferring the poll jobs so only the sweep can be claimed")?;

    // Both horizons in the past. `expires_at` is the only thing rewritten;
    // `status` is what the sweep has to decide.
    let moved =
        sqlx::query("UPDATE checkout_sessions SET expires_at = now() - INTERVAL '1 minute'")
            .execute(&h.pool)
            .await
            .context("moving both sessions past their horizon")?
            .rows_affected();
    assert_eq!(moved, 2, "both sessions must be past their horizon");

    vpay_worker::seed_singletons(h.repositories.as_ref())
        .await
        .context("seeding the singleton jobs a worker seeds at boot")?;

    let endpoints = support::no_webhook_endpoints();
    let egress = support::default_egress_policy();
    let mut swept = false;
    for _ in 0..8 {
        let Some(settled) = vpay_worker::run_once(
            h.repositories.as_ref(),
            &h.adapters,
            &h.rails,
            &RecoveryPolicy::default(),
            &vpay_worker::WebhookContext {
                endpoints: &endpoints,
                egress,
            },
            "checkout-sessions-sweep",
        )
        .await?
        else {
            break;
        };
        if settled.kind == "sweep_expired" {
            assert_eq!(
                settled.error, None,
                "the housekeeping sweep must not fail: {:?}",
                settled.error
            );
            swept = true;
            break;
        }
    }
    assert!(swept, "the housekeeping sweep never ran");

    assert_eq!(
        stored_session(&h.pool, &abandoned.id).await?,
        ("expired".to_owned(), "unpaid".to_owned()),
        "a session past its horizon with nothing driving it must stop saying `open` — and the \
         sweep must not rewrite what the money did"
    );
    assert_eq!(
        stored_session(&h.pool, &paying.id).await?,
        ("open".to_owned(), "unpaid".to_owned()),
        "a session whose intent has a live charge must survive the sweep: the rail may still \
         take the payment, and the settlement transaction would contradict an expiry"
    );

    // The premise of the second assertion, stated rather than assumed.
    let (state,): (String,) =
        sqlx::query_as("SELECT state::TEXT FROM charges WHERE payment_intent_id = $1")
            .bind(&paying.intent_id)
            .fetch_one(&h.pool)
            .await
            .context("the charge the guard is guarding")?;
    // The four labels of `vpay_db`'s `LIVE_CHARGE_STATES`, which is the set
    // the guard reads and the set `charges_live_idx` is built over. The
    // accepted confirm above lands on `submitted`; the point is that it is
    // *live*, not which of the four it is.
    assert!(
        ["submitting", "submitted", "pending", "unresolved"].contains(&state.as_str()),
        "the charge must still be live, or this case proves nothing: {state}"
    );

    h.shutdown().await;
    Ok(())
}

/// The session's two credentials never reach an operator's log, asserted
/// against a row that came **back out of Postgres**.
///
/// A unit test in `vpay-db` pins the `Debug` impl in isolation; this one
/// catches a column added to `COLUMNS` without being added to the redaction.
#[tokio::test]
async fn a_stored_sessions_debug_output_carries_neither_credential() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    let token = return_token(&h.pool, &session.id).await?;

    let row = h
        .repositories
        .get_by_id_unscoped(&session.id)
        .await?
        .context("the created session is stored")?;

    let suffix = session
        .secret
        .split_once("_secret_")
        .map(|(_, suffix)| suffix.to_owned())
        .context("the credential carries the separator")?;
    assert_eq!(row.client_secret_suffix, suffix);
    assert_eq!(row.return_token, token);

    let formatted = format!("{row:?}");
    assert!(!formatted.contains(&suffix), "the stored suffix leaked");
    assert!(
        !formatted.contains(&session.secret),
        "the joined credential leaked"
    );
    assert!(!formatted.contains(&token), "the return token leaked");
    assert_eq!(
        formatted.matches("[32 chars redacted]").count(),
        2,
        "both credentials must be redacted: {formatted}"
    );
    // And the row is still useful to whoever is reading the log.
    assert!(formatted.contains(&session.id), "{formatted}");
    assert!(formatted.contains(&session.intent_id), "{formatted}");
    assert!(formatted.contains(MERCHANT_A), "{formatted}");

    h.shutdown().await;
    Ok(())
}

// ------------------------------------------- `checkout.session.expired` ---
//
// Claim 14: the sweep does not only move a row any more — it tells the
// merchant. Until migration `0029` an expired session notified nobody: no
// event, no webhook, and `docs/flows/hosted-checkout.md` said so under "What
// is not built". These cases are what retires that sentence, and every one of
// them drives the shipping `vpay_worker::run_once` over the shipping
// `seed_singletons` job rather than calling a repository method, because the
// claim is that a *deployment* emits the event.

/// The type migration `0029` opened the `events` vocabulary for.
const SESSION_EXPIRED: &str = "checkout.session.expired";

/// Two endpoints for merchant A, so "one delivery per configured endpoint" is
/// a count and not a tautology, and one for merchant B, so a session expiring
/// for A cannot fan out to B.
///
/// The URLs are never reached: every case below stops the loop the moment the
/// fan-out has run, before any `deliver_webhook` job can be claimed. What is
/// asserted is what the fan-out *created* — the delivery rows and their jobs —
/// which is where the event either reaches a merchant's queue or does not.
fn two_endpoints_for_a() -> vpay_worker::EndpointRegistry {
    vpay_worker::EndpointRegistry::from_pairs([
        (
            MERCHANT_A.to_owned(),
            vec![
                vpay_worker::Endpoint {
                    id: "acme-primary".to_owned(),
                    url: "https://hooks.acme.example/vpay".to_owned(),
                    secrets: vec!["whsec_acme_primary_0000000000000000".to_owned()],
                },
                vpay_worker::Endpoint {
                    id: "acme-audit".to_owned(),
                    url: "https://audit.acme.example/vpay".to_owned(),
                    secrets: vec!["whsec_acme_audit_00000000000000000".to_owned()],
                },
            ],
        ),
        (
            MERCHANT_B.to_owned(),
            vec![vpay_worker::Endpoint {
                id: "beta-only".to_owned(),
                url: "https://hooks.beta.example/vpay".to_owned(),
                secrets: vec!["whsec_beta_only_000000000000000000".to_owned()],
            }],
        ),
    ])
}

/// Drives the shipping loop until a job of `kind` has finished, and answers
/// how many jobs ran on the way.
///
/// It does **not** call `support::make_every_job_runnable`: two of the cases
/// below defer a `poll_charge` job on purpose so that the sweep, and not the
/// poll ladder, is what decides the session's fate, and a helper that pulled
/// every job forward would settle the charge out from under them.
async fn run_until(
    h: &Harness,
    endpoints: &vpay_worker::EndpointRegistry,
    kind: &str,
) -> anyhow::Result<usize> {
    let egress = support::default_egress_policy();
    let mut ran = 0_usize;
    for _ in 0..16 {
        let Some(settled) = vpay_worker::run_once(
            h.repositories.as_ref(),
            &h.adapters,
            &h.rails,
            &RecoveryPolicy::default(),
            &vpay_worker::WebhookContext { endpoints, egress },
            "checkout-sessions-expiry",
        )
        .await?
        else {
            anyhow::bail!("the loop ran out of work before `{kind}` ran");
        };
        ran += 1;
        if settled.kind == kind {
            anyhow::ensure!(
                settled.error.is_none(),
                "`{kind}` must not fail: {:?}",
                settled.error
            );
            return Ok(ran);
        }
    }
    anyhow::bail!("`{kind}` never ran")
}

/// One housekeeping sweep, then one outbox drain — and **nothing after it**,
/// so what the assertions read is what the fan-out created rather than what a
/// delivery attempt to an unreachable host left behind.
///
/// Two phases rather than one loop with two flags, because the order matters:
/// a drain that ran before the sweep would have had no event to fan out, and a
/// helper that accepted it would let every case below pass with zero
/// deliveries.
async fn sweep_then_fan_out(
    h: &Harness,
    endpoints: &vpay_worker::EndpointRegistry,
) -> anyhow::Result<()> {
    vpay_worker::seed_singletons(h.repositories.as_ref())
        .await
        .context("seeding the singleton jobs a worker seeds at boot")?;
    run_until(h, endpoints, "sweep_expired").await?;
    // The sweep's own row is left leased by `run_once`'s reschedule, exactly
    // as a running worker leaves it; the drain is a different singleton and is
    // claimable in its own right.
    run_until(h, endpoints, "fan_out_events").await?;
    Ok(())
}

/// Moves every session in this deployment past its horizon.
///
/// `expires_at` is the only column rewritten — `status` is what the sweep has
/// to decide, and staging that would prove nothing.
async fn move_every_session_past_its_horizon(h: &Harness) -> anyhow::Result<u64> {
    Ok(
        sqlx::query("UPDATE checkout_sessions SET expires_at = now() - INTERVAL '1 minute'")
            .execute(&h.pool)
            .await
            .context("moving the sessions past their horizon")?
            .rows_affected(),
    )
}

/// Every event row of `event_type`, oldest first: id, the object it names, and
/// its `data` **as stored**.
async fn events_of_type(
    pool: &PgPool,
    event_type: &str,
) -> anyhow::Result<Vec<(String, String, String, Value)>> {
    let rows: Vec<(String, String, String, Value)> = sqlx::query_as(
        "SELECT id, merchant_id, object_id, data FROM events WHERE type = $1 ORDER BY seq",
    )
    .bind(event_type)
    .fetch_all(pool)
    .await
    .context("reading the events of a type")?;
    Ok(rows)
}

/// The `webhook_deliveries` rows for one event, by endpoint id, sorted.
async fn delivery_endpoints(pool: &PgPool, event_id: &str) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT endpoint_id FROM webhook_deliveries WHERE event_id = $1 ORDER BY endpoint_id",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .context("reading the deliveries of an event")?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// The `dedupe_key` of every `deliver_webhook` job, sorted.
async fn delivery_job_keys(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT dedupe_key FROM jobs WHERE kind = 'deliver_webhook' ORDER BY dedupe_key",
    )
    .fetch_all(pool)
    .await
    .context("reading the delivery jobs")?;
    Ok(rows.into_iter().map(|(key,)| key).collect())
}

/// `GET /v1/events` for a tenant, through the shipping router.
async fn list_events(h: &Harness, client_id: &str) -> anyhow::Result<(u16, Value)> {
    let response = browser()
        .get(h.url("/v1/events"))
        .bearer_auth(h.bearer(client_id))
        .send()
        .await
        .context("listing events")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    Ok((status, body))
}

/// `GET /v1/events/{id}` for a tenant, through the shipping router.
async fn retrieve_event(h: &Harness, client_id: &str, id: &str) -> anyhow::Result<(u16, Value)> {
    let response = browser()
        .get(h.url(&format!("/v1/events/{id}")))
        .bearer_auth(h.bearer(client_id))
        .send()
        .await
        .context("retrieving an event")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    Ok((status, body))
}

/// Claim 14a: one sweep over an abandoned session writes exactly one
/// `checkout.session.expired`, whose `data.object` is that session and whose
/// body carries no credential — and the drain turns it into one delivery, and
/// one job, per configured endpoint.
///
/// The negative assertions are made on the **serialised JSON string** of the
/// stored `data`, not on the parsed object: a credential under a key this test
/// did not think to look at would still be in the bytes a merchant's endpoint
/// receives, and the bytes are the thing.
///
/// **Revert-proof, measured 2026-09-04:** delete the `events::insert_in_tx`
/// call from `vpay_db::CheckoutSessions::expire_due` and this fails on
/// "exactly one event" — the session still expires, which is precisely the
/// silent regression the case exists to catch.
#[tokio::test]
async fn an_expiry_sweep_emits_one_event_and_one_delivery_per_endpoint() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    assert_eq!(move_every_session_past_its_horizon(&h).await?, 1);

    let endpoints = two_endpoints_for_a();
    sweep_then_fan_out(&h, &endpoints).await?;

    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("expired".to_owned(), "unpaid".to_owned()),
        "the sweep must still do what it did before it emitted anything"
    );

    let events = events_of_type(&h.pool, SESSION_EXPIRED).await?;
    let [(event_id, merchant_id, object_id, data)] = events.as_slice() else {
        anyhow::bail!("exactly one checkout.session.expired was expected: {events:?}");
    };
    assert_eq!(object_id, &session.id, "the event must name the session");
    assert_eq!(merchant_id, MERCHANT_A);
    assert!(
        event_id.starts_with("evt_"),
        "an event id merchants dedupe on: {event_id}"
    );
    assert_eq!(
        data.get("id").and_then(Value::as_str),
        Some(session.id.as_str()),
        "data.object is the session: {data:#}"
    );
    assert_eq!(
        data.get("object").and_then(Value::as_str),
        Some("checkout.session")
    );
    assert_eq!(
        data.get("status").and_then(Value::as_str),
        Some("expired"),
        "the snapshot must describe the transition, not the row before it: {data:#}"
    );
    assert_eq!(
        data.get("payment_status").and_then(Value::as_str),
        Some("unpaid"),
        "an expiry never rewrites what the money did: {data:#}"
    );
    assert_eq!(
        data.get("url"),
        Some(&Value::Null),
        "the url carries the credential in its fragment and must not be delivered (the rendered \
         value is never printed; the session is {})",
        session.id
    );
    assert_eq!(
        data.get("ui_mode").and_then(Value::as_str),
        Some("hosted"),
        "…and a null url must still be distinguishable from an embedded session"
    );

    // The decisive assertion, on the bytes.
    let token = return_token(&h.pool, &session.id).await?;
    let suffix = session
        .secret
        .split_once("_secret_")
        .map(|(_, suffix)| suffix.to_owned())
        .context("the credential carries the separator")?;
    let body = serde_json::to_string(data).context("the stored data serialises")?;
    for secret in [session.secret.as_str(), suffix.as_str(), token.as_str()] {
        assert!(
            !body.contains(secret),
            "a payer credential is in the event body (the body is never printed; it is {} bytes \
             for session {})",
            body.len(),
            session.id
        );
    }
    assert!(
        !body.contains("_secret_") && !body.contains("client_secret"),
        "nothing shaped like a credential may be in the event body (the body is never printed; \
         it is {} bytes for session {})",
        body.len(),
        session.id
    );

    // One delivery, and one job, per configured endpoint of *this* merchant.
    assert_eq!(
        delivery_endpoints(&h.pool, event_id).await?,
        vec!["acme-audit".to_owned(), "acme-primary".to_owned()],
        "one delivery per configured endpoint, and none of merchant B's"
    );
    let jobs = delivery_job_keys(&h.pool).await?;
    assert_eq!(
        jobs.len(),
        2,
        "one deliver_webhook job per delivery row: {jobs:?}"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 14b: a second sweep over the same session writes no second event.
///
/// Idempotency here is the compare-and-swap and nothing else: the second pass
/// reads no due session at all, because the first one left `status = 'expired'`
/// in the same commit as the event. A merchant deduping by `event.id` would
/// survive a duplicate; a merchant reading `GET /v1/events` would not, and
/// neither would their alerting.
#[tokio::test]
async fn a_second_sweep_writes_no_second_expiry_event() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    move_every_session_past_its_horizon(&h).await?;

    let endpoints = two_endpoints_for_a();
    sweep_then_fan_out(&h, &endpoints).await?;
    let after_one = events_of_type(&h.pool, SESSION_EXPIRED).await?;
    assert_eq!(
        after_one.len(),
        1,
        "the first sweep emits one: {after_one:?}"
    );

    // A second pass over the same deployment. The sweep is a singleton whose
    // row `run_once` rescheduled, so it is pulled forward the way the loop's
    // own timer would — nothing else about the state is touched.
    sqlx::query(
        "UPDATE jobs SET run_at = now(), locked_at = NULL, locked_by = NULL \
         WHERE dedupe_key = 'sweep:expired'",
    )
    .execute(&h.pool)
    .await
    .context("re-arming the housekeeping singleton")?;
    run_until(&h, &endpoints, "sweep_expired").await?;

    let after_two = events_of_type(&h.pool, SESSION_EXPIRED).await?;
    assert_eq!(
        after_two.len(),
        1,
        "a second sweep must write no second event: {after_two:?}"
    );
    assert_eq!(
        after_one.first().map(|(id, ..)| id),
        after_two.first().map(|(id, ..)| id),
        "…and must not replace the first one either"
    );
    assert_eq!(
        stored_session(&h.pool, &session.id).await?.0,
        "expired".to_owned()
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 14c: a session whose intent has a live charge is neither expired nor
/// evented.
///
/// The event follows the flip, so proving the flip is refused is only half of
/// it: a read that rendered the session before the write refused it would mint
/// an `evt_…` and build an object claiming an abandoned checkout, and the
/// second assertion here is what says that never happens.
///
/// The poll job is deferred an hour first. Without it the drain would settle
/// the charge before the sweep ran, the session would be `complete`, and the
/// live-charge guard would have had nothing to guard.
#[tokio::test]
async fn a_session_with_a_live_charge_is_neither_expired_nor_evented() -> anyhow::Result<()> {
    let h = harness().await?;
    let paying = hosted_session(&h).await?;

    let (_status, body) = session_read(&h, &paying.id, PK_A, &paying.secret).await?;
    let intent_secret = body
        .pointer("/payment_intent/client_secret")
        .and_then(Value::as_str)
        .context("the intent credential the page confirms with")?
        .to_owned();
    assert_eq!(
        browser_confirm(&h, &paying.intent_id, &intent_secret).await?,
        200
    );
    sqlx::query("UPDATE jobs SET run_at = now() + INTERVAL '1 hour' WHERE kind = 'poll_charge'")
        .execute(&h.pool)
        .await
        .context("deferring the poll so only the sweep can be claimed")?;
    move_every_session_past_its_horizon(&h).await?;

    let endpoints = two_endpoints_for_a();
    sweep_then_fan_out(&h, &endpoints).await?;

    assert_eq!(
        stored_session(&h.pool, &paying.id).await?,
        ("open".to_owned(), "unpaid".to_owned()),
        "a session a rail is still holding must survive the sweep"
    );
    assert!(
        events_of_type(&h.pool, SESSION_EXPIRED).await?.is_empty(),
        "and must not be reported as abandoned to its merchant either"
    );
    // The guard's *read* copy, which the two assertions above cannot see: the
    // write refuses this session on its own, so both would still hold if
    // `due_for_expiry` returned it. It must not — rendering a session a rail
    // is holding mints an `evt_…` and builds an object claiming the checkout
    // was abandoned, which is one more place a future change could leak.
    assert!(
        vpay_db::CheckoutSessions::due_for_expiry(
            h.repositories.as_ref(),
            time::OffsetDateTime::now_utc(),
            100,
        )
        .await?
        .is_empty(),
        "a session a rail is holding must not even be read as due"
    );

    // The premise of both assertions, stated rather than assumed.
    let (state,): (String,) =
        sqlx::query_as("SELECT state::TEXT FROM charges WHERE payment_intent_id = $1")
            .bind(&paying.intent_id)
            .fetch_one(&h.pool)
            .await
            .context("the charge the guard is guarding")?;
    assert!(
        ["submitting", "submitted", "pending", "unresolved"].contains(&state.as_str()),
        "the charge must still be live, or this case proves nothing: {state}"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 14d: a session the **settlement** transaction finished produces
/// `payment_intent.succeeded` and **no** `checkout.session.expired`.
///
/// D10 gives a settled-then-declined session the label `expired` too, so
/// "expired" on its own is not the trigger — the sweep is, and only for a
/// session whose horizon passed with nothing driving it. A settlement already
/// emits a `payment_intent.*` event for the same thing happening, and a second
/// event would be a duplicate vpay invented.
#[tokio::test]
async fn a_session_finished_by_settlement_emits_no_expiry_event() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;

    let (_status, body) = session_read(&h, &session.id, PK_A, &session.secret).await?;
    let intent_secret = body
        .pointer("/payment_intent/client_secret")
        .and_then(Value::as_str)
        .context("the intent credential the page confirms with")?
        .to_owned();
    assert_eq!(
        browser_confirm(&h, &session.intent_id, &intent_secret).await?,
        200
    );
    drain_worker(&h).await?;

    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("complete".to_owned(), "paid".to_owned()),
        "the settlement transaction must have finished the session"
    );

    // Now sweep, with the horizon moved past. The session is no longer `open`,
    // so there is nothing due — which is the whole point.
    move_every_session_past_its_horizon(&h).await?;
    let endpoints = two_endpoints_for_a();
    sweep_then_fan_out(&h, &endpoints).await?;

    assert_eq!(
        events_of_type(&h.pool, "payment_intent.succeeded")
            .await?
            .len(),
        1,
        "the settlement's own event is the one a merchant gets"
    );
    assert!(
        events_of_type(&h.pool, SESSION_EXPIRED).await?.is_empty(),
        "a settled session must not also be reported as expired"
    );
    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("complete".to_owned(), "paid".to_owned()),
        "and the sweep must not have touched it"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 14e: the event is listable and retrievable through `/v1/events`,
/// scoped to the merchant it belongs to.
///
/// `GET /v1/events` is the documented fallback for a merchant who missed a
/// delivery (`docs/flows/webhooks.md`), so an event they cannot read there is
/// an event they have no way to recover. The tenancy half is the same rule
/// every other read on this surface keeps: another tenant's event is
/// indistinguishable from one that does not exist.
#[tokio::test]
async fn an_expiry_event_is_listable_and_retrievable_within_its_tenant() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    move_every_session_past_its_horizon(&h).await?;

    let endpoints = two_endpoints_for_a();
    sweep_then_fan_out(&h, &endpoints).await?;

    let events = events_of_type(&h.pool, SESSION_EXPIRED).await?;
    let [(event_id, ..)] = events.as_slice() else {
        anyhow::bail!("exactly one event was expected: {events:?}");
    };

    let (status, list) = list_events(&h, CLIENT_A).await?;
    assert_eq!(status, 200, "{list:#}");
    let listed = list
        .pointer("/data/0")
        .context("the merchant's newest event")?;
    assert_eq!(
        listed.get("id").and_then(Value::as_str),
        Some(event_id.as_str())
    );
    assert_eq!(
        listed.get("type").and_then(Value::as_str),
        Some(SESSION_EXPIRED),
        "rendered like every other event: {list:#}"
    );
    assert_eq!(
        listed.pointer("/data/object/id").and_then(Value::as_str),
        Some(session.id.as_str()),
        "with the session under data.object: {list:#}"
    );
    assert_eq!(listed.get("object").and_then(Value::as_str), Some("event"));
    assert_eq!(listed.get("livemode"), Some(&Value::Bool(false)));
    assert!(listed.get("created").and_then(Value::as_i64).is_some());

    let (status, retrieved) = retrieve_event(&h, CLIENT_A, event_id).await?;
    assert_eq!(status, 200, "{retrieved:#}");
    assert_eq!(
        &retrieved, listed,
        "the retrieve and the list must render one event identically"
    );
    // The credential assertion again, this time on what the *API* serves —
    // `GET /v1/events` is a merchant surface and its bytes end up in logs too.
    let served = serde_json::to_string(&retrieved).context("the response serialises")?;
    assert!(
        !served.contains("_secret_") && !served.contains(&session.secret),
        "the API must not serve a credential either (the response is never printed; it is {} \
         bytes for event {event_id})",
        served.len()
    );

    // The tenant boundary. Merchant B holds a valid token for a real client
    // and still cannot see it — and cannot tell it from an id that exists
    // nowhere.
    let unknown = "evt_00000000000000000000000z";
    let (status, body) = retrieve_event(&h, CLIENT_B, event_id).await?;
    assert_eq!(status, 404, "another tenant must get a 404: {body:#}");
    let (unknown_status, unknown_body) = retrieve_event(&h, CLIENT_B, unknown).await?;
    assert_eq!(unknown_status, 404);
    // The two envelopes differ in exactly one place — the id the *caller*
    // supplied, which they already knew — and nowhere else. Compared field by
    // field rather than as whole values, because the messages cannot be equal
    // and the point is that nothing beyond the echo distinguishes them: an
    // "unauthorised" code, a different `type`, or a message naming the owning
    // tenant would each let anyone enumerate which `evt_…` ids exist across
    // the deployment.
    for key in ["code", "type"] {
        assert_eq!(
            body.pointer(&format!("/error/{key}")),
            unknown_body.pointer(&format!("/error/{key}")),
            "the two 404s must agree on `{key}`: {body:#} vs {unknown_body:#}"
        );
    }
    assert_eq!(
        body.pointer("/error/message").and_then(Value::as_str),
        Some(format!("No such event: {event_id}").as_str()),
        "…and each message may echo the caller's own id and nothing else: {body:#}"
    );
    assert_eq!(
        unknown_body
            .pointer("/error/message")
            .and_then(Value::as_str),
        Some(format!("No such event: {unknown}").as_str()),
        "{unknown_body:#}"
    );
    assert_eq!(
        body.as_object().map(serde_json::Map::len),
        unknown_body.as_object().map(serde_json::Map::len),
        "and no extra member may appear on one of them: {body:#} vs {unknown_body:#}"
    );

    let (status, theirs) = list_events(&h, CLIENT_B).await?;
    assert_eq!(status, 200, "{theirs:#}");
    assert_eq!(
        theirs
            .pointer("/data")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "another tenant's list must not carry it: {theirs:#}"
    );

    h.shutdown().await;
    Ok(())
}

/// Claim 14f: **the flip and the event are one transaction.** Make the event
/// insert fail and the session is still `open` afterwards.
///
/// This is the case the whole design exists for. A session reporting `expired`
/// that no merchant was ever told about is invisible: there is no sweep over
/// "expired sessions with no event", no fan-out backlog entry, and nothing
/// that would ever notice — the merchant simply never hears, and their own
/// reconciliation sees an abandoned checkout they were not told about.
///
/// The failure is induced by handing the shipping repository method a `data`
/// that is not a JSON object, which migration 0018's `data_is_object` CHECK
/// refuses. Nothing is monkey-patched and no seam is added: this is
/// `CheckoutSessions::expire_due` as `sweep_expired` calls it, with one
/// argument the renderer could never produce.
///
/// **Revert-proof, measured 2026-09-04:** commit the `UPDATE` before inserting
/// the event — i.e. run the flip on the pool and the event in its own
/// transaction — and this fails with the session `expired` and no event.
#[tokio::test]
async fn a_failed_event_insert_leaves_the_session_open() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    move_every_session_past_its_horizon(&h).await?;

    // The premise: this session *is* due, so a refusal below cannot be the
    // compare-and-swap quietly declining to do anything.
    let due = vpay_db::CheckoutSessions::due_for_expiry(
        h.repositories.as_ref(),
        time::OffsetDateTime::now_utc(),
        100,
    )
    .await?;
    assert_eq!(
        due.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec![session.id.as_str()],
        "the session must be due, or this case proves nothing"
    );

    let event_id = vpay_db::events::event_id();
    let refused = vpay_db::CheckoutSessions::expire_due(
        h.repositories.as_ref(),
        &session.id,
        time::OffsetDateTime::now_utc(),
        &event_id,
        // `data_is_object` (migration 0018) refuses this. The renderer cannot
        // produce it; a schema change that broke the renderer could.
        &serde_json::json!("not an object"),
    )
    .await;
    assert!(
        refused.is_err(),
        "the event insert must fail, or the rest of this case is vacuous: {refused:?}"
    );

    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("open".to_owned(), "unpaid".to_owned()),
        "the flip must have rolled back with the event: a session that says `expired` with no \
         event is one no merchant will ever be told about"
    );
    let (events,): (i64,) = sqlx::query_as("SELECT count(*) FROM events WHERE id = $1")
        .bind(&event_id)
        .fetch_one(&h.pool)
        .await
        .context("counting the refused event")?;
    assert_eq!(events, 0, "and no half-written event may survive either");

    // …and the next sweep still picks it up, which is what makes rolling back
    // the right answer rather than a lost expiry.
    let endpoints = two_endpoints_for_a();
    sweep_then_fan_out(&h, &endpoints).await?;
    assert_eq!(
        stored_session(&h.pool, &session.id).await?.0,
        "expired".to_owned()
    );
    assert_eq!(events_of_type(&h.pool, SESSION_EXPIRED).await?.len(), 1);

    h.shutdown().await;
    Ok(())
}

/// Claim 14g: the live-charge guard is in the **write**, and not only in the
/// read that precedes it.
///
/// `due_for_expiry` and `expire_due` are two statements with a window between
/// them, and a payer confirming inside that window is the entire reason
/// `expire_due` re-evaluates the `NOT EXISTS` rather than trusting the page it
/// was handed. Every other case here goes through the sweep, which reads and
/// writes back to back — so all of them pass with the guard present in the
/// read alone, and none of them notices if the write's copy is deleted as a
/// duplicate of it. Measured 2026-09-04: deleting the `NOT EXISTS` from
/// `expire_due`'s `UPDATE` left all 23 cases in this file green.
///
/// The window is staged rather than raced, so the case is deterministic: read
/// the page the sweep would have read, *then* let the payer confirm, then run
/// the write with the row the sweep is still holding.
///
/// # How the window is opened, and why it changed
///
/// It used to be opened by moving the session's stored `expires_at` into the
/// past and then confirming. That stopped working the moment a confirm began
/// consulting the session (`vpay_api::v1::return_trip`): a session past its
/// horizon refuses the confirm on the **read**, whatever its `status`, so the
/// payer in this story could no longer act — the case failed with `409` where
/// it expected `200`, which is the new rule doing its job rather than a
/// regression.
///
/// So the window is opened the other way round, using the instant the sweep
/// carries: the session keeps its real horizon and the payer confirms while
/// inside it, and the sweep is run *at an instant 25 hours from now*, which
/// `expire_due`'s own documentation offers for exactly this ("a test can
/// sweep a future instant instead of rewriting a stored horizon"). Nothing is
/// staged that a deployment does not do — this is the ordinary shape of the
/// race, a payer who confirmed shortly before the horizon and a rail that is
/// still holding the payment when the sweep arrives.
///
/// What the guard is worth: without it that session is expired and its
/// merchant is told the checkout was abandoned, while the rail is holding a
/// live payment — and the settlement transaction that arrives minutes later
/// cannot record it, because `settle_for_intent`'s own `WHERE status = 'open'`
/// no longer matches. The session would sit `expired`/`unpaid` over a payment
/// that succeeded.
#[tokio::test]
async fn a_payer_confirming_between_the_read_and_the_write_keeps_the_session() -> anyhow::Result<()>
{
    let h = harness().await?;
    let session = hosted_session(&h).await?;

    // Read while the session is still inside its horizon: both browser reads
    // stop at it whatever the status
    // (`both_browser_reads_stop_at_the_horizon_whatever_the_status`), and the
    // payer in this story is one who has been sitting on an open page.
    let (_status, body) = session_read(&h, &session.id, PK_A, &session.secret).await?;
    let intent_secret = body
        .pointer("/payment_intent/client_secret")
        .and_then(Value::as_str)
        .context("the intent credential the page confirms with")?
        .to_owned();

    // The read half, exactly as `expire_due_sessions` runs it. `now` is held
    // and reused below, because the sweep uses one instant for both halves —
    // and it is an hour past this session's 24-hour horizon, so the row is
    // due to *the sweep* while remaining live to the payer confirming below.
    // See this case's own doc for why the horizon is not rewritten instead.
    let now = time::OffsetDateTime::now_utc() + time::Duration::hours(25);
    let due = vpay_db::CheckoutSessions::due_for_expiry(h.repositories.as_ref(), now, 100).await?;
    let row = due
        .iter()
        .find(|row| row.id == session.id)
        .context("the session must be due at the read, or this case proves nothing")?;

    // The window: the payer confirms after the page was read and before the
    // write runs.
    assert_eq!(
        browser_confirm(&h, &session.intent_id, &intent_secret).await?,
        200
    );
    let (state,): (String,) =
        sqlx::query_as("SELECT state::TEXT FROM charges WHERE payment_intent_id = $1")
            .bind(&session.intent_id)
            .fetch_one(&h.pool)
            .await
            .context("the charge the guard is guarding")?;
    assert!(
        ["submitting", "submitted", "pending", "unresolved"].contains(&state.as_str()),
        "the charge must be live before the write, or this case proves nothing: {state}"
    );

    // The write half, with the row the sweep read before any of that
    // happened — and the object it would have rendered from it.
    let data = serde_json::to_value(vpay_api::model::CheckoutSessionObject::expired_snapshot(
        row,
    ))
    .context("the rendered snapshot serialises")?;
    let event_id = vpay_db::events::event_id();
    let written = vpay_db::CheckoutSessions::expire_due(
        h.repositories.as_ref(),
        &session.id,
        now,
        &event_id,
        &data,
    )
    .await?;

    assert!(
        written.is_none(),
        "the write must re-check the live charge and refuse: {written:?}"
    );
    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("open".to_owned(), "unpaid".to_owned()),
        "a session a rail is holding must survive a sweep that read it before the payer confirmed"
    );
    assert!(
        events_of_type(&h.pool, SESSION_EXPIRED).await?.is_empty(),
        "and its merchant must not be told the checkout was abandoned"
    );

    h.shutdown().await;
    Ok(())
}

// ------------------------------------- a session that is over refuses a confirm

/// The **intent's** `client_secret`, read the way vpay's page reads it: out
/// of the session read, while the session is still open and inside its
/// horizon.
///
/// Every case below takes it *first* and then ends the session, because that
/// is the situation being defended against — a payer whose page loaded before
/// the checkout was abandoned, holding a credential nothing revokes. The
/// intent's `client_secret` is minted at create and lives as long as the
/// intent (`docs/flows/browser-checkout.md`): there is no rotation endpoint,
/// so the session's `status` is the only thing that can stop it.
async fn intent_secret_of(h: &Harness, session: &Session) -> anyhow::Result<String> {
    let (status, body) = session_read(h, &session.id, PK_A, &session.secret).await?;
    anyhow::ensure!(
        status == 200,
        "the session read must succeed; it answered {status} ({})",
        error_code_or_none(&body)
    );
    Ok(body
        .pointer("/payment_intent/client_secret")
        .and_then(Value::as_str)
        // Not the body: this is the one that carries the credential.
        .context("an open session hands the page the intent's client_secret")?
        .to_owned())
}

/// The intent is untouched and the confirm wrote nothing — the assertion
/// every refusal case below ends with.
///
/// "Wrote nothing" is asserted against the three rows a confirm creates
/// before it ever reaches a rail (`docs/reference/vpay-api.md` § the confirm
/// path, steps 3 and 4), not against the response: a `409` that had already
/// committed a charge would look identical from outside and would burn the
/// intent's one charge forever (`one_charge_per_intent`).
async fn assert_refused_without_writing(h: &Harness, intent_id: &str) -> anyhow::Result<()> {
    assert_eq!(
        write_footprint(&h.pool, intent_id).await?,
        (0, 0, 0),
        "a refused confirm must leave no charge, no provider_requests row and no job"
    );
    // `GET /v1/payment_intents/{id}` renders the intent's `client_secret`
    // (`SecretRendering::Include`), so the body stays out of the messages.
    let (status, body) = retrieve_intent(h, intent_id).await?;
    assert_eq!(
        status,
        200,
        "the intent must still be readable ({})",
        error_code_or_none(&body)
    );
    assert_eq!(
        body.get("status").and_then(Value::as_str),
        Some("requires_payment_method"),
        "the intent must not have moved"
    );
    Ok(())
}

/// Claim 18: the hourly sweep expires a session, and the payer still holding
/// the intent's credential is refused — `409 checkout_session_expired`,
/// before any charge is opened.
///
/// This is the defect the case exists for, and it was reachable end to end
/// before this landed: the sweep flips the session to `expired` **and emits
/// `checkout.session.expired`**, telling the merchant the checkout was
/// abandoned; the confirm did not consult the session; and the settlement's
/// own `WHERE status = 'open'` guard then correctly declined to touch the row
/// — leaving `expired`/`unpaid` under a `succeeded` intent, with the merchant
/// holding a webhook that said the opposite.
///
/// The sweep is the shipping `vpay_worker::run_once` over the shipping
/// `seed_singletons`, not an `UPDATE`: what is claimed is that a *deployment*
/// produces this state.
///
/// **Revert-proof.** Delete the `return_trip::admit_confirm` call from
/// `confirm_once` and this fails on the status — the confirm answers `200`
/// and opens a charge on an abandoned checkout.
#[tokio::test]
async fn a_confirm_on_a_swept_session_is_refused_before_any_charge() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    let intent_secret = intent_secret_of(&h, &session).await?;

    move_every_session_past_its_horizon(&h).await?;
    vpay_worker::seed_singletons(h.repositories.as_ref())
        .await
        .context("seeding the singleton jobs a worker seeds at boot")?;
    run_until(&h, &support::no_webhook_endpoints(), "sweep_expired").await?;
    assert_eq!(
        stored_session(&h.pool, &session.id).await?,
        ("expired".to_owned(), "unpaid".to_owned()),
        "the sweep must have expired the session, or this case proves nothing"
    );

    let (status, body) = browser_confirm_response(&h, &session.intent_id, &intent_secret).await?;
    assert_eq!(
        status,
        409,
        "the swept session must refuse the confirm ({})",
        error_code_or_none(&body)
    );
    assert_eq!(error_code(&body)?, "checkout_session_expired");
    assert!(
        body.pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains(&session.id)),
        "the refusal must name the session that refused"
    );

    assert_refused_without_writing(&h, &session.intent_id).await?;

    h.shutdown().await;
    Ok(())
}

/// Claim 19: the same refusal when the **merchant** ended the checkout
/// through `POST /v1/checkout/sessions/{id}/expire`.
///
/// The session is still inside its 24-hour horizon here — only its `status`
/// moved — so this case is decided by the `status` and the previous one could
/// have been decided by either. Together they pin both halves.
///
/// The merchant's own abandon emits no event (`docs/flows/webhooks.md`: the
/// caller already knows), which makes this the *quieter* of the two ways to
/// end a checkout and the one where a payer paying anyway would be hardest to
/// notice.
#[tokio::test]
async fn a_confirm_on_a_session_the_merchant_expired_is_refused() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    let intent_secret = intent_secret_of(&h, &session).await?;

    let (status, body) = expire_session(&h, &session.id).await?;
    assert_eq!(
        status,
        200,
        "the merchant's own expire must succeed ({})",
        error_code_or_none(&body)
    );
    assert_eq!(field(&body, "status")?, "expired");

    let (status, body) = browser_confirm_response(&h, &session.intent_id, &intent_secret).await?;
    assert_eq!(
        status,
        409,
        "a session the merchant expired must refuse the confirm ({})",
        error_code_or_none(&body)
    );
    assert_eq!(error_code(&body)?, "checkout_session_expired");

    assert_refused_without_writing(&h, &session.intent_id).await?;

    h.shutdown().await;
    Ok(())
}

/// Claim 20: a session that is still `open` but past `expires_at`, which no
/// sweep has reached, is refused too — **and the refusal writes nothing to
/// the session either**.
///
/// The read decides, exactly as `browser::checkout_sessions::authenticate`'s
/// sixth refusal does. That matters twice over. A worker that is down, or a
/// backlog the hourly sweep has not drained, must not be the difference
/// between a payer being able to pay and not — and a *confirm* is the wrong
/// place to repair a row: flipping it here would emit no
/// `checkout.session.expired` (the sweep's transaction is what does that) and
/// would skip the `NOT EXISTS` live-charge guard that transaction carries.
///
/// So `updated_at` is asserted unchanged, not merely `status`: a write that
/// set `status = 'expired'` and one that set it to what it already was would
/// both leave `status` reading `open` if the second were ever attempted, and
/// only the stamp tells them apart.
///
/// **Revert-proof.** Make `return_trip::verdict` consult `status` alone —
/// drop `now < session.expires_at` from its `open` arm — and this fails with
/// a `200`: an abandoned checkout the sweep has not reached yet takes a
/// payment.
#[tokio::test]
async fn a_confirm_past_the_horizon_is_refused_by_the_read_and_writes_nothing() -> anyhow::Result<()>
{
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    let intent_secret = intent_secret_of(&h, &session).await?;

    move_every_session_past_its_horizon(&h).await?;
    let before = stored_session_stamp(&h.pool, &session.id).await?;
    assert_eq!(
        before.0, "open",
        "the row must still say `open`, or this proves the sweep rather than the read"
    );

    let (status, body) = browser_confirm_response(&h, &session.intent_id, &intent_secret).await?;
    assert_eq!(
        status,
        409,
        "an open session past its horizon must refuse the confirm ({})",
        error_code_or_none(&body)
    );
    assert_eq!(error_code(&body)?, "checkout_session_expired");

    assert_eq!(
        stored_session_stamp(&h.pool, &session.id).await?,
        before,
        "the confirm must not have touched the session — not its status, and not its stamp"
    );
    assert_refused_without_writing(&h, &session.intent_id).await?;

    h.shutdown().await;
    Ok(())
}

/// Claim 21: a `complete` session answers a **different** code,
/// `checkout_session_complete`.
///
/// # This case stages a row, and says so
///
/// `complete` is written by exactly one thing — `vpay_db`'s settlement
/// transaction, which sets it in the same commit as the intent reaching
/// `succeeded` — so through the shipping API a `complete` session always sits
/// under a `succeeded` intent, and `load_confirmable_intent` refuses that one
/// step earlier with `invalid_state`. **No sequence of shipping operations
/// reaches this branch today**, and pretending otherwise by driving a real
/// settlement here would test the status refusal instead.
///
/// It is kept, and tested by writing the state directly, because the code is
/// reached the moment those two facts stop being welded together — a session
/// completed by anything that is not the settlement, or an intent status the
/// lifecycle admits a confirm from after a session finished. What the branch
/// must not do is fall through to "expired": a merchant told their payer
/// abandoned a checkout that was in fact paid would be vpay reporting the
/// opposite of what the money did.
#[tokio::test]
async fn a_confirm_on_a_complete_session_is_a_different_code() -> anyhow::Result<()> {
    let h = harness().await?;
    let session = hosted_session(&h).await?;
    let intent_secret = intent_secret_of(&h, &session).await?;

    // See this test's own doc: a state the shipping code only ever produces
    // alongside a `succeeded` intent, written here without one so that the
    // *session* is what decides the answer.
    sqlx::query(
        "UPDATE checkout_sessions SET status = 'complete', payment_status = 'paid' WHERE id = $1",
    )
    .bind(&session.id)
    .execute(&h.pool)
    .await
    .context("staging a complete session under an unsettled intent")?;

    let (status, body) = browser_confirm_response(&h, &session.intent_id, &intent_secret).await?;
    assert_eq!(
        status,
        409,
        "a complete session must refuse the confirm ({})",
        error_code_or_none(&body)
    );
    assert_eq!(
        error_code(&body)?,
        "checkout_session_complete",
        "a finished checkout is not an abandoned one, and the code is the difference"
    );

    assert_refused_without_writing(&h, &session.intent_id).await?;

    h.shutdown().await;
    Ok(())
}

/// Claim 22: an intent with **no** checkout session confirms exactly as it
/// did before — the case that keeps the refusal from being a blanket one.
///
/// Most confirms in this deployment are this: a merchant's own page, or its
/// server, against an intent no session was ever created for. The assertion
/// is not only the `200`; it is the three rows a successful confirm writes,
/// so a change that refused *everything* and a change that merely answered
/// `200` are told apart.
#[tokio::test]
async fn an_intent_with_no_session_confirms_exactly_as_before() -> anyhow::Result<()> {
    let h = harness().await?;
    let (intent_id, intent_secret) = create_intent_with_secret(&h).await?;

    let (status, body) = browser_confirm_response(&h, &intent_id, &intent_secret).await?;
    assert_eq!(
        status,
        200,
        "an intent with no session must confirm exactly as before ({})",
        error_code_or_none(&body)
    );
    assert_eq!(
        body.get("id").and_then(Value::as_str),
        Some(intent_id.as_str())
    );

    let (charges, requests, jobs) = write_footprint(&h.pool, &intent_id).await?;
    assert_eq!(charges, 1, "the confirm opened its one charge");
    assert_eq!(
        requests, 1,
        "and recorded the attempt against the rail before making it"
    );
    assert_eq!(jobs, 1, "and left the poll job that settles it");

    h.shutdown().await;
    Ok(())
}

/// Claim 23: the **merchant's** `/v1` confirm is refused by the same session,
/// with the same code — and a retry under the same `Idempotency-Key` replays
/// the stored `409` rather than re-deciding it.
///
/// # Why both surfaces and not only the browser's
///
/// The refusal lives in `confirm_once`, which both surfaces share, so this is
/// what the shared implementation buys rather than a second rule. It is worth
/// having deliberately rather than by accident: `/v1`'s confirm is not
/// authenticated by the payer's `client_secret` at all, so a merchant server
/// that kept confirming after telling its own systems the checkout was
/// abandoned would produce exactly the contradiction the browser refusal
/// prevents — a `succeeded` intent under an `expired`/`unpaid` session, and a
/// `checkout.session.expired` webhook already delivered.
///
/// The replay half is this surface's own: `PostRequest::finish` stores a
/// `4xx`, so the second call under the same key never reaches `confirm_once`.
///
/// Byte equality alone does **not** prove that. A retry that re-executed
/// would decide the same way against the same rows and produce an identical
/// body, so the assertion would pass either way — measured: with
/// `PostRequest::finish` mutated to release a `4xx` instead of storing it,
/// this case still passed. So the world is changed between the two calls,
/// the way `payment_intents::a_replay_survives_the_rail_being_disabled` does
/// it: the merchant expires the checkout, is refused, and *then* creates a
/// second, open session on the same intent — which is precisely the state
/// `a_second_session_after_an_expiry_makes_the_intent_payable_again` proves a
/// fresh confirm answers `200` from. A re-executed retry would therefore open
/// a charge; the stored one is still the `409`, and the intent has still
/// never been charged.
#[tokio::test]
async fn the_merchant_confirm_is_refused_too_and_the_replay_is_the_stored_409() -> anyhow::Result<()>
{
    let h = harness().await?;
    let session = hosted_session(&h).await?;

    let (status, body) = expire_session(&h, &session.id).await?;
    assert_eq!(
        status,
        200,
        "the merchant's own expire must succeed ({})",
        error_code_or_none(&body)
    );

    let key = uuid::Uuid::new_v4().to_string();
    let (status, first) = merchant_confirm(&h, &session.intent_id, &key).await?;
    assert_eq!(status, 409, "{first:#}");
    assert_eq!(error_code(&first)?, "checkout_session_expired");

    // The world the retry would re-decide against, if it re-decided.
    let (status, body) = create_session(
        &h,
        CLIENT_A,
        &[
            ("payment_intent", session.intent_id.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    assert_eq!(
        status,
        201,
        "a second session on the same intent ({})",
        error_code_or_none(&body)
    );

    let (status, replayed) = merchant_confirm(&h, &session.intent_id, &key).await?;
    assert_eq!(
        status, 409,
        "the retry answers the stored 409 even though a fresh confirm would now be admitted:          {replayed:#}"
    );
    assert_eq!(
        replayed, first,
        "the retry under the same key must be the stored response, not a second decision"
    );

    assert_refused_without_writing(&h, &session.intent_id).await?;

    h.shutdown().await;
    Ok(())
}

/// Claim 24: expiring a session and creating a **second** one leaves the
/// intent payable — the ordinary "the payer walked away, offer them a fresh
/// link" flow, which the refusal must not break.
///
/// This is the case that makes `find_latest_by_intent` the right question.
/// An intent can carry several sessions over its life — `create` refuses only
/// an intent that already has an *open* one — so a rule of the shape "refuse
/// if any session on this intent is not open" would refuse this confirm, and
/// a merchant could never re-offer a checkout. The newest row is the one that
/// decides, and `checkout_sessions_one_open_per_intent` is what makes "the
/// open one is the newest" true rather than hoped for.
///
/// **Revert-proof.** Point the gate at any session on the intent rather than
/// the newest — or order it ascending — and this fails with a `409`
/// `checkout_session_expired` on a checkout that is open in front of the
/// payer.
#[tokio::test]
async fn a_second_session_after_an_expiry_makes_the_intent_payable_again() -> anyhow::Result<()> {
    let h = harness().await?;
    let first = hosted_session(&h).await?;

    let (status, body) = expire_session(&h, &first.id).await?;
    assert_eq!(
        status,
        200,
        "the merchant's own expire must succeed ({})",
        error_code_or_none(&body)
    );

    // The same intent, a second session. `create` allows it precisely because
    // no session on this intent is open any more.
    let (status, body) = create_session(
        &h,
        CLIENT_A,
        &[
            ("payment_intent", first.intent_id.as_str()),
            ("success_url", SUCCESS_URL),
            ("cancel_url", CANCEL_URL),
        ],
    )
    .await?;
    assert_eq!(
        status,
        201,
        "a second session on the same intent ({})",
        error_code_or_none(&body)
    );
    let second = Session {
        id: field(&body, "id")?,
        secret: field(&body, "client_secret")?,
        intent_id: first.intent_id.clone(),
        url: field(&body, "url")?,
    };
    assert_ne!(second.id, first.id);
    assert!(!second.url.is_empty());

    let intent_secret = intent_secret_of(&h, &second).await?;
    let (status, body) = browser_confirm_response(&h, &second.intent_id, &intent_secret).await?;
    assert_eq!(
        status,
        200,
        "the newest session is open, so the payer on it must be able to pay ({})",
        error_code_or_none(&body)
    );

    // And it really opened a charge, rather than answering `200` off some
    // path that never wrote one.
    let (charges, requests, jobs) = write_footprint(&h.pool, &second.intent_id).await?;
    assert_eq!((charges, requests, jobs), (1, 1, 1));

    // The expired session is untouched by the payment that went through the
    // new one: `settle_for_intent`'s `WHERE status = 'open'` sees the second
    // row, and this one stays as the merchant left it.
    assert_eq!(
        stored_session(&h.pool, &first.id).await?,
        ("expired".to_owned(), "unpaid".to_owned())
    );

    h.shutdown().await;
    Ok(())
}
