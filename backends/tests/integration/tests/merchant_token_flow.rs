//! The OP's token flow, and what `/v1` makes of its tokens, end to end: the
//! real `vpay_api::router` on a real socket, backed by a real Postgres.
//!
//! `/v1` is signed now (`docs/signed-transport.md`): a merchant seals every
//! call with its registered Ed25519 key, and no bearer token is a credential
//! there. The OP (`/v1/oauth/*`) still exists until VP-4 and still mints
//! tokens, and the cases below hold it to what it claims while it does:
//!
//! - **(a)** a sealed request reaches the honest 404, and the token the OP
//!   mints for the same merchant, presented as a bearer, gets the unsigned
//!   401; a plain request gets the layer's coarse 401 too.
//! - **(b)** `vpay_db::disable_client` takes effect on the *next* request, on
//!   both doors: `invalid_client`/401 at the token endpoint, the unsigned 401
//!   on a sealed `/v1` call.
//! - **(c)** no bearer reaches `/v1`, whatever its audience — including the
//!   one this OP mints when `audience` is left off.
//! - **(d)** `/v1/oauth/jwks.json` publishes exactly the `kid` this process
//!   signs with, and the discovery document names the endpoints this test
//!   derives from the base URL independently of the server.
//! - **(e)** a `client_assertion` presented twice is refused the second
//!   time — the proof that `vpay_db::client_assertion_store` is actually wired
//!   into the store the token handler consults.
//! - **(g)** a token request that omits the `client_id` form field — which
//!   RFC 7523 §3 permits — still receives its registration's default scope.
//! - **(h)** that default widens nothing: a narrower named scope wins, an
//!   unregistered one is `invalid_scope`, an omitted one yields the
//!   registration.
//! - **(i)** the three grants `/v1` does not serve are refused before any
//!   store is consulted.
//! - **(j)** a spent `jti` outlives the validator's leeway: an assertion
//!   whose `exp` is 30 s in the past is still accepted (`jsonwebtoken`'s 60 s
//!   default leeway, which `authkestra-op` 0.13.1 leaves alone), and its
//!   replay is still refused after the worker's housekeeping sweep has run
//!   ([ADR-0028](../../../../docs/adr/0028-a-spent-jti-outlives-the-validators-leeway.md)).
//! - **(k)** the other half of that bargain: an assertion older than the
//!   sweep's production horizon is refused by the validator.
//!
//! The client assertions are minted here with `jsonwebtoken`: the merchant
//! SDK seals requests and no longer has a token endpoint to call, so a suite
//! about the token endpoint cannot be driven through it.
//!
//! # What has actually been run
//!
//! Every test below **has been executed and passes through
//! [`migrated_postgres`] itself** — a real `postgres:16-alpine` container
//! started by `testcontainers`, the same path CI takes. The counts as of the
//! last run are in this crate's summary, not repeated here: a number quoted
//! in prose goes stale the moment a case is added, which is how a doc
//! comment starts overstating what was run.
//!
//! This paragraph previously said the container bootstrap was *not*
//! verified, because the Docker daemon on the machine this was written on
//! failed to start any container (`failed to start shim: unsupported
//! protocol: Yunix`) and the assertions had to be checked against an
//! already-running Postgres through a temporary scratch-database harness
//! instead. That is no longer the gap: with `DOCKER_HOST` pointed at the
//! rootless socket (`unix:///run/user/$UID/docker.sock`) containers start
//! normally and the unmodified helper below is what these tests ran on. The
//! record is kept rather than deleted because the failure mode was real and
//! whoever meets it next should recognise it as an environment problem, not
//! a test problem.
//!
//! They are deliberately **not** `#[ignore]`d: an ignored test is a test
//! that never goes green in CI either, and CI does have Docker.
//!
//! # Why the SDK and not a hand-rolled client
//!
//! `vpay-sdk` is the shipping merchant SDK (`sdks/rust`), the same artefact a
//! merchant integrates — not a test double. The alternative, hand-rolling a
//! token exchange here, would prove that vpay's OP agrees with a client
//! written by the same person on the same afternoon, which is the one thing
//! worth nothing. Where a raw request *is* used below it is because the SDK
//! deliberately cannot express it (a request with no bearer token; the same
//! assertion sent twice).
//!
//! The pool-and-migrate helper, the key generator, the crypto-provider
//! install and the `RouterDeps` assembly all live in `tests/support/mod.rs`
//! now. Step 2 is what tipped that balance: `RouterDeps` grew the adapter map
//! and the configuration projection, and a per-suite copy of *that* would let
//! one suite's router quietly stop resembling `vpay-server`'s.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use vpay_db::Repositories;

use anyhow::Context;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::op::MerchantOp;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_config::{Config, Deployment, MERCHANT_AUDIENCE};

mod support;

use support::sealed::SealedHttp;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client_with_scopes, migrated_postgres,
    router_deps,
};

const CLIENT_ID: &str = "acme-cameroon";

/// The tenant `CLIENT_ID` acts for — never equal to the `client_id`, so a
/// query filtered by the wrong one cannot pass.
const MERCHANT_ID: &str = "acme-cameroon-tenant";

/// The `aud` a `/dash/v1` token carries.
///
/// **A registered `dashboard_client.client_id` since ADR-0017**, not the
/// retired constant `vpay:dash/v1`: the authorization-code grant mints
/// `aud = <client_id>` and has no requested-audience path, so the validator
/// was changed to expect what the grant produces. Spelled here rather than
/// imported because there is no constant left to import — it is
/// configuration — and this suite registers no dashboard client, which is
/// exactly what makes the case below meaningful: the value is one this
/// deployment has never heard of, and `/v1` must refuse it anyway.
///
/// Case (c) exists precisely because these two strings must never be
/// interchangeable.
const DASHBOARD_AUDIENCE: &str = "vpay-dashboard";

