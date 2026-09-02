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
//!   obtains a token and reaches an authenticated `/v1` route — which today
//!   answers the honest 404, because vpay implements no `/v1` resource yet;
//!   and the same route with no bearer token answers the 401 envelope.
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
//!
//! # What has actually been run
//!
//! All seven tests below **have been executed and pass through
//! [`migrated_postgres`] itself** — a real `postgres:16-alpine` container
//! started by `testcontainers`, the same path CI takes. 7/7, alongside the
//! rest of this crate's suite (26/26).
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
//! Repeats the small pool-and-migrate helper from `tests/postgres_smoke.rs`
//! for the reason `tests/authkestra_op_smoke.rs` states: each `tests/*.rs`
//! compiles to its own test binary, so there is no `pub` item to import
//! without introducing a shared module for a handful of lines. The container
//! start underneath it is shared:
//! `vpay_testkit::containers::start_postgres_with_retry`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Once};
use std::time::Duration;

use anyhow::Context;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rsa::pkcs1::{EncodeRsaPrivateKey as _, LineEnding};
use rsa::traits::PublicKeyParts as _;
use serde_json::{Value, json};
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::RouterDeps;
use vpay_api::op::MerchantOp;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_api::resource_auth::{JwtValidator, MerchantJwtValidator, Surface};
use vpay_config::oauth::{GrantType, MerchantClient};
use vpay_config::{Config, Deployment, MERCHANT_AUDIENCE};
use vpay_sdk::Credentials;

const CLIENT_ID: &str = "acme-cameroon";

/// The `aud` a `/dash/v1` token would carry —
/// `vpay_api::resource_auth::Surface::Dashboard::audience()`, which is not
/// `pub`-reachable as a value, so it is spelled here. Case (c) exists
/// precisely because these two strings must never be interchangeable.
const DASHBOARD_AUDIENCE: &str = "vpay:dash/v1";

/// Same as `tests/postgres_smoke.rs`'s: the container itself comes from
/// `vpay_testkit::containers::start_postgres_with_retry` (why the tag is
/// pinned, and which start errors are retried, are documented there); what
/// stays per-file is the pool and the migration run.
async fn migrated_postgres() -> anyhow::Result<(ContainerAsync<PostgresImage>, PgPool)> {
    let container = vpay_testkit::containers::start_postgres_with_retry()
        .await
        .context("postgres:16-alpine container starts (it is cached locally on this machine)")?;

    let host = container.get_host().await.context("container host")?;
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .context("container port")?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = PgPool::connect(&url)
        .await
        .context("connects to the freshly started container")?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("every migration under backends/migrations applies cleanly")?;

    Ok((container, pool))
}

/// `authkestra_resource::jwt::JwksCache::new` builds a `reqwest::Client`
/// eagerly, and the workspace pins reqwest with `rustls-no-provider`, which
/// panics without a process-wide default. `vpay-server`'s `main` installs
/// one at the top of startup for exactly this reason; this test binary is
/// its own process, so it has to do the same.
fn ensure_crypto_provider_installed() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
    });
}

/// An RSA keypair in the two shapes this file needs: the private half as a
/// PKCS#1 PEM (what a merchant hands `vpay_sdk::Credentials::rsa_pem`, and
/// what a Secret mount holds for the server's own key) and the public half
/// as a JWK Set (what vpay holds in YAML for a merchant).
///
/// Generated per call, never hard-coded. 2048 bits is the floor
/// `vpay_api::op::keys` enforces and is what keeps these tests to about a
/// second of key generation each.
fn generate_key() -> (String, Value) {
    let mut rng = rand::rngs::OsRng;
    let private_key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa key generation succeeds");
    let public_key = private_key.to_public_key();
    let pem = private_key
        .to_pkcs1_pem(LineEnding::LF)
        .expect("pkcs1 pem encoding succeeds")
        .to_string();

    let jwks = json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "n": URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be()),
            "e": URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be()),
        }]
    });
    (pem, jwks)
}

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
    ensure_crypto_provider_installed();

    let (container, pool) = migrated_postgres().await?;

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
        .ensure_active_in_database(&pool)
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
        merchant_clients: vec![MerchantClient {
            client_id: CLIENT_ID.to_owned(),
            jwks: Some(merchant_jwks),
            grant_types: vec![GrantType::ClientCredentials],
            scopes: vec!["payments:write".to_owned()],
            // `vpay:v1` is what both SDKs request by default and what
            // `Config::validate_all` requires a merchant to be able to
            // target; without it the token endpoint answers `invalid_target`.
            allowed_audiences: vec![MERCHANT_AUDIENCE.to_owned()],
            client_secret: None,
        }],
        dashboard_client: None,
    };

    let merchant_op = Arc::new(MerchantOp::new(&config, signing_key.clone(), pool.clone()));
    let merchant_validator = MerchantJwtValidator(
        JwtValidator::new(
            format!("http://{bound}/v1/oauth/jwks.json"),
            Duration::from_secs(300),
            merchant_op.issuer(),
            Surface::Merchant,
        )
        .expect("the vendored-roots JWKS client builds"),
    );

    let deps = RouterDeps {
        pool: pool.clone(),
        merchant_op,
        merchant_validator,
    };
    let server = tokio::spawn(async move {
        // A serve error here is not something a test can assert on
        // meaningfully — the assertions below fail on a refused connection
        // instead, which says more.
        let _ = axum::serve(listener, vpay_api::router(deps)).await;
    });

    Ok(Harness {
        _container: container,
        server,
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
    let payload = token
        .split('.')
        .nth(1)
        .expect("a JWT has three dot-separated segments");
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .expect("the payload segment is base64url");
    let claims: Value = serde_json::from_slice(&bytes).expect("the payload is JSON");
    claims
        .get("aud")
        .and_then(Value::as_str)
        .expect("every token this OP mints carries a string aud")
        .to_owned()
}

// --------------------------------------------------------------- case (a)

/// The whole happy path, and the honest answer at the end of it.
///
/// The SDK obtains a token (proving: assertion minted, client resolved from
/// YAML, `jti` recorded, token signed by the key announced in
/// `oauth_signing_keys`) and reaches `/v1/payment_intents/pi_x` — which
/// answers `404 unknown_route`, because vpay implements no payment-intent
/// route yet. That 404 **is** the assertion: it is only reachable past the
/// authentication boundary, so getting it proves the token was minted,
/// fetched back over loopback JWKS, and validated.
///
/// A 200 here would mean someone had invented a resource, which is the
/// failure mode `CLAUDE.md` names first.
#[tokio::test]
async fn an_sdk_client_authenticates_and_reaches_the_honest_404() -> anyhow::Result<()> {
    let harness = harness().await?;

    let error = harness
        .sdk_client()
        .payment_intents()
        .retrieve("pi_x")
        .await
        .expect_err("vpay implements no payment-intent route yet; a 200 would be fabricated");

    match error {
        vpay_sdk::Error::Api {
            status,
            ref kind,
            ref code,
            ..
        } => {
            assert_eq!(status, 404, "authenticated, then unimplemented: {error:?}");
            assert_eq!(kind, "invalid_request_error");
            assert_eq!(code.as_deref(), Some("unknown_route"));
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
        .expect_err("the honest 404 — see the case (a) test");

    vpay_db::disable_client(&harness.pool, CLIENT_ID, Some("integration test"))
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
            None,
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
