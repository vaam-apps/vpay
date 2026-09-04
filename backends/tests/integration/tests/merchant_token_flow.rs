//! The merchant token flow end to end: the real `vpay_api::router` on a real
//! socket, backed by a real Postgres, driven by the real merchant SDK.
//!
//! This is the only place the whole of "Step 1: merchant tokens on `/v1`" is
//! proved as one thing. Every piece has unit tests of its own —
//! `op::clients` for the registry, `op::keys` for the signing key,
//! `op::token` for the discovery document, `lib.rs` for the route tree — but
//! each of those substitutes something: a lazy pool, a hand-built
//! registration, a request that never presents a token. The claims that only
//! this file can make are the ones that span all of them:
//!
//! - **(a)** the SDK, configured exactly as a merchant would configure it,
//!   obtains a token and reaches an authenticated `/v1` route — which
//!   answers the honest 404 for an id no intent exists under; and the same
//!   route with no bearer token answers the 401 envelope.
//! - **(b)** `vpay_db::disable_client` takes effect on the *next token
//!   request*, with no restart: `invalid_client`, HTTP 401.
//! - **(c)** a token this server itself minted, but for the dashboard
//!   audience, is refused on `/v1` — so the audience separation between the
//!   two surfaces is real and not merely configured.
//! - **(d)** `/v1/oauth/jwks.json` publishes exactly the `kid` this process
//!   signs with, and the discovery document names the endpoints this test
//!   derives from the base URL independently of the server (that the *SDK*
//!   derives the same ones is what case (a) shows, by using them).
//! - **(e)** a `client_assertion` presented twice is refused the second
//!   time — the proof that `SqlClientAssertionStore` is actually wired into
//!   the store the token handler consults, which no unit test can show
//!   because the wiring is the thing under test.
//! - **(f)** a token request that omits `audience` is *accepted* by the
//!   token endpoint and comes back addressed to the merchant's own
//!   `client_id` — and `/v1` refuses that token. Only this file can show
//!   it: the claim is about what `authkestra_op` actually mints, so a unit
//!   test would have to assert it against a token the test itself
//!   constructed, which proves nothing about the OP.
//! - **(g)** a token request that omits the `client_id` form field — which
//!   RFC 7523 §3 permits, since the assertion's `sub` names the client —
//!   still receives its registration's default scope, and the token it gets
//!   authorises a `/v1` call.
//! - **(h)** that default widens nothing: a request naming a *narrower*
//!   scope gets exactly that, one naming a scope the registration does not
//!   hold is `invalid_scope`, and one naming none gets the registration.
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
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::op::MerchantOp;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_api::resource_auth::{JwtValidator, MerchantJwtValidator, Surface};
use vpay_config::{Config, Deployment, MERCHANT_AUDIENCE};
use vpay_sdk::Credentials;

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client_with_scopes, migrated_postgres,
    router_deps,
};

const CLIENT_ID: &str = "acme-cameroon";

/// The tenant `CLIENT_ID` acts for — never equal to the `client_id`, so a
/// query filtered by the wrong one cannot pass.
const MERCHANT_ID: &str = "acme-cameroon-tenant";