/// A running vpay server and everything a test needs to talk to it.
///
/// The container is held (not `_`-bound) so it outlives the pool: dropping a
/// `ContainerAsync` stops the container, and a pool talking to a stopped
/// container fails in a way that looks like a vpay bug.
struct Harness {
    _container: ContainerAsync<PostgresImage>,
    /// The axum server, running on this test's own runtime. Aborted on drop
    /// via [`Harness::shutdown`]'s `JoinHandle`, so a failing assertion
    /// cannot leave a listener behind.
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    /// The harness database, for cases that read or stage a row the HTTP
    /// surface cannot show (the spent `jti`s of cases (j) and (k)).
    pool: PgPool,
    /// `http://127.0.0.1:{port}` — what a merchant would configure as their
    /// vpay base URL, and what every endpoint below is derived from by the
    /// *same* rules the SDK and `MerchantOp` each apply independently.
    base_url: String,
    /// The merchant's private key, PEM-encoded.
    merchant_pem: String,
    /// This server's own signing key, kept so case (c) can mint a token the
    /// server would consider validly signed but wrongly addressed.
    signing_key: LoadedSigningKey,
}

impl Harness {
    fn issuer(&self) -> String {
        format!("{}/v1/oauth", self.base_url)
    }

    fn token_endpoint(&self) -> String {
        format!("{}/token", self.issuer())
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

/// Boots a real server: migrated Postgres, a generated signing key announced
/// in `oauth_signing_keys`, one registered merchant, and
/// `vpay_api::router` on an ephemeral loopback port.
///
/// The ordering mirrors `vpay-server`'s `main` deliberately — bind first,
/// then derive the issuer and the validator's JWKS URL from the port that
/// was actually bound — because a harness that assembled things in a
/// different order would be testing a different program.
async fn harness() -> anyhow::Result<Harness> {
    harness_with_scopes(&[vpay_api::SCOPE_PAYMENTS_WRITE]).await
}

/// [`harness`] with the merchant's registered `scopes:` spelled out.
///
/// The registration is not decoration for cases (g) and (h): it *is* the
/// default scope the OP applies (`vpay_api::op::default_scopes`) and the set
/// a requested scope is checked against, so those two cases need a
/// registration with more than one scope in it — with a single one, "the
/// request's narrower scope won" and "the default was applied" produce
/// identical tokens and neither claim would be tested.
async fn harness_with_scopes(scopes: &[&str]) -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (container, repositories, pool) = migrated_postgres().await?;

    // Bind before building anything that needs to know the URL: the issuer,
    // the assertion audience the SDK signs, and the JWKS URL the validator
    // fetches all contain this port.
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .context("binding an ephemeral loopback port")?;
    let bound = listener.local_addr().context("reading the bound port")?;
    let base_url = format!("http://{bound}");
    let issuer = format!("{base_url}/v1/oauth");

    let (server_pem, _server_jwks) = generate_key();
    let signing_key =
        LoadedSigningKey::from_pem(&server_pem, &issuer).context("loading the signing key")?;
    signing_key
        .ensure_active_in_database(repositories.as_ref())
        .await
        .context("announcing the signing key in oauth_signing_keys")?;

    let (merchant_pem, merchant_jwks) = generate_key();
    let config = Config {
        deployment: Deployment {
            name: "merchant-token-flow".to_owned(),
            livemode: false,
            public_base_url: base_url.clone(),
            surfaces: None,
        },
        providers: Vec::new(),
        currencies: Vec::new(),
        // `support::merchant_client` sets `vpay:v1` as the allowed audience,
        // which is what both SDKs request by default and what
        // `Config::validate_all` requires a merchant to be able to target;
        // without it the token endpoint answers `invalid_target`.
        merchant_clients: vec![merchant_client_with_scopes(
            CLIENT_ID,
            MERCHANT_ID,
            merchant_jwks,
            scopes,
        )],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard: None,
        service_clients: Vec::new(),
        routing: Default::default(),
    };

    let merchant_op = Arc::new(MerchantOp::new(
        &config,
        signing_key.clone(),
        Arc::clone(&repositories),
    ));
    let deps = router_deps(
        Arc::clone(&repositories),
        merchant_op,
        // No `dashboard:` in either suite's configuration, so no management
        // nest is mounted at all — see `support::router_deps`.
        &config,
    );
    let server = tokio::spawn(async move {
        // A serve error here is not something a test can assert on
        // meaningfully — the assertions below fail on a refused connection
        // instead, which says more.
        let _ = axum::serve(listener, vpay_api::router(deps)).await;
    });

    Ok(Harness {
        _container: container,
        server,
        repositories,
        pool,
        base_url,
        merchant_pem,
        signing_key,
    })
}

/// A plain HTTP client for the requests the SDK deliberately cannot make.
fn raw_client() -> reqwest::Client {
    reqwest::Client::builder()
        .build()
        .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
}

/// The form body of one `client_credentials` + `private_key_jwt` token
/// request, built the same way `vpay_sdk::Client::fetch_token` builds it.
///
/// Spelled out here rather than reached through the SDK because case (e)
/// needs to send the *same* assertion twice, and the SDK — correctly —
/// mints a fresh one per request.
fn token_request_form(assertion: &str) -> Vec<(&'static str, String)> {
    vec![
        ("grant_type", "client_credentials".to_owned()),
        ("client_id", CLIENT_ID.to_owned()),
        (
            "client_assertion_type",
            "urn:ietf:params:oauth:client-assertion-type:jwt-bearer".to_owned(),
        ),
        ("client_assertion", assertion.to_owned()),
        ("audience", MERCHANT_AUDIENCE.to_owned()),
    ]
}

/// [`token_request_form`] with the `client_id` field removed — the request
/// RFC 7523 §3 explicitly permits, since the assertion's `sub` already names
/// the client.
///
/// Derived from `token_request_form` for the same reason
/// [`token_request_form_without_audience`] is: a case that failed because it
/// had also dropped `client_assertion_type` would prove nothing about
/// `client_id`.
fn token_request_form_without_client_id(assertion: &str) -> Vec<(&'static str, String)> {
    let mut form = token_request_form(assertion);
    form.retain(|(field, _)| *field != "client_id");
    assert_eq!(
        form.len(),
        4,
        "exactly one field must have been removed from the token request"
    );
    form
}

/// [`token_request_form`] with an explicit `scope` — the request a client
/// that wants *less* than its registration makes.
fn token_request_form_with_scope(assertion: &str, scope: &str) -> Vec<(&'static str, String)> {
    let mut form = token_request_form(assertion);
    form.push(("scope", scope.to_owned()));
    form
}

/// [`token_request_form`] with the `audience` field removed — the request a
/// client that has simply never heard of RFC 8707 resource indicators sends,
/// which both vpay SDKs avoid by always setting it.
///
/// Derived from `token_request_form` rather than written out again so the
/// two cannot drift on the four fields they share: a case (f) that failed
/// because it had also dropped `client_assertion_type` would prove nothing
/// about audiences.
fn token_request_form_without_audience(assertion: &str) -> Vec<(&'static str, String)> {
    let mut form = token_request_form(assertion);
    form.retain(|(field, _)| *field != "audience");
    assert_eq!(
        form.len(),
        4,
        "exactly one field must have been removed from the token request"
    );
    form
}

/// The `aud` claim of a token, read **without verifying anything**.
///
/// Deliberately not a `jsonwebtoken::decode` with
/// `insecure_disable_signature_validation()`: this reads what the OP put in
/// the token, and a decode call configured to skip checks is one edit away
/// from being read as a validation the test performed. The signature *is*
/// checked in this file — by the server, over HTTP, which is the only place
/// it counts.
fn unverified_aud(token: &str) -> String {
    unverified_claim(token, "aud").expect("every token this OP mints carries a string aud")
}

/// One string claim of a token, read the same way and with the same caveat
/// as [`unverified_aud`]. `None` when the claim is absent — which for
/// `scope` is a real and important outcome, not a failure: it is what a
/// token carries when no default was applied.
fn unverified_claim(token: &str, name: &str) -> Option<String> {
    let payload = token
        .split('.')
        .nth(1)
        .expect("a JWT has three dot-separated segments");
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .expect("the payload segment is base64url");
    let claims: Value = serde_json::from_slice(&bytes).expect("the payload is JSON");
    claims.get(name).and_then(Value::as_str).map(str::to_owned)
}

/// A token's `scope` claim as a *set*, which is what RFC 6749 §3.3 says a
/// scope is: a space-delimited list whose order carries no meaning. Asserted
/// as a set so these cases pin what was granted rather than the order
/// `default_scopes` happened to join a registration in.
fn granted_scopes(token: &str) -> Vec<String> {
    let mut scopes: Vec<String> = unverified_claim(token, "scope")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    scopes.sort();
    scopes
}

/// Posts a token request and returns its status and body.
async fn post_token(
    harness: &Harness,
    form: &[(&'static str, String)],
) -> anyhow::Result<(u16, Value)> {
    let response = raw_client()
        .post(harness.token_endpoint())
        .form(form)
        .send()
        .await
        .context("the token endpoint answers")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    Ok((status, body))
}

/// The `access_token` of a successful token response, or a failure quoting
/// the body that came back instead.
fn access_token(status: u16, body: &Value) -> anyhow::Result<String> {
    anyhow::ensure!(status == 200, "expected a token, got {status}: {body:#}");
    Ok(body
        .get("access_token")
        .and_then(Value::as_str)
        .context("the token response carries an access_token")?
        .to_owned())
}

// --------------------------------------------------------------- case (a)

/// The honest 404 behind the signature, and the credential kind that cannot
/// reach it.
///
/// A sealed request from the registered merchant reaches
/// `/v1/payment_intents/pi_x` and gets `404 resource_missing` — sealed, with
/// the code, because this merchant has no intent under that id. The access
/// token this very server's OP mints for the same merchant, presented as a
/// bearer on the same path, gets the layer's unsigned `401`. Both halves
/// matter: the first says the route exists and looked; the second says the
/// token the OP still issues until VP-4 retires it buys nothing on `/v1`.
#[tokio::test]
async fn a_sealed_request_reaches_the_honest_404_and_the_ops_token_does_not() -> anyhow::Result<()>
{
    let harness = harness().await?;
    let url = format!("{}/v1/payment_intents/pi_x", harness.base_url);

    let sealed = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_ID)
        .send()
        .await?;
    assert!(sealed.is_sealed(), "the answer to a sealed call is sealed");
    assert_eq!(sealed.status().as_u16(), 404);
    let body: Value = sealed.json().await.context("the sealed body is JSON")?;
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("resource_missing"),
        "got {body:#}"
    );

    let (status, token_body) =
        post_token(&harness, &token_request_form(&mint_assertion(&harness)?)).await?;
    let token = access_token(status, &token_body)?;
    let refused = raw_client()
        .get(&url)
        .header("accept", "application/json")
        .bearer_auth(&token)
        .send()
        .await
        .context("the server answers")?;
    assert_eq!(
        refused.status().as_u16(),
        401,
        "a bearer token is not a credential on /v1"
    );

    harness.shutdown().await;
    Ok(())
}

/// The other side of the boundary: the same path, nothing presented at all.
///
/// The layer's coarse `401` — `code: UNAUTHORIZED`, identical for every
/// cause and no longer the Stripe `authentication_error` envelope the retired
/// bearer middleware answered (`docs/reference/api.md`, "The unsigned answer"
/// states the change). A `404` here would mean the envelope is not in front
/// of the `/v1` nest's fallback.
#[tokio::test]
async fn a_v1_request_that_is_not_signed_is_the_layers_coarse_401() -> anyhow::Result<()> {
    let harness = harness().await?;

    let response = raw_client()
        .get(format!("{}/v1/payment_intents/pi_x", harness.base_url))
        .header("accept", "application/json")
        .send()
        .await
        .context("the server answers an unsigned /v1 request")?;

    assert_eq!(response.status().as_u16(), 401);
    let body: Value = response.json().await.context("the 401 body is JSON")?;
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("UNAUTHORIZED"),
        "got {body:#}"
    );
    assert!(
        body.pointer("/error/type").is_none(),
        "the unsigned answer is the layer's, not the Stripe envelope: {body:#}"
    );

