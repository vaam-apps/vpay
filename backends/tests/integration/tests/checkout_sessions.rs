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
    Ok(response.status().as_u16())
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
