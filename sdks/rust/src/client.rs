//! [`Client`] — the token-caching, retrying HTTP client for `/v1`, built via
//! [`ClientBuilder`].
//!
//! `docs/flows/merchant-auth.md` §3-4 pins the caching and re-auth behaviour
//! this module implements: a token is reused until `expires_in` minus a
//! safety margin, concurrent callers share one in-flight token request
//! rather than each minting their own, and a `401` from a resource route is
//! answered with exactly one re-auth-and-retry rather than an automatic
//! retry loop.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::auth::{self, CLIENT_ASSERTION_TYPE_JWT_BEARER, Credentials};
use crate::error::{ConfigError, Error};
use crate::form::FormValue;
use crate::resources::{
    BalanceResource, EventsResource, PaymentIntentsResource, RefundsResource, RequestOptions,
};

/// The `aud` value `/v1` access tokens must carry
/// (`vpay_api::resource_auth::Surface::Merchant::audience()`, currently
/// `"vpay:v1"`). Requested by default on every token exchange — see
/// `docs/flows/merchant-auth.md`: "`audience=vpay:v1` is provisional and
/// load-bearing".
pub const DEFAULT_AUDIENCE: &str = "vpay:v1";

/// How much a cached token's usable lifetime is shortened by, so a request
/// never starts against a token that expires mid-flight. `docs/flows/
/// merchant-auth.md` §3: "margin: 30s, or half of `expires_in` for very
/// short TTLs — integer arithmetic only".
const MAX_MARGIN_SECS: u64 = 30;

/// The maximum bytes of a non-JSON-envelope error body kept in
/// [`Error::UnexpectedResponse`]. An unbounded prefix would let a
/// misbehaving upstream hand this crate an unbounded amount of memory to
/// hold in an error value.
const BODY_PREFIX_LIMIT: usize = 500;

/// Mozilla's CA bundle as a rustls root store — see
/// [`rustls_client_config`] for why the roots are vendored rather than read
/// from the platform.
fn vendored_root_store() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

/// The TLS configuration every request in this crate goes out over: rustls
/// with the `ring` provider and Mozilla's vendored root bundle.
///
/// Built here rather than left to `reqwest::ClientBuilder`'s own TLS setup,
/// for two reasons that both bite a *library*:
///
/// 1. **No panic.** reqwest 0.13 is pinned with `rustls-no-provider` (its
///    `rustls` feature would select the `aws-lc-rs` provider, which
///    `deny.toml` bans — two providers in one process make
///    `rustls::crypto::CryptoProvider::install_default()` panic). Under that
///    feature reqwest's own builder panics if no process-wide default
///    provider was installed. This crate runs inside a merchant's process; it
///    may not panic there, and it may not install a process-wide default on
///    that process's behalf either — that is the application's decision, and
///    an SDK quietly making it would break a merchant who wanted a different
///    provider. Handing reqwest a finished `ClientConfig` takes its
///    `BuiltRustls` path, which never consults the process default at all.
/// 2. **Deterministic roots.** reqwest 0.13 dropped its vendored-roots
///    feature for `rustls-platform-verifier` (the OS trust store). vpay's
///    runtime image is `FROM scratch` (`docs/adr/0004`) and has no trust
///    store, and Node's SDK ships its own CA bundle rather than reading the
///    OS one — so vendored roots keep both the deployment target and the two
///    SDKs consistent.
///
/// ALPN is set explicitly because reqwest only sets it on the path *not*
/// taken here: without these two protocol names a TLS connection would
/// silently never negotiate HTTP/2.
fn rustls_client_config() -> Result<rustls::ClientConfig, ConfigError> {
    let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| ConfigError::Http(e.to_string()))?
    .with_root_certificates(vendored_root_store())
    .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

fn user_agent() -> String {
    format!("vpay-sdk-rust/{}", env!("CARGO_PKG_VERSION"))
}

