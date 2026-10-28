//! `POST /dash/v1/$procs/searchPaymentIntents`, end to end: the real
//! `vpay_api::router` on a real socket, over a real Postgres — Lane C
//! (`docs/plans/2026-09-13-dashboard-nav-notes/transport.md`), signed under a
//! paired console key since VP-6a.
//!
//! Sibling of `dashboard_read_surface.rs` rather than an addition to it: that
//! suite is about `/dash/v1`'s hand-written `GET` routes, and this one is about
//! the CrateStack procedure transport mounted *beside* them, behind its **own**
//! `EnvelopeLayer` — the one the generated `envelope_layer` builds over this
//! schema's `ROUTE_TRANSPORTS` and `ACCEPTED_CONTRACTS`, which `vpay-db` alone
//! can name. The harness is the same (`support::serve`, a console key paired
//! through the shipping repository, a P-256 signer in `support::sealed`), and the
//! call's contract digest is read off `schemas/vpay.cstack` itself, so a bug in
//! the server's generated table cannot hide behind a harness that shares it.
//!
//! # What this proves
//!
//! 1. The procedure answers over HTTP with the same rows
//!    `search_payment_intents.rs`'s own container test asserts: the caller's
//!    own merchant's intents, newest first, and never another merchant's.
//! 2. An unsigned caller is the layer's coarse `401`, before CrateStack's own
//!    `@allow(auth() != null)` policy is ever consulted; a revoked key and a
//!    merchant's key are the same.
//! 3. An unknown `/dash/v1` path is still the honest `404`, in this crate's
//!    envelope, and a malformed body is refused without naming the tenant.
//!
//! The tenant-mismatch mutation (replacing `search_payment_intents.rs`'s
//! `WHERE merchant_id = $1` with a tautology) is `vpay-db`'s own
//! container-backed proof, re-run for this lane and reported rather than
//! duplicated here.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context as _;
use serde_json::Value;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use time::OffsetDateTime;
use vpay_config::{Config, CurrencyEntry, Dashboard, Deployment, HostEntry, ProviderHost};
use vpay_db::{NewPaymentIntent, Repositories};

mod support;

use support::sealed::SealedHttp;
use support::{
    assert_coarse_401, create_staff, ensure_crypto_provider_installed, generate_key,
    merchant_client, migrated_postgres, pair_console_key, serve,
};

const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

/// The plain member this suite's console key is paired to, and the key's label.
const STAFF_ID: &str = "stf_00000000000000000procs";
const CONSOLE: &str = "procs-reader";

const PUSH_RAIL: &str = "mtn_momo";
const CURRENCY: &str = "XAF";
const AMOUNT: i64 = 5_000;

/// The path CrateStack's generated `procedure_router` mounts —
/// `/$procs/<name>` — nested under `dash::DASH_NEST`.
const PROCS_PATH: &str = "/dash/v1/$procs/searchPaymentIntents";

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    #[allow(dead_code)]
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    base_url: String,
}

impl Harness {
    /// `POST {PROCS_PATH}` with an empty filter and no page override — the
    /// caller's own tenant's default page — signed by console key `label`, or
    /// unsigned when `None`. Decoded as JSON.
    async fn search_payment_intents(&self, label: Option<&str>) -> anyhow::Result<(u16, Value)> {
        self.post_body(
            PROCS_PATH,
            label,
            &serde_json::json!({ "page": {}, "filter": {} }),
        )
        .await
    }

    /// `POST path` with a JSON `body`.
    async fn post_body(
        &self,
        path: &str,
        label: Option<&str>,
        body: &Value,
    ) -> anyhow::Result<(u16, Value)> {
        let url = format!("{}{path}", self.base_url);
        let (status, text) = match label {
            Some(label) => {
                let answer = SealedHttp::new()
                    .post(&url)
                    .signed_by_console(label)
                    .json(body)
                    .send()
                    .await
                    .context("sending a signed POST to the procedure transport")?;
                (
                    answer.status().as_u16(),
                    String::from_utf8_lossy(&answer.bytes().await?).into_owned(),
                )
            }
            None => {
                let response = reqwest::Client::builder()
                    .build()
                    .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
                    .post(&url)
                    .header("content-type", "application/json")
                    .header("accept", "application/json")
                    .body(serde_json::to_vec(body)?)
                    .send()
                    .await
                    .context("sending an unsigned POST to the procedure transport")?;
                (response.status().as_u16(), response.text().await?)
            }
        };
        Ok((
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        ))
    }
}

