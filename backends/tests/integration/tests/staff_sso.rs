//! Staff single sign-on, end to end: the real `vpay_api::router` on a real
//! socket, over a real Postgres, against an identity provider simulated over
//! HTTP ([ADR-0030](../../../../docs/adr/0030-staff-sign-in-federates-to-an-oidc-provider.md)).
//!
//! # The provider is a WireMock host, and the tokens are really signed
//!
//! Discovery, a JWKS and a token endpoint are served by `wiremock` over real
//! HTTP, exactly as a payment rail is simulated (AGENTS.md rule 1: a stub is a
//! host in configuration, never a linked implementation). Every id_token is
//! signed by an RSA key **this file generates**, so what vpay verifies is a
//! real signature against a real JWKS fetched over a real socket — the
//! refusals below are the verifier refusing, not a test asserting that a flag
//! was set.
//!
//! The token endpoint is also where PKCE is checked: a mock matches only when
//! the `code_verifier` it is handed hashes (S256) to the `code_challenge` the
//! authorization URL carried, and only when the request authenticates with
//! `client_secret_basic`. A callback that sent the wrong verifier would find
//! no mock and be refused.
//!
//! # What it claims
//!
//! 1. the **happy path**: a first sign-in provisions a member, signs them in,
//!    and the dashboard's own authorization-code leg then mints a `/dash/v1`
//!    token that reads the bound merchant's payments — the invariant "no
//!    `/dash/v1` request runs without a token vpay issued" holding unchanged;
//! 2. a **second sign-in reuses the link**: one member, one `oidc` credential;
//! 3. **roles stay local**: claims named `is_admin` and `merchant_id` change
//!    nothing, and a provisioned member is a non-admin of the configured
//!    tenant;
//! 4. every **token defect is refused**: nonce, audience, issuer, expiry,
//!    unknown `kid`, `alg: none`, HS256;
//! 5. the **second-factor rule**: `amr` accepted, `acr` accepted, a password
//!    alone refused; and the **one `prompt=login` retry** — redirected once,
//!    signed in on a good second token, refused on a bad one, and **never**
//!    offered for any other defect;
//! 6. the **group gate**, a **disabled member**, **state replay**, a wrong
//!    state, an **email collision** (not linked by email), and a credentials
//!    row for **another issuer** (verifies nothing);
//! 7. the **rate limiter**, the **expiry** of a pending flow and its sweep, an
//!    unconfigured deployment, and the discovery document naming a different
//!    issuer.
//!
//! Password + TOTP is not re-tested here: `staff_sign_in.rs` passes unedited
//! and is the witness that the break-glass path is unchanged. One case below
//! checks the narrower thing this suite can: that configuring SSO leaves the
//! password route mounted.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs1::EncodeRsaPrivateKey as _;
use rsa::traits::PublicKeyParts as _;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, Row as _};
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use time::OffsetDateTime;
use vpay_config::{
    Config, CurrencyEntry, DashboardClient, Deployment, HostEntry, ProviderHost, RateLimitPolicy,
    RateLimits, StaffAuth, StaffSso,
};
use vpay_db::{NewPaymentIntent, Repositories};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

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
/// The dashboard's own OAuth redirect URI (vpay as OP), not the SSO one.
const REDIRECT_URI: &str = "http://127.0.0.1:3000/api/auth/callback";

const SSO_CLIENT_ID: &str = "vpay-staff";
const SSO_CLIENT_SECRET: &str = "the-sso-client-secret";
/// The dashboard's SSO callback (the provider redirects the browser here).
const SSO_REDIRECT_URI: &str = "http://127.0.0.1:3000/api/auth/sso/callback";
const REQUIRED_GROUP: &str = "vpay-staff";

const PEPPER: &str = "a-test-pepper-for-the-staff-sso-suite";

const PUSH_RAIL: &str = "mtn_momo";
const CURRENCY: &str = "XAF";

/// RFC 7636's own verifier and challenge (Appendix B).
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

// ------------------------------------------------------- the provider ------

/// The identity provider's signing key, as the two shapes the suite needs.
struct IdpKey {
    encoding: EncodingKey,
    jwk: Value,
    kid: String,
}

fn idp_key(kid: &str) -> IdpKey {
    let private = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).expect("rsa keygen");
    let pem = private
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .expect("pkcs1 pem")
        .to_string();
    let public = private.to_public_key();
    IdpKey {
        encoding: EncodingKey::from_rsa_pem(pem.as_bytes()).expect("encoding key"),
        jwk: json!({
            "kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid,
            "n": URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
            "e": URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
        }),
        kid: kid.to_owned(),
    }
}

/// A WireMock identity provider with discovery and a JWKS mounted.
struct Idp {
    server: MockServer,
    key: IdpKey,
    issuer: String,
}

impl Idp {
    /// `discovery_issuer` is what the discovery document *claims* to be; the
    /// honest provider passes the real one.
    async fn start(discovery_issuer: Option<&str>) -> Self {
        let server = MockServer::start().await;
        let issuer = format!("{}/realms/staff", server.uri());
        let key = idp_key("idp-key-1");
        let claimed = discovery_issuer.unwrap_or(&issuer);

        Mock::given(method("GET"))
            .and(path("/realms/staff/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": claimed,
                "authorization_endpoint": format!("{issuer}/protocol/openid-connect/auth"),
                "token_endpoint": format!("{issuer}/protocol/openid-connect/token"),
                "jwks_uri": format!("{issuer}/protocol/openid-connect/certs"),
                "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
                "code_challenge_methods_supported": ["S256", "plain"],
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/realms/staff/protocol/openid-connect/certs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                // An encryption key beside the signing key, as Keycloak
                // publishes: it must not break the set.
                "keys": [
                    key.jwk.clone(),
                    {"kty": "RSA", "use": "enc", "alg": "RSA-OAEP", "kid": "enc-1", "n": "AQAB", "e": "AQAB"},
                ],
            })))
            .mount(&server)
            .await;

        Self {
            server,
            key,
            issuer,
        }
    }

    /// Serves `id_token` for exactly this `code` — and only to a caller that
    /// authenticates with `client_secret_basic` and proves the PKCE verifier
    /// for `challenge`.
    async fn serve_code(&self, code: &str, challenge: &str, id_token: &str) {
        let basic = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD
                .encode(format!("{SSO_CLIENT_ID}:{SSO_CLIENT_SECRET}"))
        );
        Mock::given(method("POST"))
            .and(path("/realms/staff/protocol/openid-connect/token"))
            .and(header("authorization", basic.as_str()))
            .and(body_string_contains(format!("code={code}")))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(PkceVerifierFor(challenge.to_owned()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id_token": id_token,
                // Never read by vpay; present so a test can see it is not.
                "access_token": "IDP-ACCESS-TOKEN-MUST-NEVER-BE-USED",
                "token_type": "Bearer",
            })))
            .mount(&self.server)
            .await;
    }

    /// The provider rejects this code, as RFC 6749 §5.2 says.
    async fn refuse_code(&self, code: &str) {
        Mock::given(method("POST"))
            .and(path("/realms/staff/protocol/openid-connect/token"))
            .and(body_string_contains(format!("code={code}")))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})),
            )
            .mount(&self.server)
            .await;
    }

    /// Signs `claims` with the provider's key under the given algorithm and
    /// `kid`. RS256 only for the key; the header is what the cases vary.
    fn sign(&self, claims: &Value, kid: Option<&str>) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = kid.map(str::to_owned);
        jsonwebtoken::encode(&header, claims, &self.key.encoding).expect("signing an id_token")
    }
}