/// How many bytes of a UTF-8 sequence `lead` introduces. `1` for ASCII and
/// for any byte that cannot start a sequence at all — such a byte is not a
/// truncated character, it is simply invalid, and the lossy decode below
/// renders it as `U+FFFD` whether or not the cut fell here.
fn utf8_sequence_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// Backs the cut off the tail of a *truncated* multi-byte character, so the
/// prefix ends on a character boundary.
///
/// Parity with `sdks/nodejs/src/errors.ts`'s `boundedBodyPrefix`, which
/// decodes with a streaming `TextDecoder` it never flushes: a character
/// straddling the cut is **dropped**, not emitted as `U+FFFD`. Without this,
/// a merchant reading `err.body_prefix` from the two SDKs would see the same
/// upstream body end differently, and a `U+FFFD` that is an artefact of the
/// truncation is indistinguishable from one that was really in the body.
fn trim_truncated_tail(bytes: &[u8]) -> &[u8] {
    // At most 3 continuation bytes can precede the lead byte of the longest
    // (4-byte) UTF-8 sequence.
    for back in 0..4usize {
        let Some(index) = bytes.len().checked_sub(back + 1) else {
            return bytes;
        };
        let Some(&byte) = bytes.get(index) else {
            return bytes;
        };
        if byte & 0b1100_0000 == 0b1000_0000 {
            continue; // a continuation byte; keep walking back to the lead
        }
        // `byte` leads the final sequence and `back + 1` of its bytes are
        // present. Complete: keep everything. Short: drop the whole partial
        // character.
        return if utf8_sequence_len(byte) > back + 1 {
            bytes.get(..index).unwrap_or(bytes)
        } else {
            bytes
        };
    }
    bytes
}

/// The first [`BODY_PREFIX_LIMIT`] **bytes** of `bytes`, cut on a character
/// boundary and lossily decoded.
///
/// The bound is on bytes, not `char`s: the point is to keep an unbounded
/// response body out of an error value and a log line, and a body of
/// multi-byte text is up to four times its character count on the wire.
///
/// Written with `.get(..)` rather than `&bytes[..limit]` even though `limit`
/// is provably in range: `clippy::indexing_slicing` is on workspace-wide and
/// this repository's house style answers it by not indexing at all rather
/// than by allowing the lint locally (see `vpay_api::resource_auth`'s test
/// helper, which says so).
fn bounded_prefix(bytes: &[u8]) -> String {
    let limit = bytes.len().min(BODY_PREFIX_LIMIT);
    let cut = bytes.get(..limit).unwrap_or(bytes);
    String::from_utf8_lossy(trim_truncated_tail(cut)).into_owned()
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct TokenErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct ApiErrorBody {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    code: Option<String>,
    message: String,
    #[serde(default)]
    param: Option<String>,
}

#[derive(Deserialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

/// A cached access token and the instant it stops being usable.
///
/// Deliberately holds no `Debug`/`Clone` derive that would put the token
/// itself somewhere a stray `{:?}` could print it — [`Client`]'s own
/// hand-written `Debug` never touches this type's fields.
struct CachedToken {
    access_token: String,
    valid_until: Instant,
}

impl CachedToken {
    /// `expires_in - margin`, where `margin = min(30, expires_in / 2)` —
    /// integer division, matching `docs/flows/merchant-auth.md` §3 exactly
    /// (workspace-wide `clippy::float_arithmetic = "deny"` rules out doing
    /// this any other way regardless).
    fn new(access_token: String, expires_in: u64) -> Self {
        let margin = MAX_MARGIN_SECS.min(expires_in / 2);
        let usable_secs = expires_in.saturating_sub(margin);
        Self {
            access_token,
            valid_until: Instant::now() + Duration::from_secs(usable_secs),
        }
    }

    fn is_valid(&self) -> bool {
        Instant::now() < self.valid_until
    }
}

struct Inner {
    http: reqwest::Client,
    credentials: Credentials,
    token_endpoint: String,
    resource_base: String,
    audience: String,
    scope: Option<String>,
    assertion_lifetime: Duration,
    /// Guards the cache *and* doubles as the single-flight lock: a refresh
    /// runs with the lock held across the `.await`, so concurrent callers
    /// block on the mutex rather than each minting and spending their own
    /// assertion `jti` for the same moment in time
    /// (`docs/flows/merchant-auth.md` §3).
    token_cache: AsyncMutex<Option<CachedToken>>,
}

/// A configured `/v1` API client. Cheap to clone — internally an `Arc`, so
/// every clone shares the same token cache and connection pool.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl fmt::Debug for Client {
    /// Hand-written, like [`Credentials`]'s own `Debug`: this must never
    /// print the private key (via `credentials`) or a live bearer token (via
    /// the cache) — see `tests/debug_redaction.rs`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("credentials", &self.inner.credentials)
            .field("token_endpoint", &self.inner.token_endpoint)
            .field("resource_base", &self.inner.resource_base)
            .field("audience", &self.inner.audience)
            .field("scope", &self.inner.scope)
            .field("assertion_lifetime", &self.inner.assertion_lifetime)
            .field("cached_token", &"[redacted]")
            .finish()
    }
}