fn config_with(base_url: &str, jwks_a: Value, jwks_b: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "dashboard-procedure-transport".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
            surfaces: None,
        },
        providers: vec![ProviderHost {
            code: PUSH_RAIL.to_owned(),
            enabled: true,
            host: HostEntry {
                // Unreachable on purpose: this suite reads rows, it never
                // confirms anything, so no rail is ever called.
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
            currency: CURRENCY.to_owned(),
            credentials: BTreeMap::from([
                (
                    "subscription_key".to_owned(),
                    "stub-subscription-key".to_owned(),
                ),
                ("api_key".to_owned(), "stub-api-key".to_owned()),
            ]),
            display_name: None,
            callbacks: None,
        }],
        currencies: vec![CurrencyEntry {
            code: CURRENCY.to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![
            merchant_client(CLIENT_A, MERCHANT_A, jwks_a),
            merchant_client(CLIENT_B, MERCHANT_B, jwks_b),
        ],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard: Some(Dashboard {
            merchant_id: MERCHANT_A.to_owned(),
        }),
        service_clients: Vec::new(),
        routing: Default::default(),
        auth: Default::default(),
    }
}

async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (container, repositories, _pool) = migrated_postgres().await?;

    let (server_pem, _server_jwks) = generate_key();
    let (_pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();

    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, jwks_a, jwks_b)
    })
    .await?;

    create_staff(
        repositories.as_ref(),
        STAFF_ID,
        MERCHANT_A,
        "dash-procs-reader@example.test",
        false,
    )
    .await?;
    // A key and no password, no session and no token of any kind: the row
    // needs nothing but a console key to read.
    pair_console_key(
        repositories.as_ref(),
        STAFF_ID,
        CONSOLE,
        OffsetDateTime::now_utc(),
    )
    .await?;

    Ok(Harness {
        _container: container,
        server: served.server,
        repositories,
        base_url: served.base_url,
    })
}

async fn seed_intent(
    repositories: &dyn Repositories,
    merchant_id: &str,
    id: &str,
) -> anyhow::Result<()> {
    repositories
        .insert(&NewPaymentIntent {
            id: id.to_owned(),
            merchant_id: merchant_id.to_owned(),
            livemode: false,
            amount: AMOUNT,
            currency_code: CURRENCY.to_owned(),
            status: vpay_core::IntentStatus::INITIAL.as_wire_str().to_owned(),
            payment_method_types: serde_json::json!([PUSH_RAIL]),
            metadata: serde_json::json!({}),
            description: None,
            customer_id: None,
            last_payment_error_code: None,
            last_payment_error_message: None,
            client_secret_suffix: format!("procsuffixmarker{id:x<32}"),
            created_at: time::OffsetDateTime::now_utc(),
        })
        .await
        .with_context(|| format!("seeding {id} for {merchant_id}"))?;
    Ok(())
}

fn at<'a>(body: &'a Value, pointer: &str) -> &'a Value {
    body.pointer(pointer)
        .unwrap_or_else(|| panic!("{pointer} is absent from {body}"))
}

/// Criterion 1 of Lane C's "done": `searchPaymentIntents` answers over HTTP
/// with the same rows `search_payment_intents.rs`'s own container test
/// asserts — the caller's own merchant, newest first, never another
/// merchant's.
///
/// Decisive the same way `dashboard_read_surface.rs`'s
/// `the_dashboard_lists_only_the_merchant_it_is_bound_to` is: seeding both
/// merchants and asserting the *other* one's row is absent is what a dropped
/// `WHERE merchant_id = $1` (or a context built from
/// `persistence::system_context()`, which carries no tenant at all) would fail.
#[tokio::test]
async fn the_procedure_answers_the_callers_own_merchant_over_http() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();

    seed_intent(repositories, MERCHANT_A, "pi_procs_a_one").await?;
    seed_intent(repositories, MERCHANT_A, "pi_procs_a_two").await?;
    seed_intent(repositories, MERCHANT_B, "pi_procs_b_one").await?;

    let (status, body) = harness.search_payment_intents(Some(CONSOLE)).await?;
    assert_eq!(status, 200, "{body}");

    let ids: Vec<&str> = at(&body, "/items")
        .as_array()
        .expect("the page envelope carries an items array")
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();

    assert_eq!(ids, ["pi_procs_a_two", "pi_procs_a_one"], "{body}");
    assert!(
        ids.iter().all(|id| *id != "pi_procs_b_one"),
        "the caller is bound to {MERCHANT_A} and must not see {MERCHANT_B}'s row through the \
         procedure transport: {body}"
    );
    Ok(())
}

