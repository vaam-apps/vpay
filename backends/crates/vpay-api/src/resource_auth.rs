//! Resource-server JWT validation for vpay's two protected HTTP surfaces.
//!
//! `docs/api/README.md` defines three surfaces with three different
//! protections; this module builds the validation layer for the two that
//! need one — `/v1` (merchant, `client_credentials` + `private_key_jwt`) and
//! `/dash/v1` (dashboard, a staff OIDC session, one read-only scope). See
//! [ADR-0009](../../../docs/adr/0009-dashboard-oidc-provider.md) (vpay runs
//! its own Authkestra OP) and
//! [ADR-0010](../../../docs/adr/0010-merchant-auth-private-key-jwt.md)
//! (merchant auth).
//!
//! STATUS: validation only, not wired to a route. No `/v1/*` or `/dash/v1/*`
//! route exists in `lib.rs`'s router — mounting them, and everything that
//! needs (a live signing key, a client registry, token issuance), is later
//! work. This module exists so that work can drop an
//! [`AuthenticatedMerchant`]/[`AuthenticatedDashboard`] extractor onto a
//! real handler without also having to design the auth boundary.
//!
//! ## Validation is local, not a network round trip per request
//!
//! [`authkestra_resource::jwt::JwksCache`] fetches the JWKS once and caches
//! it (`jwks_refresh_interval`, below); every call after that looks the key
//! up by `kid` from memory and verifies the signature locally with
//! `jsonwebtoken`. Confirmed by reading `authkestra-resource-0.3.4/src/jwt.rs`,
//! and re-confirmed unchanged at the current `0.7.1` pin:
//! `JwksCache::get_key` only calls `self.refresh()` (an HTTP GET) on a cache
//! miss or once the TTL has elapsed, never on every `validate_jwt_generic`
//! call. That is what makes this safe to put in front of a payment-processing
//! route.
//!
//! ## A sharp edge in `jsonwebtoken`'s default audience validation
//!
//! `jsonwebtoken::Validation::validate_aud` defaults to `true`, but the
//! check it gates only runs *if the token has an `aud` claim at all* —
//! confirmed by reading `jsonwebtoken-11.0.0/src/validation.rs`: the
//! doc comment on `validate_aud` itself says "Validation only happens if
//! `aud` claim is present", and the `_ => {}` fallthrough arm of `validate`'s
//! `match (claims.aud, options.aud.as_ref())` proves it — a token with no
//! `aud` claim reaches that arm and passes regardless of what
//! `set_audience` was told. A token minted with no audience at all would
//! therefore sail through unchecked, which is exactly the kind of ambiguity
//! this module is required to fail closed on (a missing claim, not merely a
//! wrong one). The fix here is not to hand-roll audience comparison:
//! [`JwtValidator::new`] calls `set_required_spec_claims(&["exp", "aud",
//! "iss"])`, which makes the `aud` claim's mere *presence* mandatory — a
//! token with no audience is rejected as a missing required claim before the
//! comparison logic ever runs — and `set_audience` continues to do the real
//! membership check with the library's own (tested) logic. Covered by
//! `a_token_with_no_audience_claim_at_all_is_rejected`, below.
//!
//! ## `authkestra_resource::jwt::JwtStrategy` was deliberately not used here
//!
//! `JwtStrategy<I>`'s `cache` and `validation` fields are both private with
//! no accessor — confirmed by reading `jwt.rs`: neither field is `pub`, and
//! no getter or setter method exists for either. Once a `JwtStrategy` is
//! built via `ValidationConfig`/`ValidationConfigBuilder`, nothing outside
//! the crate can inspect or adjust its `Validation` afterwards. That is a
//! real limitation — a sibling project had to work around it by setting
//! `validation.validate_aud = false` and hand-rolling the audience check
//! itself — but it does not bite this module, because this module never
//! constructs a `JwtStrategy` at all. It calls `JwksCache` and the free
//! function `validate_jwt_generic` directly (both `pub`) and builds its own
//! `jsonwebtoken::Validation`, so every field — including `validate_aud` and
//! `required_spec_claims` — stays under this module's control from
//! construction onward. `ValidationConfigBuilder` does expose `.audience()`/
//! `.audiences()`, so the *audience* half of the sibling project's problem
//! would not necessarily recur through `JwtStrategy` either — but the
//! `required_spec_claims` fix above has no equivalent builder method at all,
//! which alone would have forced hand-rolling (or living with the gap) had
//! `JwtStrategy` been used.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use authkestra_resource::jwt::{JwksCache, ValidationError, validate_jwt_generic};
use axum::extract::{FromRef, FromRequestParts};
use axum::http::header;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use jsonwebtoken::{Algorithm, Validation};
use serde::Deserialize;
use vpay_core::{Category, Classify};