/// A wiremock matcher: the request's `code_verifier` hashes (S256) to this
/// challenge.
struct PkceVerifierFor(String);

impl Match for PkceVerifierFor {
    fn matches(&self, request: &Request) -> bool {
        let body = String::from_utf8_lossy(&request.body);
        body.split('&')
            .find_map(|pair| pair.strip_prefix("code_verifier="))
            .is_some_and(|verifier| {
                URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) == self.0
            })
    }
}

// --------------------------------------------------------- the harness ------

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    _server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    pool: PgPool,
    base_url: String,
    idp: Idp,
}

/// Which `accepted_*` rule the deployment under test configures.
#[derive(Clone)]
struct Mfa {
    acr: Vec<String>,
    amr: Vec<String>,
}

impl Mfa {
    /// The production shape, measured against Keycloak 26.6.1: `amr` only.
    fn amr_otp() -> Self {
        Self {
            acr: Vec::new(),
            amr: vec!["otp".to_owned()],
        }
    }
}

fn staff_auth(idp: &Idp, mfa: &Mfa, rate_limits: RateLimits) -> StaffAuth {
    StaffAuth {
        password_pepper: Some(PEPPER.to_owned()),
        totp_encryption_key: Some(URL_SAFE_NO_PAD.encode([9_u8; 32])),
        trusted_proxies: Vec::new(),
        rate_limits,
        access_token_ttl_seconds: StaffAuth::DEFAULT_ACCESS_TOKEN_TTL_SECONDS,
        sso: Some(StaffSso {
            issuer: idp.issuer.clone(),
            client_id: SSO_CLIENT_ID.to_owned(),
            client_secret: SSO_CLIENT_SECRET.to_owned(),
            redirect_uri: SSO_REDIRECT_URI.to_owned(),
            scopes: vec!["openid".into(), "email".into(), "profile".into()],
            required_group: REQUIRED_GROUP.to_owned(),
            groups_claim: "groups".to_owned(),
            accepted_acr: mfa.acr.clone(),
            accepted_amr: mfa.amr.clone(),
        }),
    }
}