impl Client {
    /// Starts building a client against `base_url` (e.g.
    /// `"https://api.vpay.example"`). See [`ClientBuilder`] for the defaults
    /// this derives (`docs/flows/merchant-auth.md`'s "Endpoint locations"
    /// table).
    #[must_use]
    pub fn builder(base_url: impl Into<String>) -> ClientBuilder {
        ClientBuilder::new(base_url)
    }

    /// `/v1/payment_intents`.
    #[must_use]
    pub fn payment_intents(&self) -> PaymentIntentsResource<'_> {
        PaymentIntentsResource { client: self }
    }

    /// `/v1/refunds`.
    #[must_use]
    pub fn refunds(&self) -> RefundsResource<'_> {
        RefundsResource { client: self }
    }

    /// `/v1/events`.
    #[must_use]
    pub fn events(&self) -> EventsResource<'_> {
        EventsResource { client: self }
    }

    /// `/v1/balance`.
    #[must_use]
    pub fn balance(&self) -> BalanceResource<'_> {
        BalanceResource { client: self }
    }

    fn resource_url(&self, path: &str) -> String {
        format!("{}{path}", self.inner.resource_base)
    }

    async fn get_token(&self) -> Result<String, Error> {
        let mut guard = self.inner.token_cache.lock().await;
        if let Some(cached) = guard.as_ref()
            && cached.is_valid()
        {
            return Ok(cached.access_token.clone());
        }
        // Held across this `.await`: the single-flight property depends on
        // every other concurrent caller blocking here rather than racing to
        // mint their own assertion.
        //
        // Emitted at `debug`, and naming only the endpoint and the client id:
        // the assertion and the access token are credentials, and a
        // structured logger will happily ship whatever it is handed to a
        // log aggregator (`docs/flows/merchant-auth.md`; see also
        // `tests/debug_redaction.rs`).
        tracing::debug!(
            token_endpoint = %self.inner.token_endpoint,
            client_id = %self.inner.credentials.client_id(),
            "no usable cached access token; performing the client_credentials exchange"
        );
        let fresh = self.fetch_token().await?;
        let token = fresh.access_token.clone();
        *guard = Some(fresh);
        Ok(token)
    }

    /// Discards the cached token — but **only if** the cache still holds the
    /// exact token that just got the `401`.
    ///
    /// A compare-and-swap, not a plain clear, because two concurrent callers
    /// can hold the same token and be refused a moment apart:
    ///
    /// 1. A and B both send with `tok_1`; both get a `401`.
    /// 2. A clears the cache and refreshes. Its refresh runs with the cache
    ///    mutex held (that is the single-flight property), so it finishes by
    ///    storing `tok_2`.
    /// 3. B's `401` arrives second. An unconditional clear here would throw
    ///    away `tok_2` — a token that is *valid and was never refused* —
    ///    and B would mint a third assertion, spend a third `jti`, and leave
    ///    any caller that had already read `tok_2` racing a cache that no
    ///    longer matches.
    ///
    /// Comparing first makes step 3 a no-op: B's `tok_1` is not what the
    /// cache holds, so B simply picks up `tok_2` and retries with it. The
    /// comparison is a plain `==`, not a constant-time one: both sides are
    /// this process's own cached copies of the same secret, so there is no
    /// attacker-supplied operand and nothing to leak by timing.
    async fn invalidate_if_current(&self, spent: &str) {
        let mut guard = self.inner.token_cache.lock().await;
        if guard
            .as_ref()
            .is_some_and(|cached| cached.access_token == spent)
        {
            *guard = None;
        }
    }

    async fn fetch_token(&self) -> Result<CachedToken, Error> {
        let assertion = auth::mint_client_assertion(
            &self.inner.credentials,
            &self.inner.token_endpoint,
            self.inner.assertion_lifetime,
        )?;

        let mut fields = vec![
            (
                "grant_type".to_string(),
                FormValue::from("client_credentials"),
            ),
            (
                "client_id".to_string(),
                FormValue::from(self.inner.credentials.client_id().to_string()),
            ),
            (
                "client_assertion_type".to_string(),
                FormValue::from(CLIENT_ASSERTION_TYPE_JWT_BEARER),
            ),
            ("client_assertion".to_string(), FormValue::from(assertion)),
            (
                "audience".to_string(),
                FormValue::from(self.inner.audience.as_str()),
            ),
        ];
        if let Some(scope) = &self.inner.scope {
            fields.push(("scope".to_string(), FormValue::from(scope.as_str())));
        }
        let body = crate::form::encode_form(&FormValue::Object(fields));

        let response = self
            .inner
            .http
            .post(&self.inner.token_endpoint)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, user_agent())
            .body(body)
            .send()
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        if status.is_success() {
            let token: TokenResponse =
                serde_json::from_slice(&bytes).map_err(|_| Error::UnexpectedResponse {
                    status: status.as_u16(),
                    body_prefix: bounded_prefix(&bytes),
                })?;
            Ok(CachedToken::new(token.access_token, token.expires_in))
        } else if let Ok(err) = serde_json::from_slice::<TokenErrorResponse>(&bytes) {
            Err(Error::TokenEndpoint {
                error: err.error,
                description: err.error_description,
            })
        } else {
            Err(Error::UnexpectedResponse {
                status: status.as_u16(),
                body_prefix: bounded_prefix(&bytes),
            })
        }
    }

    async fn execute(
        &self,
        method: Method,
        url: &str,
        body: Option<&str>,
        token: &str,
        idempotency_key: Option<&str>,
    ) -> Result<reqwest::Response, Error> {
        let mut request = self
            .inner
            .http
            .request(method, url)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, user_agent());

        if let Some(b) = body {
            request = request
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(b.to_string());
        }
        if let Some(key) = idempotency_key {
            request = request.header("Idempotency-Key", key);
        }

        request
            .send()
            .await
            .map_err(|e| Error::Transport(e.to_string()))
    }

    async fn decode_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, Error> {
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        if status.is_success() {
            serde_json::from_slice(&bytes).map_err(|_| Error::UnexpectedResponse {
                status: status.as_u16(),
                body_prefix: bounded_prefix(&bytes),
            })
        } else if let Ok(envelope) = serde_json::from_slice::<ApiErrorEnvelope>(&bytes) {
            Err(Error::Api {
                status: status.as_u16(),
                kind: envelope.error.kind,
                code: envelope.error.code,
                message: envelope.error.message,
                param: envelope.error.param,
            })
        } else {
            Err(Error::UnexpectedResponse {
                status: status.as_u16(),
                body_prefix: bounded_prefix(&bytes),
            })
        }
    }

    /// Sends one authenticated `/v1` request, re-authenticating and retrying
    /// exactly once on a `401` (`docs/flows/merchant-auth.md` §4). A second
    /// consecutive `401` is decoded and returned to the caller like any
    /// other error response — this function never retries more than once.
    async fn send_authenticated<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
        query: Option<String>,
        idempotency_key: Option<String>,
    ) -> Result<T, Error> {
        let mut url = self.resource_url(path);
        if let Some(q) = &query
            && !q.is_empty()
        {
            url.push('?');
            url.push_str(q);
        }

        let token = self.get_token().await?;
        let response = self
            .execute(
                method.clone(),
                &url,
                body.as_deref(),
                &token,
                idempotency_key.as_deref(),
            )
            .await?;

        if response.status() == StatusCode::UNAUTHORIZED {
            // Worth a log line even though it is handled: a merchant seeing
            // this on every request has a clock-skew or registration problem
            // that the retry is quietly papering over.
            tracing::debug!(
                url = %url,
                "401 from a resource route; discarding the cached token and retrying once"
            );
            self.invalidate_if_current(&token).await;
            let retry_token = self.get_token().await?;
            let retry_response = self
                .execute(
                    method,
                    &url,
                    body.as_deref(),
                    &retry_token,
                    idempotency_key.as_deref(),
                )
                .await?;
            return Self::decode_response(retry_response).await;
        }

        Self::decode_response(response).await
    }

    pub(crate) async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: Option<String>,
    ) -> Result<T, Error> {
        self.send_authenticated(Method::GET, path, None, query, None)
            .await
    }

    pub(crate) async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: String,
        opts: RequestOptions,
    ) -> Result<T, Error> {
        let idempotency_key = opts
            .idempotency_key
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        self.send_authenticated(Method::POST, path, Some(body), None, Some(idempotency_key))
            .await
    }
}