use crate::ApiError;

/// Which of vpay's two protected surfaces a token was minted for. vpay runs
/// one OP ([ADR-0009]) issuing tokens for both off one JWKS, so the audience
/// claim is the only thing that separates them — which is exactly why every
/// [`JwtValidator`] is pinned to exactly one and never both.
///
/// [ADR-0009]: ../../../docs/adr/0009-dashboard-oidc-provider.md
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// `/v1`, the merchant API — `client_credentials` + `private_key_jwt`.
    Merchant,
    /// `/dash/v1`, the staff dashboard — an OIDC session, one read-only scope.
    Dashboard,
}

impl Surface {
    /// The `aud` value a token must carry to be accepted on this surface.
    ///
    /// Provisional: no ADR fixes this exact string, because token issuance
    /// has not been built yet (see the module doc — issuance is later
    /// work). What this module's tests prove is that the two values differ
    /// and are enforced, not that this exact spelling is final; whichever
    /// agent wires up issuance should mint tokens whose `aud` matches this
    /// function rather than inventing a second, drifting constant.
    #[must_use]
    pub fn audience(self) -> &'static str {
        match self {
            Surface::Merchant => "vpay:v1",
            Surface::Dashboard => "vpay:dash/v1",
        }
    }
}

/// The claims a handler actually needs out of a validated token.
/// Deliberately narrower than the full JWT: a handler that needs something
/// else should have it added here explicitly, not reach into a raw claims
/// map that could silently drift from what was actually validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceClaims {
    /// `sub` on a `client_credentials`/machine token: the OAuth2 client that
    /// authenticated. Not a secret — safe to log or attach to a span.
    pub client_id: String,
    /// The token's `scope` claim, space-split per RFC 6749 §3.3. Empty if
    /// the token carried no `scope` claim at all.
    pub scope: Vec<String>,
}

impl ResourceClaims {
    /// Whether the token was granted the given scope.
    #[must_use]
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scope.iter().any(|s| s == scope)
    }
}

/// Wire shape decoded off the token. Private: handlers see [`ResourceClaims`]
/// and nothing else — this type exists only because `jsonwebtoken::decode`
/// needs a concrete `Deserialize` target, and it deliberately carries no more
/// than `ResourceClaims` re-exposes.
#[derive(Deserialize)]
struct RawClaims {
    sub: String,
    #[serde(default)]
    scope: Option<String>,
}

impl From<RawClaims> for ResourceClaims {
    fn from(raw: RawClaims) -> Self {
        ResourceClaims {
            client_id: raw.sub,
            scope: raw
                .scope
                .map(|scope| scope.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
        }
    }
}

/// Every way a bearer token can fail to authenticate a request, collapsed
/// into the Stripe-shaped envelope [`crate::error_envelope`] already defines.
/// Deliberately generic about *why* signature, expiry, audience, issuer or
/// `kid` validation failed: this fails closed without becoming an oracle a
/// caller could use to probe which specific check tripped.
///
/// A leaf error in ADR-0011's sense: it classifies itself once (below) and
/// the HTTP boundary derives status, `type`, `code` and message from that.
/// The `Display` texts are the operator-facing half and are what reaches a
/// log; the merchant-facing half is [`Classify::public_message`], and the
/// two are deliberately not the same strings.
#[derive(Debug, thiserror::Error)]
pub enum AuthRejection {
    /// No `Authorization` header at all.
    #[error("no Authorization header was presented")]
    MissingHeader,
    /// Present but not a well-formed `Bearer <token>` value.
    #[error("the Authorization header was not a well-formed `Bearer <token>` value")]
    MalformedHeader,
    /// Present and well-formed, but the token itself does not validate: bad
    /// signature, wrong or missing audience, wrong issuer, expired,
    /// not-yet-valid, or an unrecognized `kid`.
    ///
    /// The `Display` says no more than that on purpose. The underlying
    /// `ValidationError` is dropped at the `From` impl above rather than
    /// kept as a `#[source]`: keeping it would put "invalid audience" vs.
    /// "expired" into a log the same request could provoke at will, which is
    /// the oracle this type exists to avoid — and unlike a database error,
    /// the detail is not something an operator needs to fix anything.
    #[error("the bearer token did not validate")]
    InvalidToken,
}

impl Classify for AuthRejection {
    /// One category for all three. Which check tripped is not the caller's
    /// business (see the type's own doc comment), and
    /// [`Category::Authentication`] is what turns that into 401 +
    /// `authentication_error` at every boundary at once.
    fn category(&self) -> Category {
        Category::Authentication
    }