fn config_with(base_url: &str, jwks_a: Value, jwks_b: Value, staff_auth: StaffAuth) -> Config {
    Config {
        deployment: Deployment {
            name: "staff-sso".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
            surfaces: None,
        },
        providers: vec![ProviderHost {
            code: PUSH_RAIL.to_owned(),
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
        dashboard_client: Some(DashboardClient {
            client_id: DASHBOARD_CLIENT.to_owned(),
            merchant_id: MERCHANT_A.to_owned(),
            redirect_uris: vec![REDIRECT_URI.to_owned()],
            scope: DASHBOARD_SCOPE.to_owned(),
            client_secret: None,
        }),
        staff_auth,
    }
}

async fn harness() -> anyhow::Result<Harness> {
    harness_with(Mfa::amr_otp(), RateLimits::default(), None, true).await
}

/// `with_sso == false` boots the same deployment with **no** `staff_auth.sso`
/// block, for the unconfigured cases.
async fn harness_with(
    mfa: Mfa,
    rate_limits: RateLimits,
    discovery_issuer: Option<&str>,
    with_sso: bool,
) -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let idp = Idp::start(discovery_issuer).await;
    let (container, repositories, pool) = migrated_postgres().await?;
    let (server_pem, _jwks) = generate_key();
    let (_pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();

    let mut staff_auth = staff_auth(&idp, &mfa, rate_limits);
    if !with_sso {
        staff_auth.sso = None;
    }
    let served = serve(&repositories, &server_pem, move |base_url| {
        config_with(base_url, jwks_a, jwks_b, staff_auth)
    })
    .await?;

    Ok(Harness {
        _container: container,
        _server: served.server,
        repositories,
        pool,
        base_url: served.base_url,
        idp,
    })
}

/// One flow begun: what the browser was handed, and what the provider would
/// have seen in the authorization request.
struct Flow {
    pending: String,
    url: reqwest::Url,
    state: String,
    nonce: String,
    challenge: String,
}

impl Flow {
    fn param(&self, name: &str) -> Option<String> {
        self.url
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }
}

impl Harness {
    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("a plain-HTTP reqwest client builds")
    }

    async fn post_form(
        &self,
        path: &str,
        session: Option<&str>,
        form: &[(&str, &str)],
    ) -> anyhow::Result<(u16, Value)> {
        let mut request = self
            .client()
            .post(format!("{}{path}", self.base_url))
            .form(form);
        if let Some(session) = session {
            request = request.header(vpay_api::staff::SESSION_HEADER, session);
        }
        let response = request.send().await.context("sending a request")?;
        let status = response.status().as_u16();
        let body = response.text().await.context("reading the body")?;
        Ok((status, serde_json::from_str(&body).unwrap_or(Value::Null)))
    }

    async fn get_json(
        &self,
        path: &str,
        session: Option<&str>,
        bearer: Option<&str>,
    ) -> anyhow::Result<(u16, Value)> {
        let mut request = self.client().get(format!("{}{path}", self.base_url));
        if let Some(session) = session {
            request = request.header(vpay_api::staff::SESSION_HEADER, session);
        }
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        let response = request.send().await.context("sending a request")?;
        let status = response.status().as_u16();
        let body = response.text().await.context("reading the body")?;
        Ok((status, serde_json::from_str(&body).unwrap_or(Value::Null)))
    }

    /// `POST /staff/sso/start`, parsed.
    async fn begin(&self) -> anyhow::Result<Flow> {
        let (status, body) = self
            .post_form("/dash/v1/staff/sso/start", None, &[])
            .await?;
        anyhow::ensure!(status == 200, "start: {status} {body}");
        flow_from(&body)
    }

    /// Mounts the token endpoint for `code` and POSTs the callback.
    async fn finish(
        &self,
        flow: &Flow,
        code: &str,
        id_token: &str,
    ) -> anyhow::Result<(u16, Value)> {
        self.idp.serve_code(code, &flow.challenge, id_token).await;
        self.callback(&flow.pending, code, &flow.state).await
    }

    async fn callback(
        &self,
        pending: &str,
        code: &str,
        state: &str,
    ) -> anyhow::Result<(u16, Value)> {
        self.post_form(
            "/dash/v1/staff/sso/callback",
            Some(pending),
            &[("code", code), ("state", state)],
        )
        .await
    }

    /// The claims of a token that should be accepted, for this flow.
    fn claims(&self, flow: &Flow, sub: &str, email: &str) -> Value {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        json!({
            "iss": self.idp.issuer,
            "aud": SSO_CLIENT_ID,
            "azp": SSO_CLIENT_ID,
            "sub": sub,
            "exp": now + 300,
            "iat": now,
            "nonce": flow.nonce,
            "email": email,
            "name": "Ada Lovelace",
            "groups": [REQUIRED_GROUP, "unrelated"],
            "acr": "0",
            "amr": ["pwd", "otp"],
        })
    }

    /// A whole good sign-in for `sub`/`email`; the session token.
    async fn sign_in(&self, sub: &str, email: &str, code: &str) -> anyhow::Result<String> {
        let flow = self.begin().await?;
        let token = self
            .idp
            .sign(&self.claims(&flow, sub, email), Some(&self.idp.key.kid));
        let (status, body) = self.finish(&flow, code, &token).await?;
        anyhow::ensure!(status == 200, "callback: {status} {body}");
        anyhow::ensure!(
            field(&body, "outcome") == "signed_in",
            "expected signed_in: {body}"
        );
        Ok(field(&body, "session")
            .as_str()
            .context("a session token")?
            .to_owned())
    }

    /// Runs one callback with `claims` (and `kid`), expecting a refusal.
    async fn refused(
        &self,
        claims_edit: impl FnOnce(&mut Value),
        kid: Option<&str>,
    ) -> (u16, Value) {
        let flow = self.begin().await.expect("start");
        let mut claims = self.claims(&flow, "sub-refused", "refused@example.test");
        claims_edit(&mut claims);
        let token = self.idp.sign(&claims, kid);
        self.finish(&flow, "code-refused", &token)
            .await
            .expect("the callback request itself succeeds")
    }

    async fn count(&self, sql: &'static str) -> i64 {
        sqlx::query(sql)
            .fetch_one(&self.pool)
            .await
            .expect("count query")
            .get::<i64, _>(0)
    }

    async fn authorize(&self, session: &str) -> anyhow::Result<(u16, Option<String>)> {
        let response = self
            .client()
            .get(format!("{}/dash/v1/oauth/authorize", self.base_url))
            .header(vpay_api::staff::SESSION_HEADER, session)
            .query(&[
                ("client_id", DASHBOARD_CLIENT),
                ("redirect_uri", REDIRECT_URI),
                ("response_type", "code"),
                ("scope", DASHBOARD_SCOPE),
                ("code_challenge", CHALLENGE),
                ("code_challenge_method", "S256"),
            ])
            .send()
            .await
            .context("authorize")?;
        let status = response.status().as_u16();
        let code = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|location| location.split_once('?'))
            .and_then(|(_, query)| {
                query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("code=").map(str::to_owned))
            });
        Ok((status, code))
    }
}

fn flow_from(body: &Value) -> anyhow::Result<Flow> {
    let pending = field(body, "session")
        .as_str()
        .context("a pending-flow token")?
        .to_owned();
    let url = reqwest::Url::parse(
        field(body, "authorization_url")
            .as_str()
            .context("an authorization_url")?,
    )?;
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .with_context(|| format!("the authorization URL carries {name}"))
    };
    Ok(Flow {
        state: param("state")?,
        nonce: param("nonce")?,
        challenge: param("code_challenge")?,
        pending,
        url,
    })
}