    harness.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (b)

/// The kill switch, end to end and with no restart in between — on both
/// doors a disabled client could still use.
///
/// The same merchant that just had a sealed call answered and a token minted
/// is refused the moment a row lands in `disabled_clients` — ADR-0010's "an
/// operator flips a client to disabled and it takes effect immediately, no
/// deploy required":
///
/// - the token endpoint answers `invalid_client`/401 (the OP, which stays
///   until VP-4), and
/// - the sealed `/v1` call gets the unsigned `401` — the same coarse answer
///   as a key nobody registered, so a disabled client is indistinguishable
///   from an unknown one.
#[tokio::test]
async fn a_disabled_client_is_refused_with_invalid_client_and_401() -> anyhow::Result<()> {
    let harness = harness().await?;
    let url = format!("{}/v1/payment_intents/pi_x", harness.base_url);

    // Proof the client works *before* the switch is flipped, so a failure
    // after it cannot be blamed on a broken fixture.
    let before = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_ID)
        .send()
        .await?;
    assert!(before.is_sealed());
    assert_eq!(before.status().as_u16(), 404, "the honest 404 — case (a)");

    harness
        .repositories
        .disable_client(CLIENT_ID, Some("integration test"))
        .await
        .context("flipping the kill switch")?;

    let after = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_ID)
        .send()
        .await?;
    assert!(
        !after.is_sealed(),
        "a disabled client's call is refused unsigned"
    );
    assert_eq!(after.status().as_u16(), 401);

    let response = raw_client()
        .post(harness.token_endpoint())
        .form(&token_request_form(&mint_assertion(&harness)?))
        .send()
        .await
        .context("the token endpoint answers")?;
    assert_eq!(
        response.status().as_u16(),
        401,
        "invalid_client is the one token-endpoint error that answers 401"
    );
    let body: Value = response.json().await.context("the error body is JSON")?;
    assert_eq!(
        body.get("error").and_then(Value::as_str),
        Some("invalid_client"),
        "the token endpoint speaks RFC 6749, not the Stripe envelope: {body:#}"
    );

    harness.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (c)