    /// Per-variant, unlike the message. A code is a stable identifier an SDK
    /// branches on, and these three are actionable in different ways by the
    /// *legitimate* caller — "you sent no header" and "your token expired"
    /// need different fixes. They reveal nothing about the token's contents,
    /// which is where the oracle risk actually lives; `InvalidToken` is the
    /// single code behind which every validation failure hides.
    fn code(&self) -> &'static str {
        match self {
            Self::MissingHeader => "missing_bearer_token",
            Self::MalformedHeader => "malformed_authorization_header",
            Self::InvalidToken => "invalid_token",
        }
    }

    /// The exact sentences this endpoint has answered since OP-3, kept
    /// verbatim (pinned byte-for-byte in `error.rs`'s tests) rather than
    /// collapsed into `Category::Authentication`'s generic message: the
    /// first two tell a caller how to fix a request that never carried a
    /// token, which the generic sentence — written for a token that failed
    /// validation — does not.
    fn public_message(&self) -> String {
        match self {
            Self::MissingHeader => {
                "No Authorization header was provided. Send an OAuth2 access token as 'Authorization: Bearer <token>'."
            }
            Self::MalformedHeader => {
                "The Authorization header was present but was not a well-formed 'Bearer <token>' value."
            }
            Self::InvalidToken => Category::Authentication.generic_message(),
        }
        .to_owned()
    }
}

impl From<ValidationError> for AuthRejection {
    fn from(_error: ValidationError) -> Self {
        // Every ValidationError variant collapses to the same public
        // rejection on purpose — see this type's own doc comment.
        AuthRejection::InvalidToken
    }
}

impl IntoResponse for AuthRejection {
    /// Delegates to [`ApiError`], which is the crate's only envelope
    /// renderer (ADR-0011). An extractor rejection therefore cannot drift
    /// from what a handler returning `Err(ApiError::Auth(..))` produces —
    /// they are the same code path, and the identical bytes are pinned by
    /// `every_auth_rejection_is_byte_for_byte_what_it_was_before_api_error`.
    fn into_response(self) -> Response {
        ApiError::from(self).into_response()
    }
}

/// Extracts a bearer token from the `Authorization` header, rejecting a
/// missing or malformed header rather than falling back to an
/// unauthenticated path.
fn extract_bearer_token(parts: &Parts) -> Result<&str, AuthRejection> {
    let value = parts
        .headers
        .get(header::AUTHORIZATION)
        .ok_or(AuthRejection::MissingHeader)?
        .to_str()
        .map_err(|_error| AuthRejection::MalformedHeader)?;

    let token = value
        .strip_prefix("Bearer ")
        .ok_or(AuthRejection::MalformedHeader)?
        .trim();

    if token.is_empty() {
        return Err(AuthRejection::MalformedHeader);
    }

    Ok(token)
}

/// Validates bearer tokens for exactly one [`Surface`]: one JWKS cache, one
/// required audience, one required issuer. Cheap to clone (the cache is
/// `Arc`-shared) so it can live in axum state alongside a database pool.
#[derive(Clone)]
pub struct JwtValidator {
    cache: Arc<JwksCache>,
    validation: Validation,
}

impl fmt::Debug for JwtValidator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `JwksCache` has no `Debug` impl of its own; the validation policy
        // (audience, issuer, required claims) is the useful, non-sensitive
        // half to show.
        f.debug_struct("JwtValidator")
            .field("validation", &self.validation)
            .finish_non_exhaustive()
    }
}