/// The `aud` a `/dash/v1` token would carry —
/// `vpay_api::resource_auth::Surface::Dashboard::audience()`, which is not
/// `pub`-reachable as a value, so it is spelled here. Case (c) exists
/// precisely because these two strings must never be interchangeable.
const DASHBOARD_AUDIENCE: &str = "vpay:dash/v1";

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

    fn credentials(&self) -> Credentials {
        Credentials::rsa_pem(CLIENT_ID, &self.merchant_pem)
            .expect("the generated PEM is a parseable RSA key")
    }

    /// An SDK client configured exactly as `docs/flows/merchant-auth.md`
    /// tells a merchant to configure one: a base URL and a credential.
    /// Everything else — the issuer, the token endpoint, the `vpay:v1`
    /// audience — the SDK derives on its own, which is what makes case (a)
    /// a test of that derivation and not of a URL the test handed it.
    fn sdk_client(&self) -> vpay_sdk::Client {
        vpay_sdk::Client::builder(&self.base_url)
            .credentials(self.credentials())
            .build()
            .expect("the SDK client builds from a base URL and a credential")
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

    let (container, repositories, _pool) = migrated_postgres().await?;

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
        dashboard_client: None,
    };

    let merchant_op = Arc::new(MerchantOp::new(
        &config,
        signing_key.clone(),
        Arc::clone(&repositories),
    ));
    let merchant_validator = MerchantJwtValidator(
        JwtValidator::new(
            format!("http://{bound}/v1/oauth/jwks.json"),
            Duration::from_secs(300),
            merchant_op.issuer(),
            Surface::Merchant,
        )
        .expect("the vendored-roots JWKS client builds"),
    );

    let deps = router_deps(
        Arc::clone(&repositories),
        merchant_op,
        merchant_validator,
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

/// The whole happy path, and the honest answer at the end of it.
///
/// The SDK obtains a token (proving: assertion minted, client resolved from
/// YAML, `jti` recorded, token signed by the key announced in
/// `oauth_signing_keys`) and reaches `/v1/payment_intents/pi_x` — which
/// answers `404 resource_missing`, because this merchant has no intent under
/// that id. That 404 **is** the assertion: the route is behind the
/// authentication boundary and the query is filtered by the tenant the
/// middleware resolved, so getting *this* 404 proves the token was minted,
/// fetched back over loopback JWKS, validated, and mapped to a merchant.
///
/// The code matters, not just the status. Before Step 2 this answered
/// `unknown_route` — the nest's fallback, from a route that did not exist.
/// `resource_missing` is what says the route now exists and looked.
/// `backends/tests/integration/tests/payment_intents.rs` is where the
/// resource itself is proved; this file's claim is only that a token reaches
/// it.
#[tokio::test]
async fn an_sdk_client_authenticates_and_reaches_the_honest_404() -> anyhow::Result<()> {
    let harness = harness().await?;

    let error = harness
        .sdk_client()
        .payment_intents()
        .retrieve("pi_x")
        .await
        .expect_err("no intent exists under `pi_x`; a 200 would be fabricated");

    match error {
        vpay_sdk::Error::Api {
            status,
            ref kind,
            ref code,
            ..
        } => {
            assert_eq!(status, 404, "authenticated, then no such object: {error:?}");
            assert_eq!(kind, "invalid_request_error");
            assert_eq!(code.as_deref(), Some("resource_missing"));
        }
        other => panic!(
            "expected the vpay 404 envelope after a successful token exchange, got {other:?}"
        ),
    }

    harness.shutdown().await;
    Ok(())
}

/// The other side of the boundary: the same path, no bearer token, over a
/// raw client the SDK cannot impersonate (it always attaches a token).
///
/// 401 with `authentication_error`/`missing_bearer_token`, not 404 — a 404
/// here would mean the authentication layer is not in front of the `/v1`
/// nest's fallback, which is the exact hole `Router::route_layer` would
/// have left.
#[tokio::test]
async fn a_v1_request_with_no_bearer_token_is_the_401_envelope() -> anyhow::Result<()> {
    let harness = harness().await?;

    let response = raw_client()
        .get(format!("{}/v1/payment_intents/pi_x", harness.base_url))
        .send()
        .await
        .context("the server answers an unauthenticated /v1 request")?;

    assert_eq!(response.status().as_u16(), 401);
    let body: Value = response.json().await.context("the 401 body is JSON")?;
    assert_eq!(
        body.pointer("/error/type").and_then(Value::as_str),
        Some("authentication_error"),
        "got {body:#}"
    );
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("missing_bearer_token"),
        "got {body:#}"
    );

    harness.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (b)

/// The kill switch, end to end and with no restart in between.
///
/// The same client that just obtained a token is refused the moment a row
/// lands in `disabled_clients` — ADR-0010's "an operator flips a client to
/// disabled and it takes effect immediately, no deploy required".
///
/// A *fresh* SDK client is built for the second attempt on purpose: the
/// first client caches its token until expiry (by design —
/// `docs/flows/merchant-auth.md` §3), so reusing it would prove nothing
/// about the token endpoint. That the already-issued token stays valid for
/// its remaining TTL is a real, documented property of a stateless bearer
/// token and not something this test papers over.
///
/// Both the SDK's typed error and the raw HTTP status are asserted: the
/// status is what `sdks/nodejs` and any third-party OAuth client branch on,
/// and it is what `crate::op::token`'s status mapping exists to produce.
#[tokio::test]
async fn a_disabled_client_is_refused_with_invalid_client_and_401() -> anyhow::Result<()> {
    let harness = harness().await?;

    // Proof the client works *before* the switch is flipped, so a failure
    // after it cannot be blamed on a broken fixture.
    harness
        .sdk_client()
        .payment_intents()
        .retrieve("pi_x")
        .await
        .expect_err("the honest 404 for a missing id — see the case (a) test");

    harness
        .repositories
        .disable_client(CLIENT_ID, Some("integration test"))
        .await
        .context("flipping the kill switch")?;

    let error = harness
        .sdk_client()
        .payment_intents()
        .retrieve("pi_x")
        .await
        .expect_err("a disabled client must not obtain a token");
    match error {
        vpay_sdk::Error::TokenEndpoint { ref error, .. } => {
            assert_eq!(
                error, "invalid_client",
                "a disabled client must be indistinguishable from an unknown one: {error:?}"
            );
        }
        other => panic!("expected an RFC 6749 token-endpoint error, got {other:?}"),
    }

    // The status, separately: the SDK's own error decoding would report
    // `TokenEndpoint` for a 400 just as happily as for a 401.
    let assertion = vpay_sdk::auth::mint_client_assertion(
        &harness.credentials(),
        &harness.token_endpoint(),
        Duration::from_secs(60),
    )
    .context("minting an assertion by hand")?;
    let response = raw_client()
        .post(harness.token_endpoint())
        .form(&token_request_form(&assertion))
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

/// Audience separation, proved with a token this very server signed.
///
/// The token is minted directly through the server's own `TokenManager`, so
/// its signature verifies and its `kid` is in the JWKS — everything about it
/// is right except the `aud`, which names the dashboard surface. `/v1` must
/// still refuse it.
///
/// This is the test that would fail if `JwtValidator` were built without
/// `set_audience`, or with both audiences, or if `Surface` were ever
/// collapsed to one value. It cannot be written with the SDK, which requests
/// `vpay:v1` and would in any case be refused `invalid_target` by the token
/// endpoint for asking for an audience this merchant is not registered for
/// — a *different* mechanism, and one that a resource server must not rely
/// on.
#[tokio::test]
async fn a_dashboard_audience_token_is_refused_on_v1() -> anyhow::Result<()> {
    let harness = harness().await?;

    let dashboard_token = harness
        .signing_key
        .token_manager()
        .issue_client_token_with_extra(
            CLIENT_ID,
            900,
            None,
            Some(DASHBOARD_AUDIENCE.to_owned()),
            HashMap::new(),
        )
        .context("minting a dashboard-audience token with the server's own signer")?;

    let response = raw_client()
        .get(format!("{}/v1/payment_intents/pi_x", harness.base_url))
        .bearer_auth(&dashboard_token)
        .send()
        .await
        .context("the server answers")?;

    assert_eq!(
        response.status().as_u16(),
        401,
        "a correctly signed token for the wrong surface must not reach /v1"
    );
    let body: Value = response.json().await.context("the 401 body is JSON")?;
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("invalid_token"),
        "got {body:#}"
    );

    // And the control: the same signer, the same TTL, the merchant
    // audience — accepted, reaching the honest 404. Without this, the
    // assertion above would also pass if `/v1` refused *every* token.
    let merchant_token = harness
        .signing_key
        .token_manager()
        .issue_client_token_with_extra(
            CLIENT_ID,
            900,
            // The scope has to be named because this token is minted
            // directly rather than obtained from the OP, which is where the
            // registration's scopes are applied to a request that asks for
            // none. Without it the control would be a `403` about
            // authorisation, and this test's whole claim is that the *only*
            // difference between the two tokens is the audience.
            Some(vpay_api::SCOPE_PAYMENTS_READ.to_owned()),
            Some(MERCHANT_AUDIENCE.to_owned()),
            HashMap::new(),
        )
        .context("minting a merchant-audience token with the server's own signer")?;
    let response = raw_client()
        .get(format!("{}/v1/payment_intents/pi_x", harness.base_url))
        .bearer_auth(&merchant_token)
        .send()
        .await
        .context("the server answers")?;
    assert_eq!(
        response.status().as_u16(),
        404,
        "the only difference from the refused token above is its `aud`"
    );

    harness.shutdown().await;
    Ok(())
}

/// The audience-confusion token this OP really mints, and the boundary
/// refusing it.
///
/// Case (c) proves the boundary refuses a *dashboard*-audience token, which
/// is a token nothing in this deployment produces by accident. This case is
/// the one an integrator produces by accident: leave `audience` off the
/// token request and `authkestra_op` mints a token whose `aud` is the
/// client's own `client_id` (`authkestra-op-0.7.1/src/handlers/token.rs`:
/// "No audience requested; defaulting client_credentials token audience to
/// client_id"). It is signed by this server's active key, carries the right
/// issuer and is inside its lifetime — everything about it is valid except
/// that it is not addressed to `/v1`.
///
/// Three assertions, in the order they matter:
///
/// 1. the token endpoint *accepts* the request (200) — the confusion is not
///    prevented upstream, and a test that assumed it was would be asserting
///    the wrong thing;
/// 2. its `aud` is the `client_id`, read straight off the payload, so the
///    claim about what the OP mints is measured here rather than quoted from
///    upstream's source;
/// 3. `/v1` refuses it with the 401 envelope.
///
/// Decisive: widening `JwtValidator::new`'s `set_audience` — the shape this
/// finding is about — makes step 3 answer 404 instead. The same property is
/// pinned without Docker by
/// `vpay_api::resource_auth`'s
/// `a_token_whose_audience_is_the_client_id_is_refused_on_the_merchant_surface`,
/// whose own doc comment records why *deleting* `set_audience` is not the
/// decisive mutation for it.
#[tokio::test]
async fn a_token_minted_with_no_audience_is_addressed_to_the_client_and_refused_on_v1()
-> anyhow::Result<()> {
    let harness = harness().await?;

    let assertion = vpay_sdk::auth::mint_client_assertion(
        &harness.credentials(),
        &harness.token_endpoint(),
        Duration::from_secs(60),
    )
    .context("minting an assertion for a token request with no audience")?;

    let response = raw_client()
        .post(harness.token_endpoint())
        .form(&token_request_form_without_audience(&assertion))
        .send()
        .await
        .context("the token endpoint answers a request with no audience")?;
    assert_eq!(
        response.status().as_u16(),
        200,
        "a token request with no `audience` is accepted, which is why the resource server has to \
         be the thing that refuses the token"
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
        .bearer_auth(token)
        .send()
        .await
        .context("the server answers")?;
    assert_eq!(
        response.status().as_u16(),
        401,
        "a token addressed to the client rather than to vpay:v1 must not reach /v1"
    );
    let body: Value = response.json().await.context("the 401 body is JSON")?;
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("invalid_token"),
        "got {body:#}"
    );

    // The control, same as case (c)'s: the identical request *with* the
    // audience reaches the honest 404, so the 401 above is about the `aud`
    // claim and not about anything else in the exchange.
    let assertion = vpay_sdk::auth::mint_client_assertion(
        &harness.credentials(),
        &harness.token_endpoint(),
        Duration::from_secs(60),
    )
    .context("minting a second assertion (the first is spent)")?;
    let body: Value = raw_client()
        .post(harness.token_endpoint())
        .form(&token_request_form(&assertion))
        .send()
        .await
        .context("the token endpoint answers")?
        .json()
        .await
        .context("the token body is JSON")?;
    let token = body
        .get("access_token")
        .and_then(Value::as_str)
        .context("the token response carries an access_token")?;
    assert_eq!(unverified_aud(token), MERCHANT_AUDIENCE);

    let response = raw_client()
        .get(format!("{}/v1/payment_intents/pi_x", harness.base_url))
        .bearer_auth(token)
        .send()
        .await
        .context("the server answers")?;
    assert_eq!(
        response.status().as_u16(),
        404,
        "the only difference from the refused token above is the `audience` form field"
    );

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

/// Replay protection, which is the whole reason `SqlClientAssertionStore`
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

    let assertion = vpay_sdk::auth::mint_client_assertion(
        &harness.credentials(),
        &harness.token_endpoint(),
        Duration::from_secs(60),
    )
    .context("minting one assertion to send twice")?;
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

    let assertion = vpay_sdk::auth::mint_client_assertion(
        &harness.credentials(),
        &harness.token_endpoint(),
        Duration::from_secs(60),
    )
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

    // And the token works, which is the thing the merchant actually
    // noticed: a 404 for a missing id rather than a 403 about scope.
    let response = raw_client()
        .get(format!("{}/v1/payment_intents/pi_x", harness.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .context("the server answers")?;
    assert_eq!(
        response.status().as_u16(),
        404,
        "the token must authorise a read; a 403 here is the bug this case exists for"
    );

    // The control: the same request *with* `client_id` grants the same set.
    let assertion = vpay_sdk::auth::mint_client_assertion(
        &harness.credentials(),
        &harness.token_endpoint(),
        Duration::from_secs(60),
    )
    .context("minting a second assertion (the first is spent)")?;
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

    let mint = || {
        vpay_sdk::auth::mint_client_assertion(
            &harness.credentials(),
            &harness.token_endpoint(),
            Duration::from_secs(60),
        )
    };

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

    // ...and the narrowed token really is narrower at the boundary, not
    // merely in its claim: a read passes, a write does not.
    let response = raw_client()
        .get(format!("{}/v1/payment_intents/pi_x", harness.base_url))
        .bearer_auth(&narrowed)
        .send()
        .await
        .context("the server answers a read")?;
    assert_eq!(response.status().as_u16(), 404, "a read scope reads");
    let response = raw_client()
        .post(format!("{}/v1/payment_intents", harness.base_url))
        .bearer_auth(&narrowed)
        .header("Idempotency-Key", "narrowed-tries-to-write")
        .form(&[("amount", "5000"), ("currency", "xaf")])
        .send()
        .await
        .context("the server answers a write")?;
    assert_eq!(
        response.status().as_u16(),
        403,
        "a token narrowed to `payments:read` must not be able to take a payment"
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