/// Sets one claim; `clippy::indexing_slicing` is denied in these suites.
fn put(claims: &mut Value, key: &str, value: Value) {
    if let Some(object) = claims.as_object_mut() {
        object.insert(key.to_owned(), value);
    }
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

/// The callback's `outcome`, or `""`.
fn outcome(body: &Value) -> &str {
    field(body, "outcome").as_str().unwrap_or("")
}

async fn seed_intent(repositories: &dyn Repositories, merchant_id: &str, id: &str) {
    vpay_db::PaymentIntents::insert(
        repositories,
        &NewPaymentIntent {
            id: id.to_owned(),
            merchant_id: merchant_id.to_owned(),
            livemode: false,
            amount: 5000,
            currency_code: CURRENCY.to_owned(),
            status: vpay_core::IntentStatus::INITIAL.as_wire_str().to_owned(),
            payment_method_types: json!([PUSH_RAIL]),
            metadata: json!({}),
            description: None,
            customer_id: None,
            last_payment_error_code: None,
            last_payment_error_message: None,
            client_secret_suffix: format!("ssosuffix{id:x<32}"),
            created_at: OffsetDateTime::now_utc(),
        },
    )
    .await
    .expect("seeding an intent");
}

// ------------------------------------------------------------------ cases ---

/// **The sentence this file exists for**: a person who has never been here
/// signs in through the provider and reads their merchant's payments with a
/// token vpay itself issued.
#[tokio::test]
async fn a_first_sso_sign_in_provisions_a_member_and_reads_their_merchants_payments()
-> anyhow::Result<()> {
    let h = harness().await?;
    seed_intent(h.repositories.as_ref(), MERCHANT_A, "pi_sso_a").await;
    seed_intent(h.repositories.as_ref(), MERCHANT_B, "pi_sso_b").await;

    // The dashboard asks whether to show the button.
    let (status, body) = h.get_json("/dash/v1/staff/sso", None, None).await?;
    assert_eq!(
        (status, field(&body, "enabled")),
        (200, &Value::Bool(true)),
        "{body}"
    );

    let flow = h.begin().await?;
    assert_eq!(flow.param("response_type").as_deref(), Some("code"));
    assert_eq!(flow.param("client_id").as_deref(), Some(SSO_CLIENT_ID));
    assert_eq!(
        flow.param("redirect_uri").as_deref(),
        Some(SSO_REDIRECT_URI)
    );
    assert_eq!(flow.param("scope").as_deref(), Some("openid email profile"));
    assert_eq!(flow.param("code_challenge_method").as_deref(), Some("S256"));
    assert_eq!(
        flow.param("prompt"),
        None,
        "the first leg does not force a login"
    );
    assert!(
        flow.url
            .as_str()
            .starts_with(&format!("{}/protocol/openid-connect/auth?", h.idp.issuer)),
        "{}",
        flow.url
    );

    // A pending-flow token is not a session.
    let (status, _) = h
        .get_json("/dash/v1/staff/session", Some(&flow.pending), None)
        .await?;
    assert_eq!(
        status, 401,
        "the pending-flow token must not read as a session"
    );

    let token = h.idp.sign(
        &h.claims(&flow, "kc-user-1", "Ada@Example.Test"),
        Some("idp-key-1"),
    );
    let (status, body) = h.finish(&flow, "code-1", &token).await?;
    assert_eq!(status, 200, "{body}");
    assert_eq!(field(&body, "outcome"), "signed_in", "{body}");
    let session = field(&body, "session")
        .as_str()
        .expect("a session")
        .to_owned();
    assert_ne!(
        session, flow.pending,
        "the pending-flow token never becomes the session (fixation)"
    );

    // The member, as provisioned: bound to the configured tenant, not an admin.
    let row =
        sqlx::query("SELECT merchant_id, is_admin, email, display_name, status FROM staff_members")
            .fetch_one(&h.pool)
            .await?;
    assert_eq!(row.get::<String, _>("merchant_id"), MERCHANT_A);
    assert!(!row.get::<bool, _>("is_admin"));
    assert_eq!(
        row.get::<String, _>("email"),
        "ada@example.test",
        "lower-cased"
    );
    assert_eq!(row.get::<String, _>("display_name"), "Ada Lovelace");
    assert_eq!(row.get::<String, _>("status"), "active");
    let link =
        sqlx::query("SELECT kind, material, issuer, subject FROM credentials WHERE kind = 'oidc'")
            .fetch_one(&h.pool)
            .await?;
    assert_eq!(
        link.get::<Option<String>, _>("material"),
        None,
        "a federated link holds no secret"
    );
    assert_eq!(link.get::<String, _>("issuer"), h.idp.issuer);
    assert_eq!(link.get::<String, _>("subject"), "kc-user-1");
    assert_eq!(
        h.count("SELECT count(*) FROM credentials WHERE kind IN ('password','totp')")
            .await,
        0,
        "an SSO member never holds a password or a second factor of vpay's"
    );

    // The session is authenticated and owns no one-time password.
    let (status, body) = h
        .get_json("/dash/v1/staff/session", Some(&session), None)
        .await?;
    assert_eq!(status, 200, "{body}");
    assert_eq!(field(&body, "merchant_id"), MERCHANT_A);
    assert_eq!(
        field(&body, "password_change_required"),
        &Value::Bool(false)
    );

    // The dashboard's own authorization-code leg, unchanged.
    let (status, code) = h.authorize(&session).await?;
    assert_eq!(status, 302);
    let code = code.expect("a code");
    let (status, body) = h
        .post_form(
            "/dash/v1/oauth/token",
            None,
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", REDIRECT_URI),
                ("code_verifier", VERIFIER),
                ("client_id", DASHBOARD_CLIENT),
            ],
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let access = field(&body, "access_token")
        .as_str()
        .expect("a token")
        .to_owned();
    assert!(
        !access.contains("IDP-ACCESS-TOKEN"),
        "the provider's access token is never what /dash/v1 sees"
    );

    let (status, body) = h
        .get_json("/dash/v1/payment_intents", None, Some(&access))
        .await?;
    assert_eq!(status, 200, "{body}");
    let rendered = body.to_string();
    assert!(rendered.contains("pi_sso_a"), "{rendered}");
    assert!(
        !rendered.contains("pi_sso_b"),
        "bound to {MERCHANT_A}: {rendered}"
    );

    // The flow is gone: it was consumed, not flagged.
    assert_eq!(h.count("SELECT count(*) FROM staff_sso_attempts").await, 0);
    Ok(())
}

/// A second sign-in finds the link: one member, one credential — and it
/// matches on `(iss, sub)` even when the address in the token has changed.
#[tokio::test]
async fn a_second_sign_in_reuses_the_link_and_never_matches_on_email() -> anyhow::Result<()> {
    let h = harness().await?;
    h.sign_in("kc-user-1", "ada@example.test", "code-1").await?;
    h.sign_in("kc-user-1", "changed-address@example.test", "code-2")
        .await?;

    assert_eq!(h.count("SELECT count(*) FROM staff_members").await, 1);
    assert_eq!(
        h.count("SELECT count(*) FROM credentials WHERE kind = 'oidc'")
            .await,
        1
    );
    let email: String = sqlx::query("SELECT email FROM staff_members")
        .fetch_one(&h.pool)
        .await?
        .get("email");
    assert_eq!(
        email, "ada@example.test",
        "the address is written once, when the member is provisioned, and never updated from a token"
    );
    Ok(())
}

/// ADR-0019 § R2: no claim reads or writes `is_admin` or `merchant_id`.
#[tokio::test]
async fn roles_stay_local_whatever_the_token_claims() -> anyhow::Result<()> {
    let h = harness().await?;
    let flow = h.begin().await?;
    let mut claims = h.claims(&flow, "kc-boss", "boss@example.test");
    for (key, value) in [
        ("is_admin", json!(true)),
        ("admin", json!(true)),
        ("merchant_id", json!(MERCHANT_B)),
        ("vpay_merchant_id", json!(MERCHANT_B)),
        ("roles", json!(["admin"])),
    ] {
        put(&mut claims, key, value);
    }
    let token = h.idp.sign(&claims, Some("idp-key-1"));
    let (status, body) = h.finish(&flow, "code-1", &token).await?;
    assert_eq!(status, 200, "{body}");

    let row = sqlx::query("SELECT merchant_id, is_admin FROM staff_members")
        .fetch_one(&h.pool)
        .await?;
    assert_eq!(row.get::<String, _>("merchant_id"), MERCHANT_A);
    assert!(!row.get::<bool, _>("is_admin"));
    Ok(())
}

/// Every id_token defect is refused with the module's one `401`, the flow is
/// burned, and nobody is provisioned.
#[tokio::test]
async fn token_defects_are_refused_and_provision_nobody() -> anyhow::Result<()> {
    let h = harness().await?;

    let now = OffsetDateTime::now_utc().unix_timestamp();
    #[allow(clippy::type_complexity)]
    let cases: Vec<(&str, Box<dyn FnOnce(&mut Value)>, Option<&str>)> = vec![
        (
            "wrong nonce",
            Box::new(|c| put(c, "nonce", json!("not-the-nonce"))),
            Some("idp-key-1"),
        ),
        (
            "no nonce",
            Box::new(|c| {
                c.as_object_mut().unwrap().remove("nonce");
            }),
            Some("idp-key-1"),
        ),
        (
            "wrong audience",
            Box::new(|c| put(c, "aud", json!("another-client"))),
            Some("idp-key-1"),
        ),
        (
            "wrong azp",
            Box::new(|c| put(c, "azp", json!("another-client"))),
            Some("idp-key-1"),
        ),
        (
            "wrong issuer",
            Box::new(|c| put(c, "iss", json!("http://evil.example.test/realms/staff"))),
            Some("idp-key-1"),
        ),
        (
            "expired",
            Box::new(move |c| put(c, "exp", json!(now - 3600))),
            Some("idp-key-1"),
        ),
        (
            "issued in the future",
            Box::new(move |c| put(c, "iat", json!(now + 3600))),
            Some("idp-key-1"),
        ),
        ("unknown kid", Box::new(|_| {}), Some("rotated-away")),
        ("no kid", Box::new(|_| {}), None),
    ];
    for (name, edit, kid) in cases {
        let (status, body) = h.refused(edit, kid).await;
        assert_eq!(status, 401, "{name}: {body}");
        assert_ne!(
            field(&body, "outcome"),
            "reauthenticate",
            "{name} must not earn a retry"
        );
    }

    assert_eq!(h.count("SELECT count(*) FROM staff_members").await, 0);
    assert_eq!(h.count("SELECT count(*) FROM credentials").await, 0);
    Ok(())
}

/// `alg: none` and HS256 — the two algorithm-confusion shapes — are refused
/// before any key is consulted.
#[tokio::test]
async fn alg_none_and_hs256_are_refused() -> anyhow::Result<()> {
    let h = harness().await?;

    // HS256 "signed" with the issuer's public key bytes as the secret, which
    // is the classic confusion attack.
    let flow = h.begin().await?;
    let claims = h.claims(&flow, "kc-evil", "evil@example.test");
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some("idp-key-1".to_owned());
    let hs = jsonwebtoken::encode(
        &header,
        &claims,
        &EncodingKey::from_secret(field(&h.idp.key.jwk, "n").as_str().unwrap().as_bytes()),
    )?;
    let (status, body) = h.finish(&flow, "code-hs", &hs).await?;
    assert_eq!(status, 401, "{body}");

    let flow = h.begin().await?;
    let claims = h.claims(&flow, "kc-evil", "evil@example.test");
    let none = format!(
        "{}.{}.",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT","kid":"idp-key-1"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let (status, body) = h.finish(&flow, "code-none", &none).await?;
    assert_eq!(status, 401, "{body}");

    assert_eq!(h.count("SELECT count(*) FROM staff_members").await, 0);
    Ok(())
}

/// The group gate: a valid, second-factor-proving token for somebody outside
/// the required group is refused — immediately, without a retry.
#[tokio::test]
async fn a_person_outside_the_required_group_is_refused_without_a_retry() -> anyhow::Result<()> {
    let h = harness().await?;
    // Also lacks the second factor: the group refusal must win, with no retry.
    let (status, body) = h
        .refused(
            |c| {
                put(c, "groups", json!(["somebody-else"]));
                put(c, "amr", json!(["pwd"]));
            },
            Some("idp-key-1"),
        )
        .await;
    assert_eq!(status, 401, "{body}");
    assert_ne!(field(&body, "outcome"), "reauthenticate");

    // The claim is read by its configured name only, and exactly.
    let (status, _) = h
        .refused(
            |c| put(c, "groups", json!(["/vpay-staff", "VPAY-STAFF"])),
            Some("idp-key-1"),
        )
        .await;
    assert_eq!(status, 401);
    let (status, _) = h
        .refused(
            |c| {
                c.as_object_mut().unwrap().remove("groups");
            },
            Some("idp-key-1"),
        )
        .await;
    assert_eq!(status, 401);
    assert_eq!(h.count("SELECT count(*) FROM staff_members").await, 0);
    Ok(())
}

/// The second factor, `amr` rule: accepted when it intersects the list.
#[tokio::test]
async fn an_amr_that_names_an_accepted_method_is_accepted() -> anyhow::Result<()> {
    let h = harness().await?;
    h.sign_in("kc-user-1", "ada@example.test", "code-1").await?;
    Ok(())
}

/// The second factor, `acr` rule: a deployment that configures only
/// `accepted_acr` accepts a token whose `acr` is in it and nothing else.
#[tokio::test]
async fn an_acr_that_is_accepted_is_accepted_when_it_is_the_configured_rule() -> anyhow::Result<()>
{
    let h = harness_with(
        Mfa {
            acr: vec!["mfa".to_owned()],
            amr: Vec::new(),
        },
        RateLimits::default(),
        None,
        true,
    )
    .await?;

    let flow = h.begin().await?;
    let mut claims = h.claims(&flow, "kc-user-1", "ada@example.test");
    put(&mut claims, "acr", json!("mfa"));
    put(&mut claims, "amr", json!([]));
    let token = h.idp.sign(&claims, Some("idp-key-1"));
    let (status, body) = h.finish(&flow, "code-1", &token).await?;
    assert_eq!((status, outcome(&body)), (200, "signed_in"), "{body}");

    // `amr: [otp]` means nothing to a deployment whose rule is acr-only.
    let flow = h.begin().await?;
    let mut claims = h.claims(&flow, "kc-user-2", "grace@example.test");
    put(&mut claims, "acr", json!("1"));
    put(&mut claims, "amr", json!(["pwd", "otp"]));
    let token = h.idp.sign(&claims, Some("idp-key-1"));
    let (status, body) = h.finish(&flow, "code-2", &token).await?;
    assert_eq!((status, outcome(&body)), (200, "reauthenticate"), "{body}");
    Ok(())
}

/// **The measured Keycloak behaviour, (a)**: a first token whose only defect
/// is `acr "1"` / `amr ["pwd"]` is not refused; the browser is sent back to
/// the provider with `prompt=login`, under a new flow.
///
/// **(b)**: the retry's token proves `otp` and the person is signed in.
#[tokio::test]
async fn a_password_only_token_earns_one_prompt_login_retry_and_the_retry_signs_in()
-> anyhow::Result<()> {
    let h = harness().await?;

    let flow = h.begin().await?;
    let mut claims = h.claims(&flow, "kc-user-1", "ada@example.test");
    put(&mut claims, "acr", json!("1"));
    put(&mut claims, "amr", json!(["pwd"]));
    let first = h.idp.sign(&claims, Some("idp-key-1"));
    let (status, body) = h.finish(&flow, "code-1", &first).await?;
    assert_eq!(status, 200, "{body}");
    assert_eq!(field(&body, "outcome"), "reauthenticate", "{body}");
    assert_eq!(
        h.count("SELECT count(*) FROM staff_members").await,
        0,
        "nobody is provisioned on a token that does not prove a second factor"
    );

    let retry = flow_from(&body)?;
    assert_eq!(
        retry.param("prompt").as_deref(),
        Some("login"),
        "{}",
        retry.url
    );
    assert_ne!(retry.pending, flow.pending, "a new pending-flow token");
    assert_ne!(retry.state, flow.state, "a new state");
    assert_ne!(retry.nonce, flow.nonce, "a new nonce");
    assert_ne!(retry.challenge, flow.challenge, "a new PKCE pair");
    assert_eq!(
        h.count("SELECT count(*) FROM staff_sso_attempts WHERE is_retry")
            .await,
        1,
        "the retry is recorded as the retry"
    );

    // The old flow is spent.
    let (status, _) = h.callback(&flow.pending, "code-1", &flow.state).await?;
    assert_eq!(status, 401);

    let mut claims = h.claims(&retry, "kc-user-1", "ada@example.test");
    put(&mut claims, "acr", json!("0"));
    put(&mut claims, "amr", json!(["pwd", "otp"]));
    let second = h.idp.sign(&claims, Some("idp-key-1"));
    let (status, body) = h.finish(&retry, "code-2", &second).await?;
    assert_eq!((status, outcome(&body)), (200, "signed_in"), "{body}");
    assert_eq!(h.count("SELECT count(*) FROM staff_members").await, 1);
    Ok(())
}

/// **(c)**: a retry whose token still lacks the second factor is refused —
/// at most one retry, no loop.
#[tokio::test]
async fn a_retry_that_still_lacks_the_second_factor_is_refused() -> anyhow::Result<()> {
    let h = harness().await?;

    let flow = h.begin().await?;
    let mut claims = h.claims(&flow, "kc-user-1", "ada@example.test");
    put(&mut claims, "amr", json!(["pwd"]));
    let (status, body) = h
        .finish(&flow, "code-1", &h.idp.sign(&claims, Some("idp-key-1")))
        .await?;
    assert_eq!(
        field(&body, "outcome"),
        "reauthenticate",
        "{body} ({status})"
    );

    let retry = flow_from(&body)?;
    let mut claims = h.claims(&retry, "kc-user-1", "ada@example.test");
    put(&mut claims, "amr", json!(["pwd"]));
    let (status, body) = h
        .finish(&retry, "code-2", &h.idp.sign(&claims, Some("idp-key-1")))
        .await?;
    assert_eq!(status, 401, "{body}");
    assert_ne!(
        field(&body, "outcome"),
        "reauthenticate",
        "no second retry: {body}"
    );

    assert_eq!(h.count("SELECT count(*) FROM staff_members").await, 0);
    assert_eq!(
        h.count("SELECT count(*) FROM staff_sso_attempts").await,
        0,
        "nothing left in flight"
    );
    Ok(())
}

/// **(d)**: any other failure of a retry is refused too, and any other
/// failure of a *first* flow never earns one — the retry is for the second
/// factor alone.
#[tokio::test]
async fn no_other_failure_earns_a_retry() -> anyhow::Result<()> {
    let h = harness().await?;
    type Edit = Box<dyn FnOnce(&mut Value)>;
    let cases: Vec<(&str, Edit)> = vec![
        ("bad nonce", Box::new(|c| put(c, "nonce", json!("nope")))),
        (
            "wrong aud",
            Box::new(|c| put(c, "aud", json!("someone-else"))),
        ),
        (
            "wrong iss",
            Box::new(|c| put(c, "iss", json!("http://evil.example.test/realms/staff"))),
        ),
        ("missing group", Box::new(|c| put(c, "groups", json!([])))),
    ];
    for (name, edit) in cases {
        // Each also lacks the second factor, so that a retry WOULD be the
        // outcome if the second-factor rule were consulted first.
        let (status, body) = h
            .refused(
                |c| {
                    put(c, "amr", json!(["pwd"]));
                    edit(c);
                },
                Some("idp-key-1"),
            )
            .await;
        assert_eq!(status, 401, "{name}: {body}");
        assert_ne!(field(&body, "outcome"), "reauthenticate", "{name}");
    }
    assert_eq!(
        h.count("SELECT count(*) FROM staff_sso_attempts WHERE is_retry")
            .await,
        0
    );
    Ok(())
}

/// A disabled member is refused even with a perfect token — before any retry.
#[tokio::test]
async fn a_disabled_member_is_refused_even_with_a_valid_token() -> anyhow::Result<()> {
    let h = harness().await?;
    h.sign_in("kc-user-1", "ada@example.test", "code-1").await?;
    sqlx::query("UPDATE staff_members SET status = 'disabled'")
        .execute(&h.pool)
        .await?;

    let (status, body) = h
        .refused(
            |c| {
                put(c, "sub", json!("kc-user-1"));
                put(c, "email", json!("ada@example.test"));
            },
            Some("idp-key-1"),
        )
        .await;
    assert_eq!(status, 401, "{body}");

    // And one that also lacks the second factor is refused outright.
    let (status, body) = h
        .refused(
            |c| {
                put(c, "sub", json!("kc-user-1"));
                put(c, "amr", json!(["pwd"]));
            },
            Some("idp-key-1"),
        )
        .await;
    assert_eq!(status, 401, "{body}");
    assert_ne!(field(&body, "outcome"), "reauthenticate");
    Ok(())
}

/// The callback's `state` is single use: replaying a spent flow is refused,
/// and so is a wrong `state` — which burns the flow it was presented to.
#[tokio::test]
async fn a_replayed_or_wrong_state_is_refused_and_burns_the_flow() -> anyhow::Result<()> {
    let h = harness().await?;

    let flow = h.begin().await?;
    let token = h.idp.sign(
        &h.claims(&flow, "kc-user-1", "ada@example.test"),
        Some("idp-key-1"),
    );
    let (status, body) = h.finish(&flow, "code-1", &token).await?;
    assert_eq!(status, 200, "{body}");
    // The identical callback again: the flow is gone.
    let (status, _) = h.callback(&flow.pending, "code-1", &flow.state).await?;
    assert_eq!(status, 401, "a replay of a completed callback");

    // A wrong state.
    let flow = h.begin().await?;
    let token = h.idp.sign(
        &h.claims(&flow, "kc-user-2", "grace@example.test"),
        Some("idp-key-1"),
    );
    h.idp.serve_code("code-2", &flow.challenge, &token).await;
    let (status, _) = h.callback(&flow.pending, "code-2", "not-the-state").await?;
    assert_eq!(status, 401);
    let (status, _) = h.callback(&flow.pending, "code-2", &flow.state).await?;
    assert_eq!(
        status, 401,
        "the right state, afterwards, finds the flow already burned"
    );

    // A pending-flow token that never existed.
    let (status, _) = h.callback("made-up-token", "code-3", "state").await?;
    assert_eq!(status, 401);
    // And none at all.
    let (status, _) = h
        .post_form(
            "/dash/v1/staff/sso/callback",
            None,
            &[("code", "c"), ("state", "s")],
        )
        .await?;
    assert_eq!(status, 401);
    assert_eq!(h.count("SELECT count(*) FROM staff_members").await, 1);
    Ok(())
}

/// ADR-0019 decision 5: an address that belongs to a member this identity is
/// not linked to is a refusal — never a link by email.
#[tokio::test]
async fn an_email_that_belongs_to_an_unlinked_member_is_refused_not_linked() -> anyhow::Result<()> {
    let h = harness().await?;
    sqlx::query(
        "INSERT INTO staff_members (id, merchant_id, email, display_name, status, is_admin, created_at, updated_at)
         VALUES ('stf_existing', $1, 'ada@example.test', 'Existing Ada', 'active', true, now(), now())",
    )
    .bind(MERCHANT_A)
    .execute(&h.pool)
    .await?;

    let flow = h.begin().await?;
    let token = h.idp.sign(
        &h.claims(&flow, "kc-impostor", "ada@example.test"),
        Some("idp-key-1"),
    );
    let (status, body) = h.finish(&flow, "code-1", &token).await?;
    assert_eq!(status, 401, "{body}");

    assert_eq!(
        h.count("SELECT count(*) FROM staff_members").await,
        1,
        "nobody new"
    );
    assert_eq!(
        h.count("SELECT count(*) FROM credentials").await,
        0,
        "and nothing linked"
    );
    assert_eq!(
        h.count("SELECT count(*) FROM staff_sessions").await,
        0,
        "and no session minted for the existing member"
    );
    Ok(())
}

/// ADR-0019 § R3: a `credentials` row whose issuer is not the configured one
/// verifies nothing. The same `sub` from the configured issuer is a stranger.
#[tokio::test]
async fn a_credentials_row_for_another_issuer_verifies_nothing() -> anyhow::Result<()> {
    let h = harness().await?;
    sqlx::query(
        "INSERT INTO staff_members (id, merchant_id, email, display_name, status, is_admin, created_at, updated_at)
         VALUES ('stf_other_idp', $1, 'other@example.test', 'Other', 'active', true, now(), now())",
    )
    .bind(MERCHANT_A)
    .execute(&h.pool)
    .await?;
    sqlx::query(
        "INSERT INTO credentials (id, staff_member_id, kind, material, issuer, subject, counter, must_change, created_at, updated_at)
         VALUES ('cred_other_idp', 'stf_other_idp', 'oidc', NULL, 'https://accounts.elsewhere.test', 'kc-user-1', 0, false, now(), now())",
    )
    .execute(&h.pool)
    .await?;

    let session = h.sign_in("kc-user-1", "ada@example.test", "code-1").await?;
    let (status, body) = h
        .get_json("/dash/v1/staff/session", Some(&session), None)
        .await?;
    assert_eq!(status, 200, "{body}");
    assert_ne!(
        field(&body, "staff_id"),
        "stf_other_idp",
        "the foreign row must not have authenticated anybody"
    );
    assert_eq!(field(&body, "email"), "ada@example.test");
    assert_eq!(h.count("SELECT count(*) FROM staff_members").await, 2);
    Ok(())
}

/// The provider refusing the code is one `401`.
#[tokio::test]
async fn a_code_the_provider_refuses_is_refused() -> anyhow::Result<()> {
    let h = harness().await?;
    let flow = h.begin().await?;
    h.idp.refuse_code("spent-code").await;
    let (status, body) = h.callback(&flow.pending, "spent-code", &flow.state).await?;
    assert_eq!(status, 401, "{body}");
    Ok(())
}

/// PKCE is checked by the provider and the token endpoint is called with
/// `client_secret_basic`: a mock that matches only on those proves vpay sent
/// them, because without them no mock matches and the callback is refused.
/// (The happy path above is the positive half; this is the negative half.)
#[tokio::test]
async fn a_token_endpoint_that_wants_another_verifier_is_not_satisfied() -> anyhow::Result<()> {
    let h = harness().await?;
    let flow = h.begin().await?;
    let token = h.idp.sign(
        &h.claims(&flow, "kc-user-1", "ada@example.test"),
        Some("idp-key-1"),
    );
    // Served only for a DIFFERENT challenge: vpay's verifier hashes to
    // `flow.challenge`, so this mock must not match.
    h.idp.serve_code("code-1", CHALLENGE, &token).await;
    let (status, _) = h.callback(&flow.pending, "code-1", &flow.state).await?;
    assert_eq!(status, 401);
    Ok(())
}

/// A pending flow older than ten minutes is refused, and starting a new one
/// sweeps the expired rows.
#[tokio::test]
async fn a_pending_flow_expires_and_start_sweeps_expired_rows() -> anyhow::Result<()> {
    let h = harness().await?;

    let flow = h.begin().await?;
    let token = h.idp.sign(
        &h.claims(&flow, "kc-user-1", "ada@example.test"),
        Some("idp-key-1"),
    );
    sqlx::query("UPDATE staff_sso_attempts SET created_at = now() - interval '20 minutes', expires_at = now() - interval '10 minutes'")
        .execute(&h.pool)
        .await?;
    let (status, body) = h.finish(&flow, "code-1", &token).await?;
    assert_eq!(status, 401, "{body}");
    assert_eq!(h.count("SELECT count(*) FROM staff_members").await, 0);

    // Two abandoned flows, then a start: only the new one survives.
    h.begin().await?;
    h.begin().await?;
    sqlx::query("UPDATE staff_sso_attempts SET created_at = now() - interval '20 minutes', expires_at = now() - interval '10 minutes'")
        .execute(&h.pool)
        .await?;
    h.begin().await?;
    assert_eq!(h.count("SELECT count(*) FROM staff_sso_attempts").await, 1);
    Ok(())
}

/// `start` is behind the sign-in rate limiter, per address.
#[tokio::test]
async fn start_is_rate_limited() -> anyhow::Result<()> {
    let h = harness_with(
        Mfa::amr_otp(),
        RateLimits {
            sign_in: RateLimitPolicy {
                attempts: 3,
                window_seconds: 300,
            },
            ..RateLimits::default()
        },
        None,
        true,
    )
    .await?;
    let mut statuses = Vec::new();
    for _ in 0..4 {
        statuses.push(h.post_form("/dash/v1/staff/sso/start", None, &[]).await?.0);
    }
    assert_eq!(statuses, [200, 200, 200, 429], "{statuses:?}");
    Ok(())
}

/// A deployment with no `staff_auth.sso` offers none: the status says so and
/// `start` and `callback` are the honest 404.
#[tokio::test]
async fn an_unconfigured_deployment_offers_no_sso() -> anyhow::Result<()> {
    let h = harness_with(Mfa::amr_otp(), RateLimits::default(), None, false).await?;

    let (status, body) = h.get_json("/dash/v1/staff/sso", None, None).await?;
    assert_eq!(
        (status, field(&body, "enabled")),
        (200, &Value::Bool(false)),
        "{body}"
    );
    let (status, _) = h.post_form("/dash/v1/staff/sso/start", None, &[]).await?;
    assert_eq!(status, 404);
    let (status, _) = h
        .post_form(
            "/dash/v1/staff/sso/callback",
            Some("x"),
            &[("code", "c"), ("state", "s")],
        )
        .await?;
    assert_eq!(status, 404);
    Ok(())
}

/// Discovery whose `issuer` is not the configured one byte for byte is
/// refused: no flow starts, nothing is written.
#[tokio::test]
async fn a_discovery_document_for_another_issuer_is_refused() -> anyhow::Result<()> {
    let h = harness_with(
        Mfa::amr_otp(),
        RateLimits::default(),
        Some("https://accounts.elsewhere.test"),
        true,
    )
    .await?;
    let (status, body) = h.post_form("/dash/v1/staff/sso/start", None, &[]).await?;
    assert_eq!(status, 401, "{body}");
    assert_eq!(h.count("SELECT count(*) FROM staff_sso_attempts").await, 0);
    Ok(())
}

/// Configuring SSO leaves password sign-in mounted (the break-glass path): a
/// wrong password is the same `401` as ever, not a `404`.
#[tokio::test]
async fn configuring_sso_leaves_password_sign_in_mounted() -> anyhow::Result<()> {
    let h = harness().await?;
    let (status, body) = h
        .post_form(
            "/dash/v1/staff/login",
            None,
            &[("email", "nobody@example.test"), ("password", "whatever")],
        )
        .await?;
    assert_eq!(status, 401, "{body}");
    Ok(())
}

/// The provider's tokens and the sealed verifier never reach a log or a
/// response body: the callback's answer holds a session token and nothing
/// the provider sent.
#[tokio::test]
async fn nothing_the_provider_sent_reaches_a_response() -> anyhow::Result<()> {
    let h = harness().await?;
    let flow = h.begin().await?;
    let token = h.idp.sign(
        &h.claims(&flow, "kc-user-1", "ada@example.test"),
        Some("idp-key-1"),
    );
    let (_, body) = h.finish(&flow, "code-1", &token).await?;
    let rendered = body.to_string();
    for secret in [
        token.as_str(),
        "IDP-ACCESS-TOKEN",
        "code-1",
        SSO_CLIENT_SECRET,
    ] {
        assert!(!rendered.contains(secret), "{rendered}");
    }
    let stored: String = sqlx::query(
        "SELECT string_agg(state_hash || nonce_hash || code_verifier, '') FROM staff_sso_attempts",
    )
    .fetch_one(&h.pool)
    .await
    .map(|row| row.get::<Option<String>, _>(0).unwrap_or_default())?;
    assert!(!stored.contains(&flow.nonce) && !stored.contains(&flow.state));
    Ok(())
}