impl JwtValidator {
    /// `jwks_url` is polled at most once per `jwks_refresh_interval` (plus
    /// once more on an unrecognized `kid`, to tolerate an in-flight
    /// rotation) — see the module doc for why this is not a per-request
    /// network call. `require_kid(true)` on the underlying cache: a token
    /// presented with no `kid` header is rejected rather than silently
    /// matched against the first key in the JWKS, which matters the moment
    /// the JWKS ever holds more than one key during a rotation window.
    #[must_use]
    pub fn new(
        jwks_url: impl Into<String>,
        jwks_refresh_interval: Duration,
        issuer: impl Into<String>,
        surface: Surface,
    ) -> Self {
        let cache = JwksCache::new(jwks_url.into(), jwks_refresh_interval).require_kid(true);

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[issuer.into()]);
        validation.set_audience(&[surface.audience()]);
        // See the module doc's "sharp edge" section: `aud` is only checked
        // by `validate_aud` when the claim is present at all, so its
        // presence has to be required explicitly, separately from its value.
        validation.set_required_spec_claims(&["exp", "aud", "iss"]);

        Self {
            cache: Arc::new(cache),
            validation,
        }
    }

    /// Validates signature, expiry, issuer and audience, and returns the
    /// claims a handler needs. Fails closed: any ambiguity (unknown `kid`,
    /// missing claim, expired, wrong audience or issuer, bad signature) is
    /// `Err`, never a best-effort `Ok`.
    pub async fn validate(&self, token: &str) -> Result<ResourceClaims, AuthRejection> {
        validate_jwt_generic::<RawClaims>(token, &self.cache, &self.validation)
            .await
            .map(ResourceClaims::from)
            .map_err(AuthRejection::from)
    }
}

/// Newtype so a single router state can hold two [`JwtValidator`]s
/// (merchant and dashboard) and axum's `FromRef` can still tell them apart.
#[derive(Debug, Clone)]
pub struct MerchantJwtValidator(pub JwtValidator);

/// See [`MerchantJwtValidator`] — the `/dash/v1` counterpart.
#[derive(Debug, Clone)]
pub struct DashboardJwtValidator(pub JwtValidator);

/// Extractor for `/v1` handlers:
///
/// ```ignore
/// async fn create_payment_intent(
///     AuthenticatedMerchant(claims): AuthenticatedMerchant,
///     // ... other extractors ...
/// ) -> impl IntoResponse {
///     // claims.client_id, claims.scope
/// }
/// ```
///
/// Requires the router's state to implement `FromRef<S> for MerchantJwtValidator`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedMerchant(pub ResourceClaims);

/// Extractor for `/dash/v1` handlers. See [`AuthenticatedMerchant`]; requires
/// `FromRef<S> for DashboardJwtValidator` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedDashboard(pub ResourceClaims);

impl<S> FromRequestParts<S> for AuthenticatedMerchant
where
    S: Send + Sync,
    MerchantJwtValidator: FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let token = extract_bearer_token(parts)?;
        let validator = MerchantJwtValidator::from_ref(state);
        validator.0.validate(token).await.map(AuthenticatedMerchant)
    }
}