/// No bearer reaches `/v1`, whatever it is addressed to.
///
/// Both tokens are minted directly through the server's own `TokenManager`,
/// so their signatures verify and their `kid` is in the JWKS — everything
/// about them is right. One is addressed to the dashboard surface, the other
/// to `vpay:v1`. Before the signed transport the audience was the whole
/// difference between a `401` and the honest `404`; now neither is a
/// credential at all and both are refused with the same unsigned answer,
/// which is what the "no token fallback" ruling means in a test.
#[tokio::test]
async fn a_bearer_token_of_any_audience_is_refused_on_v1() -> anyhow::Result<()> {
    let harness = harness().await?;

    for (label, audience, scope) in [
        ("dashboard", DASHBOARD_AUDIENCE, None),
        (
            "merchant",
            MERCHANT_AUDIENCE,
            Some(vpay_api::SCOPE_PAYMENTS_READ.to_owned()),
        ),
    ] {
        let token = harness
            .signing_key
            .token_manager()
            .issue_client_token_with_extra(
                CLIENT_ID,
                900,
                scope,
                Some(audience.to_owned()),
                HashMap::new(),
            )
            .context("minting a token with the server's own signer")?;

        let response = raw_client()
            .get(format!("{}/v1/payment_intents/pi_x", harness.base_url))
            .header("accept", "application/json")
            .bearer_auth(&token)
            .send()
            .await
            .context("the server answers")?;
        assert_eq!(
            response.status().as_u16(),
            401,
            "a {label}-audience bearer must not reach /v1"
        );
    }

    harness.shutdown().await;
    Ok(())
}