/// Builds a [`Client`]. See `docs/flows/merchant-auth.md`'s "Endpoint
/// locations" table for the defaults `issuer`/`token_endpoint` derive from
/// `base_url`, and for why they are each independently overridable.
pub struct ClientBuilder {
    base_url: String,
    credentials: Option<Credentials>,
    issuer: Option<String>,
    token_endpoint: Option<String>,
    audience: String,
    scope: Option<String>,
    assertion_lifetime: Duration,
    timeout: Duration,
}

impl fmt::Debug for ClientBuilder {
    /// Hand-written for the same reason as [`Client`]'s: `credentials`, once
    /// set, must never reach a `{:?}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientBuilder")
            .field("base_url", &self.base_url)
            .field("credentials", &self.credentials)
            .field("issuer", &self.issuer)
            .field("token_endpoint", &self.token_endpoint)
            .field("audience", &self.audience)
            .field("scope", &self.scope)
            .field("assertion_lifetime", &self.assertion_lifetime)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl ClientBuilder {
    fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            credentials: None,
            issuer: None,
            token_endpoint: None,
            audience: DEFAULT_AUDIENCE.to_string(),
            scope: None,
            assertion_lifetime: Duration::from_secs(60),
            timeout: Duration::from_secs(30),
        }
    }

    /// The merchant's credentials. Required — [`ClientBuilder::build`]
    /// returns [`ConfigError::MissingCredentials`] without it.
    #[must_use]
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Overrides the default `{base_url}/v1/oauth` issuer.
    #[must_use]
    pub fn issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = Some(issuer.into());
        self
    }

    /// Overrides the default `{issuer}/token` token endpoint (also the
    /// assertion's `aud`).
    #[must_use]
    pub fn token_endpoint(mut self, token_endpoint: impl Into<String>) -> Self {
        self.token_endpoint = Some(token_endpoint.into());
        self
    }

    /// Overrides the default [`DEFAULT_AUDIENCE`] (`"vpay:v1"`) requested on
    /// the token exchange.
    #[must_use]
    pub fn audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = audience.into();
        self
    }

    /// A scope to request. Omitted from the token request entirely when
    /// unset, per `docs/flows/merchant-auth.md`'s token-request table.
    #[must_use]
    pub fn scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    /// How long a minted assertion's `exp` is set for. Must be `1..=300`
    /// seconds; checked at [`ClientBuilder::build`], not here, so builder
    /// calls can be chained in any order.
    #[must_use]
    pub fn assertion_lifetime(mut self, lifetime: Duration) -> Self {
        self.assertion_lifetime = lifetime;
        self
    }

    /// The HTTP client's request timeout.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Validates configuration and constructs the [`Client`].
    ///
    /// # Errors
    /// [`ConfigError::MissingCredentials`] if [`ClientBuilder::credentials`]
    /// was never called; [`ConfigError::InvalidAssertionLifetime`] if
    /// [`ClientBuilder::assertion_lifetime`] is outside `1..=300` seconds;
    /// [`ConfigError::Http`] if the underlying `reqwest::Client` fails to
    /// build.
    pub fn build(self) -> Result<Client, ConfigError> {
        let credentials = self.credentials.ok_or(ConfigError::MissingCredentials)?;
        auth::validate_lifetime_secs(self.assertion_lifetime)?;

        let base_url = self.base_url.trim_end_matches('/').to_string();
        // A build-time intermediate, not client state: the issuer's only job
        // is to supply the default token endpoint below. Nothing on the wire
        // carries it — the assertion's `aud` is the token endpoint, and the
        // resource base comes from `base_url` — so keeping a copy on `Inner`
        // would be a field that could drift out of step with
        // `token_endpoint` without any test noticing.
        let issuer = self
            .issuer
            .unwrap_or_else(|| format!("{base_url}/v1/oauth"))
            .trim_end_matches('/')
            .to_string();
        let token_endpoint = self
            .token_endpoint
            .unwrap_or_else(|| format!("{issuer}/token"));
        let resource_base = format!("{base_url}/v1");

        let http = reqwest::Client::builder()
            .timeout(self.timeout)
            // The bare config, not `Some(config)`: `tls_backend_preconfigured`
            // wraps its argument in an `Option` itself before downcasting, so
            // an already-`Option` argument silently becomes
            // `UnknownPreconfigured` and `build()` returns "builder error".
            .tls_backend_preconfigured(rustls_client_config()?)
            .build()
            .map_err(|e| ConfigError::Http(e.to_string()))?;

        Ok(Client {
            inner: Arc::new(Inner {
                http,
                credentials,
                token_endpoint,
                resource_base,
                audience: self.audience,
                scope: self.scope,
                assertion_lifetime: self.assertion_lifetime,
                token_cache: AsyncMutex::new(None),
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_RSA_PEM: &str = include_str!("../tests/fixtures/test_rsa_key_pkcs8.pem");

    fn creds() -> Credentials {
        Credentials::rsa_pem("svc-1", TEST_RSA_PEM).unwrap()
    }

    #[test]
    fn derives_the_documented_defaults() {
        let client = Client::builder("https://api.vpay.example")
            .credentials(creds())
            .build()
            .unwrap();
        // The issuer's whole effect is this default; it is not kept on the
        // client, so this is where it is observable.
        assert_eq!(
            client.inner.token_endpoint,
            "https://api.vpay.example/v1/oauth/token"
        );
        assert_eq!(client.inner.resource_base, "https://api.vpay.example/v1");
        assert_eq!(client.inner.audience, DEFAULT_AUDIENCE);
    }

    #[test]
    fn strips_a_trailing_slash_from_base_url() {
        let client = Client::builder("https://api.vpay.example/")
            .credentials(creds())
            .build()
            .unwrap();
        assert_eq!(client.inner.resource_base, "https://api.vpay.example/v1");
    }

    #[test]
    fn overriding_the_issuer_moves_the_default_token_endpoint_but_not_the_resource_base() {
        let client = Client::builder("https://api.vpay.example")
            .credentials(creds())
            .issuer("https://auth.vpay.example")
            .build()
            .unwrap();
        assert_eq!(
            client.inner.token_endpoint,
            "https://auth.vpay.example/token"
        );
        assert_eq!(client.inner.resource_base, "https://api.vpay.example/v1");
    }

    #[test]
    fn build_fails_without_credentials() {
        let result = Client::builder("https://api.vpay.example").build();
        assert!(matches!(result, Err(ConfigError::MissingCredentials)));
    }

    #[test]
    fn build_rejects_an_assertion_lifetime_outside_one_to_three_hundred_seconds() {
        let too_long = Client::builder("https://api.vpay.example")
            .credentials(creds())
            .assertion_lifetime(Duration::from_secs(301))
            .build();
        assert!(matches!(
            too_long,
            Err(ConfigError::InvalidAssertionLifetime { .. })
        ));

        let too_short = Client::builder("https://api.vpay.example")
            .credentials(creds())
            .assertion_lifetime(Duration::from_millis(0))
            .build();
        assert!(matches!(
            too_short,
            Err(ConfigError::InvalidAssertionLifetime { .. })
        ));
    }

    #[test]
    fn cached_token_margin_is_thirty_seconds_or_half_expires_in_whichever_is_smaller() {
        let long = CachedToken::new("t".to_string(), 300);
        let remaining = long.valid_until.saturating_duration_since(Instant::now());
        // 300 - 30 = 270, allow scheduling slack.
        assert!(remaining.as_secs() >= 268 && remaining.as_secs() <= 270);

        let short = CachedToken::new("t".to_string(), 1);
        // margin = min(30, 0) = 0, usable = 1.
        let remaining_short = short.valid_until.saturating_duration_since(Instant::now());
        assert!(remaining_short.as_secs() <= 1);
        assert!(short.is_valid());
    }

    #[test]
    fn the_tls_config_carries_vendored_roots_and_advertises_http2() {
        // Both properties are invisible from the outside: an empty root store
        // fails only against a real server (there is no live-TLS test — see
        // `tests/tls.rs`), and missing ALPN silently downgrades every HTTPS
        // connection to HTTP/1.1, since reqwest only sets ALPN on the code
        // path this crate deliberately does not take.
        let roots = vendored_root_store();
        assert!(
            roots.roots.len() > 100,
            "the vendored Mozilla bundle should hold hundreds of anchors, got {}",
            roots.roots.len()
        );

        let config = rustls_client_config().expect("the rustls config builds");
        assert_eq!(
            config.alpn_protocols,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        );
    }

    #[test]
    fn bounded_prefix_cuts_on_a_character_boundary_and_keeps_real_replacements() {
        // Under the byte limit: returned whole.
        assert_eq!(bounded_prefix("héllo".as_bytes()), "héllo");

        // Exactly the limit, all ASCII: 500 bytes, nothing dropped.
        let ascii = "x".repeat(10_000);
        assert_eq!(bounded_prefix(ascii.as_bytes()).len(), BODY_PREFIX_LIMIT);

        // 3-byte characters: 166 fit in 498 bytes and the 167th straddles the
        // cut, so it is dropped rather than replaced.
        let three_byte = "€".repeat(1_000);
        let cut = bounded_prefix(three_byte.as_bytes());
        assert_eq!(cut.len(), 498);
        assert!(!cut.contains('\u{FFFD}'));

        // 4-byte characters: 125 fit exactly in 500 bytes, so nothing is
        // dropped — the back-off must not fire when the cut already lands on
        // a boundary.
        let four_byte = "😀".repeat(1_000);
        let cut = bounded_prefix(four_byte.as_bytes());
        assert_eq!(cut.len(), BODY_PREFIX_LIMIT);
        assert_eq!(cut.chars().count(), 125);

        // A byte that is invalid UTF-8 wherever it sits is still rendered as
        // U+FFFD: only a character *truncated by the cut* is dropped.
        assert!(bounded_prefix(b"ok\xffthen").contains('\u{FFFD}'));
    }

    #[test]
    fn client_debug_output_never_contains_the_pem_or_a_cached_token() {
        let client = Client::builder("https://api.vpay.example")
            .credentials(creds())
            .build()
            .unwrap();
        let debug = format!("{client:?}");
        assert!(!debug.contains("BEGIN"));
        assert!(!debug.contains("PRIVATE KEY"));
        assert!(debug.contains("[redacted]"));
    }

    #[test]
    fn builder_debug_output_never_contains_the_pem() {
        let builder = Client::builder("https://api.vpay.example").credentials(creds());
        let debug = format!("{builder:?}");
        assert!(!debug.contains("BEGIN"));
        assert!(!debug.contains("PRIVATE KEY"));
    }
}