/// Criterion 2 of Lane C's "done": an unsigned caller is refused — the layer's
/// own coarse `401`, before CrateStack's `@allow(auth() != null)` policy is
/// ever consulted. A merchant's key and a revoked console key are the same
/// answer.
///
/// The decisive mutation: build the procedure layer with `EnvelopeMode::Optional`
/// (or drop it): the unsigned call then reaches `dash_scope` and is a `500`, and
/// the assertion on the layer's body names it.
#[tokio::test]
async fn an_unsigned_caller_a_merchant_and_a_revoked_key_are_the_coarse_401() -> anyhow::Result<()>
{
    let harness = harness().await?;
    let url = format!("{}{PROCS_PATH}", harness.base_url);
    let body = serde_json::json!({ "page": {}, "filter": {} });

    let (status, unsigned) = harness.search_payment_intents(None).await?;
    assert_coarse_401(
        status,
        false,
        unsigned.to_string().as_bytes(),
        "an unsigned procedure call",
    );

    let merchant = SealedHttp::new()
        .post(&url)
        .signed_by(CLIENT_A)
        .json(&body)
        .send()
        .await?;
    assert_coarse_401(
        merchant.status().as_u16(),
        merchant.is_sealed(),
        &merchant.bytes().await?,
        "a merchant's key on the procedure transport",
    );

    // The control reads; then the key is revoked; the next call is refused.
    let (control, control_body) = harness.search_payment_intents(Some(CONSOLE)).await?;
    assert_eq!(control, 200, "{control_body}");
    let thumbprint = support::sealed::console_thumbprint(CONSOLE);
    assert!(
        harness
            .repositories
            .revoke_console_key(&thumbprint, OffsetDateTime::now_utc())
            .await?
    );
    let revoked = SealedHttp::new()
        .post(&url)
        .signed_by_console(CONSOLE)
        .json(&body)
        .send()
        .await?;
    assert_coarse_401(
        revoked.status().as_u16(),
        revoked.is_sealed(),
        &revoked.bytes().await?,
        "a revoked key on the procedure transport",
    );
    Ok(())
}

/// A bodiless anonymous `OPTIONS` on the procedure transport is the router's
/// `405`, not a `500` (see `dashboard_read_surface.rs`).
#[tokio::test]
async fn an_anonymous_options_on_the_procedure_is_not_a_500() -> anyhow::Result<()> {
    let harness = harness().await?;
    let response = reqwest::Client::builder()
        .build()
        .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
        .request(
            reqwest::Method::OPTIONS,
            format!("{}{PROCS_PATH}", harness.base_url),
        )
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 405);
    Ok(())
}

/// **`/dash/v1`'s uniform-404 property, which Lane C must not have widened.**
///
/// `dash::routes()`'s own fallback is `vpay_api::not_found`, so an operator who
/// mistypes a path is told "no such route", in this crate's own error envelope,
/// and nothing about what else this deployment mounts.
///
/// **Decisive for the way Lane C mounts its transport.** The procedure router is
/// attached with `Router::nest_service(DASH_NEST, ..)`, which registers a
/// *catch-all* in the outer **path** router that beats the fallback router. Without
/// the `.fallback(not_found)` added *outside* the procedure layers, every
/// previously-unmatched `/dash/v1/...` path reaches CrateStack's own fallback.
///
/// A COSE-signed call to a path no contract names is the layer's `415` (it has
/// no op to bind a signature to); an anonymous one is the `404`.
#[tokio::test]
async fn an_unknown_dash_path_is_still_the_honest_404() -> anyhow::Result<()> {
    let harness = harness().await?;
    let http = reqwest::Client::builder()
        .build()
        .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed");

    let response = http
        .get(format!(
            "{}/dash/v1/not_a_dashboard_route",
            harness.base_url
        ))
        .header("accept", "application/json")
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 404);
    let body: Value = response.json().await?;
    assert_eq!(
        body.pointer("/error/type").and_then(Value::as_str),
        Some("invalid_request_error"),
        "the 404 must be this crate's own error envelope, not CrateStack's: {body}"
    );

    // And a POST on a path that is not a procedure.
    let (status, body) = harness
        .post_body(
            "/dash/v1/$procs/noSuchProcedure",
            None,
            &serde_json::json!({}),
        )
        .await?;
    assert_eq!(
        status, 404,
        "a procedure this schema does not declare must be a 404: {body}"
    );
    Ok(())
}