/// The token this OP really mints when `audience` is left off, and what
/// `/v1` makes of it.
///
/// Three assertions, in the order they matter:
///
/// 1. the token endpoint *accepts* the request (200) — the OP still issues
///    it, until VP-4;
/// 2. its `aud` is the `client_id`, read straight off the payload, so the
///    claim about what the OP mints is measured here rather than quoted from
///    upstream's source
///    (`authkestra-op-0.13.1/src/handlers/token.rs`: "No audience requested;
///    defaulting client_credentials token audience to client_id");
/// 3. `/v1` refuses it with the unsigned `401`.
#[tokio::test]
async fn a_token_minted_with_no_audience_is_addressed_to_the_client_and_refused_on_v1()
-> anyhow::Result<()> {
    let harness = harness().await?;

    let response = raw_client()
        .post(harness.token_endpoint())
        .form(&token_request_form_without_audience(&mint_assertion(
            &harness,
        )?))
        .send()
        .await
        .context("the token endpoint answers a request with no audience")?;
    assert_eq!(
        response.status().as_u16(),
        200,
        "a token request with no `audience` is accepted"
    );
    let body: Value = response.json().await.context("the token body is JSON")?;
    let token = body
        .get("access_token")
        .and_then(Value::as_str)
        .context("the token response carries an access_token")?;

    assert_eq!(
        unverified_aud(token),
        CLIENT_ID,
        "with no `audience` requested, this OP addresses the token to the client itself"
    );

    let response = raw_client()
        .get(format!("{}/v1/payment_intents/pi_x", harness.base_url))
        .header("accept", "application/json")
        .bearer_auth(token)
        .send()
        .await
        .context("the server answers")?;
    assert_eq!(response.status().as_u16(), 401);

    harness.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (d)

/// The two public documents, checked against the process that serves them.
///
/// `/jwks.json` must list exactly the `kid` this process signs with — not a
/// superset, and not a document that happens to parse. A second key would
/// mean `ensure_active_in_database` wrote a row it should not have; zero
/// keys would mean no token this deployment issues can be verified by
/// anyone.
///
/// The discovery document's `issuer` and `token_endpoint` are compared
/// against this **harness's own** derivation from the base URL
/// ([`Harness::issuer`], [`Harness::token_endpoint`]:
/// `{base_url}/v1/oauth` and `{issuer}/token`) — a literal spelled out here,
/// independently of both the server and the SDK, so that a change on either
/// side has to be made deliberately in three places rather than agreeing
/// with itself.
///
/// It is **case (a)**, not this case, that proves the SDK's own derivation
/// reaches the real endpoint: there the SDK is given nothing but a base URL
/// and a credential, and the token it comes back with is one the `/v1`
/// boundary accepts. This case only pins what the two public documents say.
#[tokio::test]
async fn the_jwks_and_discovery_documents_describe_this_process() -> anyhow::Result<()> {
    let harness = harness().await?;

    let jwks: Value = raw_client()
        .get(format!("{}/v1/oauth/jwks.json", harness.base_url))
        .send()
        .await
        .context("the JWKS endpoint answers")?
        .json()
        .await
        .context("the JWKS document is JSON")?;

    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .expect("a JWKS has a `keys` array");
    let published: Vec<&str> = keys
        .iter()
        .filter_map(|key| key.get("kid").and_then(Value::as_str))
        .collect();
    assert_eq!(
        published,
        vec![harness.signing_key.kid()],
        "exactly the active key, and nothing else: {jwks:#}"
    );

    let discovery: Value = raw_client()
        .get(format!(
            "{}/v1/oauth/.well-known/openid-configuration",
            harness.base_url
        ))
        .send()
        .await
        .context("the discovery endpoint answers")?
        .json()
        .await
        .context("the discovery document is JSON")?;

    assert_eq!(
        discovery.get("issuer").and_then(Value::as_str),
        Some(harness.issuer().as_str()),
        "got {discovery:#}"
    );
    assert_eq!(
        discovery.get("token_endpoint").and_then(Value::as_str),
        Some(harness.token_endpoint().as_str()),
        "the SDK signs its assertion with this URL as `aud`; a mismatch is a silent 401"
    );
    assert_eq!(
        discovery.get("jwks_uri").and_then(Value::as_str),
        Some(format!("{}/v1/oauth/jwks.json", harness.base_url).as_str()),
    );
    assert_eq!(
        discovery.get("grant_types_supported"),
        Some(&json!(["client_credentials"]))
    );
    assert_eq!(
        discovery.get("token_endpoint_auth_methods_supported"),
        Some(&json!(["private_key_jwt"]))
    );

    harness.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (e)

/// Replay protection, which is the whole reason `vpay_db::client_assertion_store`
/// exists and the only thing that proves it is wired into the store the
/// token handler consults.
///
/// One assertion, sent twice. The first exchange succeeds; the second is
/// refused with `invalid_client`/401 even though the assertion is still
/// inside its own lifetime and would verify perfectly on its own. Sent by
/// hand because the SDK mints a fresh assertion per request, which is
/// exactly the behaviour that makes this impossible to reach through it.
///
/// If `with_client_assertion_store` were dropped from `MerchantOp::new`,
/// `authkestra_op`'s `NoClientAssertionStore` would fail *closed* and the
/// **first** request would already be refused — so this test fails loudly in
/// both directions, rather than only catching a store that accepts
/// everything.
#[tokio::test]
async fn the_same_client_assertion_cannot_be_spent_twice() -> anyhow::Result<()> {
    let harness = harness().await?;

    let assertion = mint_assertion(&harness).context("minting one assertion to send twice")?;
    let form = token_request_form(&assertion);

    let first = raw_client()
        .post(harness.token_endpoint())
        .form(&form)
        .send()
        .await
        .context("the first token request answers")?;
    assert_eq!(
        first.status().as_u16(),
        200,
        "the first use of a fresh assertion must succeed; body: {}",
        first.text().await.unwrap_or_default()
    );

    let second = raw_client()
        .post(harness.token_endpoint())
        .form(&form)
        .send()
        .await
        .context("the second token request answers")?;
    assert_eq!(
        second.status().as_u16(),
        401,
        "a spent assertion must be refused"
    );
    let body: Value = second.json().await.context("the error body is JSON")?;
    assert_eq!(
        body.get("error").and_then(Value::as_str),
        Some("invalid_client"),
        "a replayed assertion is a client-authentication failure: {body:#}"
    );

    harness.shutdown().await;
    Ok(())
}
// --------------------------------------------------------------- case (g)

/// The default scope is found for a `private_key_jwt` request that omits
/// `client_id`, which RFC 7523 §3 explicitly permits.
///
/// `token_handler` keyed its RFC 6749 §3.3 default on the `client_id` **form
/// field** alone. `handle_token` does not need that field — it identifies
/// the client from the assertion — so such a request authenticated
/// perfectly, was handed no default, and came back with a token carrying no
/// `scope` claim at all. The merchant then got a `403` on every `/v1` call
/// while their registration plainly listed the scope they were refused for.
///
/// Only this file can show it: the claim is about what the OP *mints* for a
/// request it also authenticates, and the assertion has to be a real one
/// signed by the registered key. `vpay-api`'s own `op::token` tests
/// deliberately do not exercise the token endpoint at all (see their module
/// comment) — every path through it reads `disabled_clients` and spends a
/// `jti`.
///
/// The control at the end is what makes this a test of the *lookup* rather
/// than of a hardcoded string: the identical request *with* `client_id`
/// yields the same scopes, so the two paths agree.
///
/// Decisive: revert `default_scope_client_id` to `request.client_id` only
/// and the first assertion sees no `scope` claim.
#[tokio::test]
async fn a_token_request_with_no_client_id_still_gets_its_registered_default_scope()
-> anyhow::Result<()> {
    let harness = harness_with_scopes(&[
        vpay_api::SCOPE_PAYMENTS_WRITE,
        vpay_api::SCOPE_PAYMENTS_READ,
    ])
    .await?;

    let assertion = mint_assertion(&harness)
        .context("minting an assertion for a request that names no client_id")?;

    let (status, body) =
        post_token(&harness, &token_request_form_without_client_id(&assertion)).await?;
    let token = access_token(status, &body)?;
    assert_eq!(
        granted_scopes(&token),
        vec![
            vpay_api::SCOPE_PAYMENTS_READ.to_owned(),
            vpay_api::SCOPE_PAYMENTS_WRITE.to_owned()
        ],
        "a client identified only by its assertion must still get its registration's scopes; \
         without them every /v1 call is a 403 the merchant cannot explain"
    );

    // The control: the same request *with* `client_id` grants the same set.
    let assertion =
        mint_assertion(&harness).context("minting a second assertion (the first is spent)")?;
    let (status, body) = post_token(&harness, &token_request_form(&assertion)).await?;
    let with_client_id = access_token(status, &body)?;
    assert_eq!(
        granted_scopes(&with_client_id),
        granted_scopes(&token),
        "the two ways of naming the same client must grant the same scopes"
    );

    harness.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (h)

/// The default widens nothing: a narrower request wins, an unregistered
/// scope is refused, and an omitted one yields the registration.
///
/// `MerchantOp::default_scopes`' doc claims all three. Two of them were
/// claims about `authkestra_op`'s behaviour that nothing here measured, and
/// the third is the property that makes filling in an omitted `scope` safe
/// at all — so it is worth being unable to break silently. A change that
/// made the default *replace* a named scope, or that stopped the
/// registration check applying to a request the default touched, would leave
/// every other test in this file green.
///
/// The registration deliberately holds two scopes: with one, "the request's
/// narrower scope was honoured" and "the default was applied" mint identical
/// tokens.
#[tokio::test]
async fn a_named_scope_narrows_and_an_unregistered_one_is_refused() -> anyhow::Result<()> {
    let harness = harness_with_scopes(&[
        vpay_api::SCOPE_PAYMENTS_WRITE,
        vpay_api::SCOPE_PAYMENTS_READ,
    ])
    .await?;

    let mint = || mint_assertion(&harness);

    // 1. Narrower wins. The client is registered to write; it asks only to
    //    read, and gets only read — the default must not put `payments:write`
    //    back.
    let (status, body) = post_token(
        &harness,
        &token_request_form_with_scope(&mint()?, vpay_api::SCOPE_PAYMENTS_READ),
    )
    .await?;
    let narrowed = access_token(status, &body)?;
    assert_eq!(
        granted_scopes(&narrowed),
        vec![vpay_api::SCOPE_PAYMENTS_READ.to_owned()],
        "a client that asks for less must get exactly what it asked for"
    );

    // 2. An unregistered scope is `invalid_scope`, not silently dropped and
    //    not silently replaced by the default.
    let (status, body) = post_token(
        &harness,
        &token_request_form_with_scope(&mint()?, "refunds:write"),
    )
    .await?;
    assert_eq!(
        status, 400,
        "an unregistered scope is a request problem, not a credential one: {body:#}"
    );
    assert_eq!(
        body.get("error").and_then(Value::as_str),
        Some("invalid_scope"),
        "got {body:#}"
    );

    // 3. An omitted scope yields the registration — both of it.
    let (status, body) = post_token(&harness, &token_request_form(&mint()?)).await?;
    let defaulted = access_token(status, &body)?;
    assert_eq!(
        granted_scopes(&defaulted),
        vec![
            vpay_api::SCOPE_PAYMENTS_READ.to_owned(),
            vpay_api::SCOPE_PAYMENTS_WRITE.to_owned()
        ],
        "a request that names no scope is granted the registration, and nothing beyond it"
    );

    harness.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (i)

/// The three grants `/v1` does not serve are refused **before any store is
/// consulted** — `unauthorized_client` for each, never `server_error`.
///
/// This is the case that lets the three `OpStore` slots be fail-closed
/// stores (`vpay_api::op::refusing_stores`) rather than real Postgres ones.
/// Those stores return `Err` from all twelve of their methods; every one of
/// `authkestra_op`'s grant handlers turns *any* store error into
/// `server_error`. So "the stores are unreachable" and "a merchant is told
/// `server_error` on these grants" are the same question asked twice, and
/// this test is the answer that stops being true if the reasoning is wrong.
///
/// **The `error` code carries the proof, not the status** (corrected
/// 2026-09-05 by review; this comment used to say a store error would be a
/// 500). `vpay_api::op::token::token_error_status` answers *everything*
/// except `invalid_client` with a 400 — `server_error` included, which
/// `only_invalid_client_answers_401` asserts by name. This endpoint has no
/// 500 to emit, so the `assert_ne!(status, 500)` below is a cheap guard
/// against that mapping changing under this test, not the assertion that
/// makes the case decisive. The decisive pair is
/// `error == "unauthorized_client"` and `error != "server_error"`.
///
/// It is not asserting a property of vpay's own code: the refusal comes from
/// `default_handle_authorization_code`, `default_handle_refresh_token` and
/// `handle_device_code`, each of which checks
/// `client.allows_grant_type(..)` as its first statement, and a merchant
/// registration can only ever declare `client_credentials`
/// (`vpay_config::ConfigError::DisallowedMerchantGrant`). That is a claim
/// about a pinned dependency's control flow, which is exactly the kind of
/// claim that stops being true at a version bump with nothing noticing —
/// so it is measured here rather than argued in a comment.
///
/// Every field the grant's own handler would need is supplied (`code`,
/// `redirect_uri`, `refresh_token`, `device_code`), so a refusal cannot be
/// "you left out a parameter": `default_handle_refresh_token` and
/// `handle_device_code` both answer `invalid_request` for a missing one,
/// which would pass a "the error is not `server_error`" assertion while
/// proving nothing about the grant check. The credential is a real, freshly
/// minted assertion for a registered client — an unauthenticated request is
/// refused at `authenticate_client`, upstream of the dispatch this case is
/// about.
///
/// Decisive, and measured on 2026-09-05 rather than argued: point one slot
/// at a store that answers `Ok(None)` instead of refusing and this test
/// still passes (it did — nothing reaches it), while
/// `vpay_api::op::refusing_stores`' own
/// `every_method_of_every_slot_refuses` fails. Delete the
/// `allows_grant_type` check inside `authkestra-op` and this one fails with
/// `server_error`, which is the failure it exists to catch.
#[tokio::test]
async fn the_three_grants_vpay_does_not_serve_are_refused_before_any_store() -> anyhow::Result<()> {
    let harness = harness().await?;

    for grant_type in [
        "authorization_code",
        "refresh_token",
        "urn:ietf:params:oauth:grant-type:device_code",
    ] {
        // A fresh assertion per grant: `jti`s are single-use (case (e)), so
        // reusing one would make the second and third iterations
        // `invalid_client` and this loop would test replay protection three
        // times instead of the grant check once each.
        let assertion =
            mint_assertion(&harness).context("minting an assertion for an unserved grant")?;

        let mut form = token_request_form(&assertion);
        form.retain(|(field, _)| *field != "grant_type");
        form.push(("grant_type", grant_type.to_owned()));
        form.push(("code", "code-that-was-never-issued".to_owned()));
        form.push((
            "redirect_uri",
            "https://merchant.example/callback".to_owned(),
        ));
        form.push(("refresh_token", "token-that-was-never-issued".to_owned()));
        form.push((
            "device_code",
            "device-code-that-was-never-issued".to_owned(),
        ));

        let (status, body) = post_token(&harness, &form).await?;

        assert_ne!(
            status, 500,
            "grant_type={grant_type}: this endpoint maps every error but \
             `invalid_client` to 400, so a 500 means the status mapping \
             changed under this test: {body:#}"
        );
        assert_ne!(
            body.get("error").and_then(Value::as_str),
            Some("server_error"),
            "grant_type={grant_type} reached a store — every authkestra grant \
             handler renders any store error as `server_error`: {body:#}"
        );
        assert_eq!(
            status, 400,
            "grant_type={grant_type} must be a request-level refusal: {body:#}"
        );
        assert_eq!(
            body.get("error").and_then(Value::as_str),
            Some("unauthorized_client"),
            "grant_type={grant_type}: the registration allows client_credentials only, \
             so the grant check refuses before any store is consulted: {body:#}"
        );
    }

    // The control: the same harness still mints a token for the grant `/v1`
    // does serve. Without it, a harness that refused *everything* — a broken
    // registration, a mis-signed assertion — would pass the loop above.
    let assertion = mint_assertion(&harness)
        .context("minting an assertion for the one grant /v1 does serve")?;
    let (status, body) = post_token(&harness, &token_request_form(&assertion)).await?;
    access_token(status, &body)?;

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------- cases (j) and (k)

/// A client assertion signed by hand with `exp` set `exp_offset_secs` from the
/// API's `now` (negative: already past).
///
/// Hand-signed, with the claims of a real assertion (`iss = sub = client_id`,
/// the token endpoint as `aud`, a UUID
/// `jti`, no `kid`), so the only thing that differs from a real assertion is
/// the one claim under test. The API under test runs in this process, so
/// "now" here is the API's clock.
fn assertion_with_exp(harness: &Harness, exp_offset_secs: i64) -> anyhow::Result<(String, String)> {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let jti = uuid::Uuid::new_v4().to_string();
    let claims = json!({
        "iss": CLIENT_ID,
        "sub": CLIENT_ID,
        "aud": harness.token_endpoint(),
        "jti": jti,
        "iat": now + exp_offset_secs - 60,
        "exp": now + exp_offset_secs,
    });
    let assertion = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        &jsonwebtoken::EncodingKey::from_rsa_pem(harness.merchant_pem.as_bytes())
            .context("the merchant PEM is an RSA key")?,
    )
    .context("signing a hand-built assertion")?;
    Ok((assertion, jti))
}

/// A fresh client assertion, valid for the next minute: what a merchant's
/// client presents at the token endpoint.
///
/// Signed here with `jsonwebtoken` ([`assertion_with_exp`]) now that the
/// merchant SDK no longer mints one — it seals requests instead and has no
/// token endpoint to call. The OP stays until VP-4, and this file is what
/// keeps it honest until then.
fn mint_assertion(harness: &Harness) -> anyhow::Result<String> {
    assertion_with_exp(harness, 60).map(|(assertion, _jti)| assertion)
}

/// Runs the shipping housekeeping job, `sweep_expired`, once — through
/// `vpay_worker::run_once` over the shipping `seed_singletons`, the way
/// `checkout_sessions.rs`' claim 10 does, because what is being claimed is
/// that a *deployment* keeps a spent `jti`, not that a statement does.
///
/// No rails and no webhook endpoints: the sweep needs neither, and a harness
/// with an empty registry cannot deliver anything by accident. The other
/// singletons seeded alongside it may be claimed first; the loop runs them
/// until the sweep has run, and refuses a sweep that failed.
async fn run_the_housekeeping_sweep(repositories: &dyn Repositories) -> anyhow::Result<()> {
    vpay_worker::seed_singletons(repositories)
        .await
        .context("seeding the singleton jobs a worker seeds at boot")?;

    let adapters = vpay_worker::Adapters::new();
    let rails = vpay_worker::RailConfigs::new();
    let endpoints = support::no_webhook_endpoints();
    let egress = support::default_egress_policy();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let claimed = vpay_worker::run_once(
            repositories,
            &adapters,
            &rails,
            &vpay_worker::RecoveryPolicy::default(),
            &vpay_worker::WebhookContext {
                endpoints: &endpoints,
                egress,
            },
            &vpay_worker::CallbackPlan::none(),
            "merchant-token-flow",
        )
        .await
        .context("running a worker job")?;
        match claimed {
            Some(settled) if settled.kind == "sweep_expired" => {
                anyhow::ensure!(
                    settled.error.is_none(),
                    "the housekeeping sweep must not fail: {:?}",
                    settled.error
                );
                return Ok(());
            }
            Some(_) => {}
            None => tokio::time::sleep(Duration::from_millis(20)).await,
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "`sweep_expired` never ran within 10s"
        );
    }
}

/// Case (j): the replay of an assertion that is past its `exp` but inside the
/// validator's leeway is refused, **after** the housekeeping sweep has run.
///
/// `authkestra-op` 0.13.1 leaves `jsonwebtoken`'s `leeway` at 60 s, so an
/// assertion with `exp` 30 s ago verifies. It is spent once. Until ADR-0028
/// the hourly sweep deleted `expires_at < now()`, so a sweep landing between
/// `exp` and `exp + 61 s` deleted the row for an assertion the API still
/// accepted, and the next presentation was a second access token from one
/// captured assertion. The sweep here is the shipping one, with the shipping
/// horizon, and the replay is over HTTP.
///
/// A control row, long past its `exp`, is staged first so the sweep is shown
/// to have run its `jti` statement: without it "the row survived" would also
/// be true of a sweep that never touched the table.
///
/// Decisive: set `CLIENT_ASSERTION_JTI_RETENTION` to `Duration::ZERO` (the
/// pre-ADR-0028 behaviour) and the replay is accepted with a `200`.
#[tokio::test]
async fn a_spent_assertion_past_its_exp_but_inside_the_leeway_cannot_be_replayed_after_a_sweep()
-> anyhow::Result<()> {
    let harness = harness().await?;

    let (assertion, jti) = assertion_with_exp(&harness, -30)?;
    let form = token_request_form(&assertion);

    let first = raw_client()
        .post(harness.token_endpoint())
        .form(&form)
        .send()
        .await
        .context("the first token request answers")?;
    assert_eq!(
        first.status().as_u16(),
        200,
        "an assertion 30 s past its exp is inside jsonwebtoken's 60 s leeway and must verify \
         — if this is a 401 the upstream leeway shrank and this case needs rethinking; body: {}",
        first.text().await.unwrap_or_default()
    );

    sqlx::query(
        "INSERT INTO oauth_client_assertion_jtis (jti, expires_at) \
         VALUES ('control-long-expired', now() - INTERVAL '1 day')",
    )
    .execute(&harness.pool)
    .await
    .context("staging a control row the sweep must delete")?;

    run_the_housekeeping_sweep(harness.repositories.as_ref()).await?;

    let control: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM oauth_client_assertion_jtis WHERE jti = 'control-long-expired'",
    )
    .fetch_one(&harness.pool)
    .await
    .context("reading the control row back")?;
    assert_eq!(
        control, 0,
        "the sweep must have run its jti statement, or the next assertion proves nothing"
    );
    let replay = raw_client()
        .post(harness.token_endpoint())
        .form(&form)
        .send()
        .await
        .context("the replay answers")?;
    assert_eq!(
        replay.status().as_u16(),
        401,
        "a spent assertion must stay refused after a sweep, for as long as the validator would \
         accept it"
    );
    let body: Value = replay.json().await.context("the error body is JSON")?;
    assert_eq!(
        body.get("error").and_then(Value::as_str),
        Some("invalid_client"),
        "a replayed assertion is a client-authentication failure: {body:#}"
    );

    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM oauth_client_assertion_jtis WHERE jti = $1")
            .bind(&jti)
            .fetch_one(&harness.pool)
            .await
            .context("reading the spent jti back")?;
    assert_eq!(
        kept, 1,
        "the spent jti of an assertion the validator still accepts must survive the sweep"
    );

    harness.shutdown().await;
    Ok(())
}

/// Case (k): an assertion whose `exp` is older than the sweep's production
/// horizon is refused by the validator. This is the invariant that makes the
/// horizon sufficient.
///
/// The sweep may delete a row once `expires_at < now() - horizon`. That is
/// safe only if no assertion with such an `exp` can verify any more — i.e. the
/// validator's leeway is no larger than the horizon. The leeway is
/// `authkestra-op`'s, pinned at `=0.13.1`, and not vpay's to set. If a bump
/// widens it (or `jsonwebtoken` changes its default), the first assertion
/// below is accepted, this case fails, and the horizon in
/// `vpay_worker::CLIENT_ASSERTION_JTI_RETENTION` has to grow with it before
/// the bump merges. The horizon is read from that constant, never copied.
///
/// A control with the same hand-built shape and an `exp` 30 s past is
/// accepted first, so the refusal below cannot be about anything but age
/// (a malformed assertion would be refused too, and prove nothing). The
/// refused assertion's `jti` is checked unrecorded: it was refused before the
/// store was consulted.
#[tokio::test]
async fn an_assertion_older_than_the_sweep_horizon_is_refused_by_the_validator()
-> anyhow::Result<()> {
    let harness = harness().await?;

    let (control, _) = assertion_with_exp(&harness, -30)?;
    let accepted = raw_client()
        .post(harness.token_endpoint())
        .form(&token_request_form(&control))
        .send()
        .await
        .context("the control request answers")?;
    assert_eq!(
        accepted.status().as_u16(),
        200,
        "the control (exp 30 s past) must verify, or this case cannot tell age from malformation; \
         body: {}",
        accepted.text().await.unwrap_or_default()
    );

    let horizon_secs = i64::try_from(vpay_worker::CLIENT_ASSERTION_JTI_RETENTION.as_secs())
        .context("the horizon fits i64")?;
    // One second past the horizon: a row for this assertion is exactly what
    // the sweep is allowed to delete.
    let (stale, stale_jti) = assertion_with_exp(&harness, -horizon_secs - 1)?;
    let refused = raw_client()
        .post(harness.token_endpoint())
        .form(&token_request_form(&stale))
        .send()
        .await
        .context("the stale request answers")?;
    assert_eq!(
        refused.status().as_u16(),
        401,
        "an assertion older than CLIENT_ASSERTION_JTI_RETENTION verified: the validator's leeway \
         now exceeds the horizon, so the sweep can delete the jti of an assertion that is still \
         accepted (ADR-0028). Raise the horizon before taking this dependency bump."
    );
    let body: Value = refused.json().await.context("the error body is JSON")?;
    assert_eq!(
        body.get("error").and_then(Value::as_str),
        Some("invalid_client"),
        "got {body:#}"
    );

    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM oauth_client_assertion_jtis WHERE jti = $1")
            .bind(&stale_jti)
            .fetch_one(&harness.pool)
            .await
            .context("looking for the refused jti")?;
    assert_eq!(
        recorded, 0,
        "a refused assertion must be refused before its jti is spent"
    );

    harness.shutdown().await;
    Ok(())
}