impl<S> FromRequestParts<S> for AuthenticatedDashboard
where
    S: Send + Sync,
    DashboardJwtValidator: FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let token = extract_bearer_token(parts)?;
        let validator = DashboardJwtValidator::from_ref(state);
        validator
            .0
            .validate(token)
            .await
            .map(AuthenticatedDashboard)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{LazyLock, Once};
    use std::time::{SystemTime, UNIX_EPOCH};

    use authkestra_resource::jwt::Jwk;
    use axum::body::Body;
    use axum::http::Request;
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::{Json, Router};
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use jsonwebtoken::{EncodingKey, Header};
    use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
    use rsa::traits::PublicKeyParts;
    use serde_json::{Value, json};
    use tower::ServiceExt as _;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const ISSUER: &str = "https://op.vpay.test";

    /// A single RSA keypair, generated once and shared across every test
    /// that does not specifically need a *different* key (the
    /// wrong-signing-key test generates its own second keypair). RSA
    /// generation is the slow part of this test module; sharing keeps the
    /// suite fast without making any test's signature verification fake.
    static KEYPAIR: LazyLock<(EncodingKey, Jwk)> = LazyLock::new(|| generate_keypair("test-key-1"));

    static CRYPTO_PROVIDER_INSTALLED: Once = Once::new();

    /// `authkestra_resource::jwt::Jwks::fetch` builds a fresh
    /// `reqwest::Client` on every JWKS fetch, which eagerly constructs a
    /// rustls TLS config at build time — even for a plain-HTTP wiremock
    /// target — and panics without a process-default `CryptoProvider`
    /// installed (root `Cargo.toml`'s own comment on the `authkestra-*`
    /// pins names this exact prerequisite). Test-only setup: production
    /// code in this crate never calls this; whichever agent wires this
    /// module into a real binary must make the same call once in `main()`,
    /// before the first JWKS fetch.
    fn ensure_crypto_provider_installed() {
        CRYPTO_PROVIDER_INSTALLED.call_once(|| {
            rustls::crypto::ring::default_provider()
                .install_default()
                .ok();
        });
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the unix epoch")
            .as_secs()
    }

    /// Generates a real 2048-bit RSA keypair and the JWK describing its
    /// public half, so every test in this module signs and verifies against
    /// actual cryptographic material — never a stubbed decoder.
    fn generate_keypair(kid: &str) -> (EncodingKey, Jwk) {
        let mut rng = rand::rngs::OsRng;
        let private_key =
            rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa key generation succeeds");
        let public_key = private_key.to_public_key();

        let pem = private_key
            .to_pkcs1_pem(LineEnding::LF)
            .expect("pkcs1 pem encoding succeeds");
        let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes())
            .expect("jsonwebtoken accepts a pkcs1 rsa private-key pem");

        let jwk = Jwk {
            kid: Some(kid.to_string()),
            kty: "RSA".to_string(),
            alg: Some("RS256".to_string()),
            n: Some(URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be())),
            e: Some(URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be())),
            // `authkestra-engine` 0.4.0 (PR #190) widened `Jwk` with two more
            // fields to also represent the OKP/Ed25519 JWK shape (RFC 8037)
            // alongside the existing RSA one. Both are `None` for every RSA
            // key this test module generates — see the struct's own doc
            // comment on `authkestra_engine::token::jwk::Jwk`.
            crv: None,
            x: None,
        };

        (encoding_key, jwk)
    }

    /// Signs `claims` with `encoding_key`, stamping `kid` onto the header —
    /// exactly what `JwksCache`/`validate_jwt_generic` key their lookup on.
    fn mint_token(encoding_key: &EncodingKey, kid: &str, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        jsonwebtoken::encode(&header, claims, encoding_key).expect("signing succeeds")
    }

    fn valid_claims(aud: &str, sub: &str, scope: &str) -> Value {
        json!({
            "iss": ISSUER,
            "aud": aud,
            "sub": sub,
            "scope": scope,
            "exp": now_secs() + 300,
        })
    }

    /// Serves `{"keys": [jwk]}` at `/jwks.json` from a real local HTTP
    /// server (wiremock), so `JwksCache` performs a real fetch — never an
    /// injected/faked `Jwks` value. Per ADR-0006 (blessed by this crate's
    /// own AGENTS.md as the pattern for a stubbed dependency in tests).
    async fn jwks_server(jwk: &Jwk) -> MockServer {
        ensure_crypto_provider_installed();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [jwk] })))
            .mount(&server)
            .await;
        server
    }

    fn merchant_validator(jwks_url: String) -> JwtValidator {
        JwtValidator::new(
            jwks_url,
            Duration::from_secs(300),
            ISSUER,
            Surface::Merchant,
        )
    }

    fn dashboard_validator(jwks_url: String) -> JwtValidator {
        JwtValidator::new(
            jwks_url,
            Duration::from_secs(300),
            ISSUER,
            Surface::Dashboard,
        )
    }

    #[tokio::test]
    async fn a_validly_signed_unexpired_correct_audience_token_is_accepted_and_its_claims_surface()
    {
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let validator = merchant_validator(format!("{}/jwks.json", server.uri()));

        let token = mint_token(
            encoding_key,
            "test-key-1",
            &valid_claims(
                Surface::Merchant.audience(),
                "merchant-123",
                "payment_intents:write refunds:write",
            ),
        );

        let claims = validator
            .validate(&token)
            .await
            .expect("a validly-signed, unexpired, correct-audience token is accepted");

        assert_eq!(claims.client_id, "merchant-123");
        assert_eq!(
            claims.scope,
            vec![
                "payment_intents:write".to_string(),
                "refunds:write".to_string()
            ]
        );
        assert!(claims.has_scope("refunds:write"));
        assert!(!claims.has_scope("dash:read"));
    }

    #[tokio::test]
    async fn a_token_signed_by_a_different_key_is_rejected() {
        let (_good_key, good_jwk) = &*KEYPAIR;
        // Same `kid` the JWKS actually advertises, but signed by an
        // entirely different private key — proves the JWKS lookup finding
        // the "right" key by name is not enough; the signature itself must
        // verify against that key's public half.
        let (forged_key, _forged_jwk) = generate_keypair("test-key-1");

        let server = jwks_server(good_jwk).await;
        let validator = merchant_validator(format!("{}/jwks.json", server.uri()));

        let token = mint_token(
            &forged_key,
            "test-key-1",
            &valid_claims(
                Surface::Merchant.audience(),
                "merchant-123",
                "payment_intents:write",
            ),
        );

        let error = validator
            .validate(&token)
            .await
            .expect_err("a token signed by a different key must be rejected");
        assert!(matches!(error, AuthRejection::InvalidToken));
    }

    #[tokio::test]
    async fn an_expired_token_is_rejected() {
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let validator = merchant_validator(format!("{}/jwks.json", server.uri()));

        let claims = json!({
            "iss": ISSUER,
            "aud": Surface::Merchant.audience(),
            "sub": "merchant-123",
            "scope": "",
            "exp": now_secs().saturating_sub(3600),
        });
        let token = mint_token(encoding_key, "test-key-1", &claims);

        let error = validator
            .validate(&token)
            .await
            .expect_err("an expired token must be rejected");
        assert!(matches!(error, AuthRejection::InvalidToken));
    }

    #[tokio::test]
    async fn a_merchant_audience_token_is_rejected_by_the_dashboard_validator() {
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        // Minted with the *merchant* audience, presented to the *dashboard*
        // validator — the separation the whole module exists to enforce.
        let validator = dashboard_validator(format!("{}/jwks.json", server.uri()));

        let token = mint_token(
            encoding_key,
            "test-key-1",
            &valid_claims(Surface::Merchant.audience(), "merchant-123", ""),
        );

        let error = validator
            .validate(&token)
            .await
            .expect_err("a merchant-audience token must not validate on the dashboard surface");
        assert!(matches!(error, AuthRejection::InvalidToken));
    }

    #[tokio::test]
    async fn a_dashboard_audience_token_is_rejected_by_the_merchant_validator() {
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        // The mirror image of the test above: proven in both directions,
        // not assumed from one.
        let validator = merchant_validator(format!("{}/jwks.json", server.uri()));

        let token = mint_token(
            encoding_key,
            "test-key-1",
            &valid_claims(
                Surface::Dashboard.audience(),
                "staff-oidc-session",
                "dash:read",
            ),
        );

        let error = validator
            .validate(&token)
            .await
            .expect_err("a dashboard-audience token must not validate on the merchant surface");
        assert!(matches!(error, AuthRejection::InvalidToken));
    }

    #[tokio::test]
    async fn a_dashboard_audience_token_is_accepted_by_the_dashboard_validator() {
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let validator = dashboard_validator(format!("{}/jwks.json", server.uri()));

        let token = mint_token(
            encoding_key,
            "test-key-1",
            &valid_claims(
                Surface::Dashboard.audience(),
                "staff-oidc-session",
                "dash:read",
            ),
        );

        let claims = validator
            .validate(&token)
            .await
            .expect("a correctly-audienced dashboard token is accepted by the dashboard validator");
        assert_eq!(claims.client_id, "staff-oidc-session");
        assert_eq!(claims.scope, vec!["dash:read".to_string()]);
    }

    #[tokio::test]
    async fn a_token_with_no_audience_claim_at_all_is_rejected() {
        // The sharp edge documented in this module's own doc comment:
        // jsonwebtoken's `validate_aud` only runs its check when `aud` is
        // present at all. A token that omits `aud` entirely must still be
        // rejected, not silently accepted because there was nothing to
        // compare against.
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let validator = merchant_validator(format!("{}/jwks.json", server.uri()));

        let claims = json!({
            "iss": ISSUER,
            "sub": "merchant-123",
            "exp": now_secs() + 300,
        });
        let token = mint_token(encoding_key, "test-key-1", &claims);

        let error = validator
            .validate(&token)
            .await
            .expect_err("a token with no audience claim at all must be rejected");
        assert!(matches!(error, AuthRejection::InvalidToken));
    }

    #[tokio::test]
    async fn an_unknown_kid_is_rejected_rather_than_falling_back_to_any_key() {
        let (encoding_key, jwk) = &*KEYPAIR;
        // The JWKS only ever advertises "test-key-1".
        let server = jwks_server(jwk).await;
        let validator = merchant_validator(format!("{}/jwks.json", server.uri()));

        // Signed with a real (matching) private key, but the header claims
        // a `kid` the JWKS has never heard of.
        let token = mint_token(
            encoding_key,
            "does-not-exist",
            &valid_claims(Surface::Merchant.audience(), "merchant-123", ""),
        );

        let error = validator
            .validate(&token)
            .await
            .expect_err("an unknown kid must be rejected, not matched against any available key");
        assert!(matches!(error, AuthRejection::InvalidToken));
    }

    // --- extractor + Stripe-shaped error envelope, over real HTTP ---

    #[derive(Clone)]
    struct TestState {
        merchant: MerchantJwtValidator,
    }

    impl FromRef<TestState> for MerchantJwtValidator {
        fn from_ref(state: &TestState) -> Self {
            state.merchant.clone()
        }
    }

    async fn probe(AuthenticatedMerchant(claims): AuthenticatedMerchant) -> Json<Value> {
        Json(json!({ "client_id": claims.client_id, "scope": claims.scope }))
    }

    fn test_app(validator: JwtValidator) -> Router {
        Router::new()
            .route("/probe", get(probe))
            .with_state(TestState {
                merchant: MerchantJwtValidator(validator),
            })
    }

    async fn envelope_of(response: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("reading the response body succeeds");
        serde_json::from_slice(&bytes).expect("the body is valid JSON")
    }

    /// `docs/status.md`'s own house style avoids `clippy::indexing_slicing`
    /// entirely rather than allowing it locally — this mirrors that for
    /// walking a JSON error envelope without `value["key"]` indexing.
    fn error_field<'a>(envelope: &'a Value, field: &str) -> Option<&'a str> {
        envelope.get("error")?.get(field)?.as_str()
    }

    #[tokio::test]
    async fn a_missing_authorization_header_produces_the_stripe_shaped_envelope_with_401() {
        let (_encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let app = test_app(merchant_validator(format!("{}/jwks.json", server.uri())));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/probe")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router does not fail to serve");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = envelope_of(response).await;
        assert_eq!(error_field(&body, "type"), Some("authentication_error"));
        assert_eq!(error_field(&body, "code"), Some("missing_bearer_token"));
        assert!(error_field(&body, "message").is_some());
    }

    #[tokio::test]
    async fn a_malformed_authorization_header_produces_the_stripe_shaped_envelope_with_401() {
        let (_encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let app = test_app(merchant_validator(format!("{}/jwks.json", server.uri())));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/probe")
                    // Not a "Bearer <token>" value at all.
                    .header("Authorization", "Basic dXNlcjpwYXNz")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router does not fail to serve");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = envelope_of(response).await;
        assert_eq!(error_field(&body, "type"), Some("authentication_error"));
        assert_eq!(
            error_field(&body, "code"),
            Some("malformed_authorization_header")
        );
    }

    #[tokio::test]
    async fn a_valid_bearer_token_reaches_the_handler_with_claims_attached() {
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let app = test_app(merchant_validator(format!("{}/jwks.json", server.uri())));

        let token = mint_token(
            encoding_key,
            "test-key-1",
            &valid_claims(
                Surface::Merchant.audience(),
                "merchant-123",
                "payment_intents:write",
            ),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/probe")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router does not fail to serve");

        assert_eq!(response.status(), StatusCode::OK);
        let body = envelope_of(response).await;
        assert_eq!(
            body.get("client_id").and_then(Value::as_str),
            Some("merchant-123")
        );
        assert_eq!(body.get("scope"), Some(&json!(["payment_intents:write"])));
    }
}
