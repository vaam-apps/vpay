//! `POST /dash/v1/$procs/searchPaymentIntents`, end to end: the real
//! `vpay_api::router` on a real socket, over a real Postgres — Lane C
//! (`docs/plans/2026-09-13-dashboard-nav-notes/transport.md`).
//!
//! Sibling of `dashboard_read_surface.rs` rather than an addition to it:
//! that suite is about `/dash/v1`'s hand-written `GET` routes, and this one
//! is about the CrateStack procedure transport mounted *beside* them —
//! `require_dashboard_procedure_token`, not `require_dashboard_token`, and a
//! `POST` carrying a JSON body rather than a query string. Reusing the same
//! harness pattern (`support::serve`, a token minted by hand through the
//! server's own signing key) is deliberate: this is the same resource
//! server, answering a second surface.
//!
//! # What this proves
//!
//! 1. The procedure answers over HTTP with the same rows
//!    `search_payment_intents.rs`'s own container test asserts: the caller's
//!    own merchant's intents, newest first, and never another merchant's —
//!    `dashboard_read_surface.rs`'s `the_dashboard_lists_only_the_merchant_it_is_bound_to`
//!    is the `GET` surface's version of this same claim.
//! 2. An unauthenticated caller is refused, before `require_dashboard_procedure_token`
//!    ever looks at CrateStack's own `@allow(auth() != null)` policy.
//!
//! The tenant-mismatch mutation (replacing `search_payment_intents.rs`'s
//! `WHERE merchant_id = $1` with a tautology) is `vpay-db`'s own
//! container-backed proof, re-run for this lane and reported in the
//! session's summary rather than duplicated here — it is a property of the
//! procedure body, which every transport that ever calls it shares, not of
//! this HTTP surface specifically.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context as _;
use serde_json::Value;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_config::{
    Config, CurrencyEntry, DASHBOARD_MERCHANT_CLAIM, DashboardClient, Deployment, HostEntry,
    ProviderHost,
};
use vpay_db::{NewPaymentIntent, Repositories};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client, migrated_postgres, serve,
};

const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

const DASHBOARD_CLIENT: &str = "vpay-dashboard";
const DASHBOARD_SCOPE: &str = "dashboard:read";

/// The `sub` of every staff token this suite mints — see
/// `dashboard_read_surface.rs`'s identical constant for why it has to be a
/// real, seeded `staff_members` row (ADR-0017, the exp24 review).
const STAFF_ID: &str = "stf_00000000000000000procs";

const PUSH_RAIL: &str = "mtn_momo";
const CURRENCY: &str = "XAF";
const AMOUNT: i64 = 5_000;
const TOKEN_TTL_SECS: u64 = 300;

/// The path CrateStack's generated `procedure_router` mounts —
/// `cratestack-macros-0.12.0/src/axum/procedure/route_attrs.rs`'s
/// `/$procs/<name>` naming — nested under `dash::DASH_NEST`.
const PROCS_PATH: &str = "/dash/v1/$procs/searchPaymentIntents";

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    #[allow(dead_code)]
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    base_url: String,
    signing_key: LoadedSigningKey,
}

impl Harness {
    /// A staff token bound to `MERCHANT_A` — the shape
    /// `vpay_api::staff::oauth::token` mints, exactly as
    /// `dashboard_read_surface.rs::Harness::dashboard_token` builds it.
    fn dashboard_token(&self) -> String {
        let mut extra = std::collections::HashMap::new();
        extra.insert(
            DASHBOARD_MERCHANT_CLAIM.to_owned(),
            Value::String(MERCHANT_A.to_owned()),
        );
        self.signing_key
            .token_manager()
            .issue_user_token_with_extra(
                vpay_api::op::dashboard::token_identity_for(STAFF_ID),
                TOKEN_TTL_SECS,
                Some(DASHBOARD_SCOPE.to_owned()),
                Some(DASHBOARD_CLIENT.to_owned()),
                extra,
            )
            .expect("the server's own signing key mints a token")
    }