/// ADR-0033 D6: the merchant MCP procedures live in the same schema, and
/// CrateStack's generated procedure router mounts every procedure it
/// declares. A console signed under a paired key, naming one, gets the same
/// honest 404 a procedure the schema does not declare gets, and nothing on
/// the merchant's books moves.
///
/// Removing `only_dashboard_procedures` from `vpay_api::router` makes this
/// a `403` (the procedure's own `@allow` refusing a context with no
/// `audience` claim) rather than a `404`, and that is the mutation this
/// pins: the allowlist is the outer wall, the policy the inner one. The
/// allowlist sits outside the envelope, so the `404` does not depend on the
/// signature either.
#[tokio::test]
async fn a_merchant_mcp_procedure_is_not_served_to_the_dashboard() -> anyhow::Result<()> {
    let harness = harness().await?;

    for procedure in [
        "openPaymentIntent",
        "confirmPaymentIntent",
        "cancelPaymentIntent",
        "retrievePaymentIntent",
        "issueRefund",
        "cancelRefund",
        "retrieveRefund",
    ] {
        let (status, body) = harness
            .post_body(
                &format!("/dash/v1/$procs/{procedure}"),
                Some(CONSOLE),
                &serde_json::json!({
                    "args": { "idempotency_key": "k", "amount": 100, "currency": "XAF",
                              "payment_method_types": [], "metadata": [] }
                }),
            )
            .await?;
        assert_eq!(
            status, 404,
            "`{procedure}` is a merchant MCP procedure and /dash/v1/$procs must not serve it: \
             {body}"
        );
    }
    Ok(())
}

/// **A malformed body is a refusal that says nothing about the tenant.**
///
/// Criterion 2's other half: a caller who *has* a key and sends nonsense is
/// told only that the body was unacceptable — never whether the tenant, or any
/// id inside the body, exists.
#[tokio::test]
async fn a_malformed_body_is_refused_without_naming_the_tenant() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_procs_a_mal").await?;

    // The control: the same key reads, so what follows is about the body and not
    // about the key or the binding (a refusal from the layer would also be a
    // 4xx, and would make this case pass for the wrong reason).
    let (control, control_body) = harness.search_payment_intents(Some(CONSOLE)).await?;
    assert_eq!(control, 200, "{control_body}");

    let (status, body) = harness
        .post_body(
            PROCS_PATH,
            Some(CONSOLE),
            // `page` is an object in the schema; a string is the cheapest
            // well-formed JSON the decoder must refuse.
            &serde_json::json!({ "page": "not-an-object", "filter": {} }),
        )
        .await?;
    // A 4xx, deliberately not pinned to one code: which 4xx CrateStack's codec
    // chooses for an undecodable argument is its decision. What must hold is
    // that the request is *refused* — not served, and not a 5xx.
    assert!(
        (400..500).contains(&status) && ![401, 415].contains(&status),
        "a malformed procedure body must be refused as a client error by the handler, not by \
         the layer, not served and not a 500: {status} {body}"
    );

    let rendered = body.to_string();
    assert!(
        !rendered.contains(MERCHANT_A),
        "a decode failure must not echo the caller's tenant back: {body}"
    );
    assert!(
        !rendered.contains("pi_procs_a_mal"),
        "a decode failure must not name rows: {body}"
    );
    Ok(())
}