    /// `POST {PROCS_PATH}` with an empty filter and no page override — the
    /// caller's own tenant's default page — decoded as JSON. `token: None`
    /// is how the unauthenticated case is asked.
    async fn search_payment_intents(&self, token: Option<&str>) -> anyhow::Result<(u16, Value)> {
        let mut request = reqwest::Client::builder()
            .build()
            .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
            .post(format!("{}{PROCS_PATH}", self.base_url))
            .header("content-type", "application/json")
            .body(serde_json::to_vec(
                &serde_json::json!({ "page": {}, "filter": {} }),
            )?);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .context("sending a POST to the procedure transport")?;
        let status = response.status().as_u16();
        let body = response.text().await.context("reading the body")?;
        Ok((
            status,
            serde_json::from_str(&body).unwrap_or(Value::String(body)),
        ))
    }
}

fn config_with(base_url: &str, jwks_a: Value, jwks_b: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "dashboard-procedure-transport".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
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
        dashboard_client: Some(DashboardClient {
            client_id: DASHBOARD_CLIENT.to_owned(),
            merchant_id: MERCHANT_A.to_owned(),
            redirect_uris: vec![format!("{base_url}/dash/v1/callback")],
            scope: DASHBOARD_SCOPE.to_owned(),
            client_secret: None,
        }),
        // No staff secrets: this suite mints tokens directly, exactly like
        // `dashboard_read_surface.rs` — see that file's identical field for
        // why a staff login mounted beside it would be scenery here.
        staff_auth: vpay_config::StaffAuth::default(),
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

    vpay_db::Staff::create(
        repositories.as_ref(),
        vpay_db::NewStaff {
            id: STAFF_ID.to_owned(),
            merchant_id: MERCHANT_A.to_owned(),
            email: "dash-procs-reader@example.test".to_owned(),
            display_name: "Dash Procs Reader".to_owned(),
            // Never verified: nothing in this suite signs in.
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$notarealhash"
                .to_owned(),
            is_admin: false,
            now: time::OffsetDateTime::now_utc(),
        },
    )
    .await
    .context("seeding the staff member this suite's token names")?;

    Ok(Harness {
        _container: container,
        server: served.server,
        repositories,
        base_url: served.base_url,
        signing_key: served.signing_key,
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
/// merchants and asserting the *other* one's row is absent is what a
/// dropped `WHERE merchant_id = $1` (or a context built from
/// `persistence::system_context()`, which carries no tenant at all) would
/// fail. The equivalent mutation is `vpay-db`'s own
/// `against_postgres::the_page_is_the_tenants_own_rows_filtered_and_bounded`,
/// re-run for this lane and reported rather than duplicated as a second
/// container start here.
#[tokio::test]
async fn the_procedure_answers_the_callers_own_merchant_over_http() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();

    seed_intent(repositories, MERCHANT_A, "pi_procs_a_one").await?;
    seed_intent(repositories, MERCHANT_A, "pi_procs_a_two").await?;
    seed_intent(repositories, MERCHANT_B, "pi_procs_b_one").await?;

    let (status, body) = harness
        .search_payment_intents(Some(&harness.dashboard_token()))
        .await?;
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

/// Criterion 2 of Lane C's "done": an unauthenticated caller is refused —
/// `require_dashboard_procedure_token` answers `401` before the request
/// reaches CrateStack's own `@allow(auth() != null)` policy at all.
#[tokio::test]
async fn an_unauthenticated_caller_is_refused() -> anyhow::Result<()> {
    let harness = harness().await?;

    let (status, body) = harness.search_payment_intents(None).await?;
    assert_eq!(
        status, 401,
        "an unauthenticated request to the procedure transport must be refused: {body}"
    );
    Ok(())
}

/// One arbitrary request against the running server, for the route-shape
/// assertions below — `search_payment_intents` is the happy path's own
/// helper and hard-codes both the method and the body.
impl Harness {
    async fn request(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> anyhow::Result<(u16, Value)> {
        let client = reqwest::Client::builder()
            .build()
            .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed");
        let mut request = client
            .request(
                reqwest::Method::from_bytes(method.as_bytes()).expect("a known method"),
                format!("{}{path}", self.base_url),
            )
            .header("content-type", "application/json");
        if let Some(body) = body {
            request = request.body(serde_json::to_vec(&body)?);
        }
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("sending {method} {path}"))?;
        let status = response.status().as_u16();
        let text = response.text().await.context("reading the body")?;
        Ok((
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        ))
    }
}

/// **`/dash/v1`'s uniform-404 property, which Lane C must not have widened.**
///
/// `dash::routes()`'s own fallback is `vpay_api::not_found`, and
/// `require_dashboard_token` answers it for every `/dash/v1/...` path the
/// route table does not name — so an authenticated operator who mistypes a
/// path is told "no such route", in this crate's own error envelope, and
/// nothing about what else this deployment mounts.
///
/// **Decisive for the way Lane C mounts its transport.** The procedure
/// router is attached with `Router::nest_service(DASH_NEST, ..)`, and
/// `nest_service` registers a *catch-all* (`/dash/v1/{*rest}`, plus
/// `/dash/v1` and `/dash/v1/`) in the outer **path** router
/// (`axum-0.8.9/src/routing/path_router.rs:246-282`). A path router entry
/// beats the fallback router, so every previously-unmatched `/dash/v1/...`
/// path now reaches `require_dashboard_procedure_token` instead of
/// `dash::routes()`'s fallback — and that middleware refuses a non-`POST`
/// with `403` before it looks at the token at all. Without the
/// `.fallback(not_found)` this lane adds *outside* that layer, the two
/// assertions below are `403` and a CrateStack-shaped body.
#[tokio::test]
async fn an_unknown_dash_path_is_still_the_honest_404() -> anyhow::Result<()> {
    let harness = harness().await?;
    let token = harness.dashboard_token();

    // A GET, the shape `/dash/v1` actually serves, on a path it does not
    // mount. `403` here would be `require_dashboard_procedure_token`'s
    // method check answering for a route that has nothing to do with the
    // procedure transport.
    let (status, body) = harness
        .request("GET", "/dash/v1/not_a_dashboard_route", Some(&token), None)
        .await?;
    assert_eq!(
        status, 404,
        "an authenticated operator naming a path /dash/v1 does not mount must be told \
         'no such route', not refused by the procedure transport's method check: {body}"
    );
    assert_eq!(
        body.pointer("/error/type").and_then(Value::as_str),
        Some("invalid_request_error"),
        "the 404 must be this crate's own error envelope, not CrateStack's: {body}"
    );

    // And a POST on a path that is not a procedure: this one *does* reach
    // the transport, and must still be a 404 rather than anything that
    // distinguishes "no such procedure" from "no such route".
    let (status, body) = harness
        .request(
            "POST",
            "/dash/v1/$procs/noSuchProcedure",
            Some(&token),
            Some(serde_json::json!({})),
        )
        .await?;
    assert_eq!(
        status, 404,
        "a procedure this schema does not declare must be a 404: {body}"
    );
    Ok(())
}

/// **A malformed body is a refusal that says nothing about the tenant.**
///
/// Criterion 2's other half: `an_unauthenticated_caller_is_refused` proves
/// the credential check, and this proves that a caller who *has* a
/// credential and sends nonsense is told only that the body was
/// unacceptable — never whether the tenant, or any id inside the body,
/// exists.
#[tokio::test]
async fn a_malformed_body_is_refused_without_naming_the_tenant() -> anyhow::Result<()> {
    let harness = harness().await?;
    let token = harness.dashboard_token();

    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_procs_a_mal").await?;

    let (status, body) = harness
        .request(
            "POST",
            PROCS_PATH,
            Some(&token),
            // `page` is an object in the schema; a string is the cheapest
            // well-formed JSON the decoder must refuse.
            Some(serde_json::json!({ "page": "not-an-object", "filter": {} })),
        )
        .await?;
    // A 4xx, deliberately not pinned to one code: which 4xx CrateStack's
    // codec chooses for an undecodable argument is its decision, and pinning
    // it here would make a CrateStack patch release this suite's problem.
    // What must hold is that the request is *refused* — not served, and not
    // a 5xx that would say the process fell over on caller-supplied input.
    assert!(
        (400..500).contains(&status),
        "a malformed procedure body must be refused as a client error, not served and not a \
         500: {status} {body}"
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
