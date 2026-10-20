//! The one identity provider a deployment federates its staff sign-in to:
//! discovery, the authorization URL, the code exchange, and the key set
//! ([ADR-0030](../../../../../../docs/adr/0030-staff-sign-in-federates-to-an-oidc-provider.md)).
//!
//! # Everything here is about exactly one issuer
//!
//! [`SsoProvider`] is built from `staff_auth.sso` and holds that block's
//! issuer. Discovery is fetched from it and refused unless the document's
//! `issuer` equals it byte for byte (OpenID Connect Discovery 1.0 §4.3 — the
//! check that stops a document served at one URL vouching for another). The key
//! set is fetched from the `jwks_uri` *that document* names, by a
//! [`KeySetCache`] with no method that takes a URI. There is no code path that
//! asks any other issuer for anything, which is what ADR-0019 decision 5 means
//! by "which issuers a deployment accepts is configuration": the issuer string
//! in a `credentials` row is never dereferenced.
//!
//! # Discovery is lazy, and a down provider is not a down vpay
//!
//! Nothing here runs at boot. A deployment whose provider is down at start-up
//! still serves password + TOTP (the break-glass path) and every `/v1`
//! request, and the first SSO attempt after the provider returns discovers it.
//! The document is cached for [`DISCOVERY_TTL`]; a refresh that fails keeps
//! serving the last good one rather than turning a provider blip into a
//! sign-in outage.
//!
//! # What goes over the wire, and what never reaches a log
//!
//! The client authenticates with `client_secret_basic`, the token request
//! carries the PKCE verifier, and the answer's `id_token` is verified by
//! [`super::verify`] — the access token is never read, and `userinfo` is never
//! called: every claim vpay uses is in the signed id_token. No request or
//! response body is logged on any path.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use tokio::sync::RwLock;
use url::Url;
use vpay_config::StaffSso;

use super::error::SsoError;
use super::verify::{self, Expectations, IdentityClaims};
use crate::jwks_cache::KeySetCache;

/// How long a discovery document is trusted before it is fetched again.
const DISCOVERY_TTL: Duration = Duration::from_secs(3600);

/// How long the issuer's key set is cached. Rotation is picked up sooner by
/// [`KeySetCache`]'s throttled re-fetch on an unknown `kid`.
const KEY_SET_TTL: Duration = Duration::from_secs(300);

/// The most of a discovery document or a token response this process holds.
const MAX_BODY_BYTES: usize = 256 * 1024;

/// How long a connection may take to open, and a whole request to finish.
///
/// Short, because a sign-in is a person waiting and the provider is on the
/// critical path of exactly one request they make.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// What discovery taught us, for as long as it is fresh.
struct Discovered {
    authorization_endpoint: Url,
    token_endpoint: Url,
    keys: KeySetCache,
}

/// The raw document, every field optional so a missing one is a refusal with a
/// name rather than a serde error.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct DiscoveryDocument {
    issuer: Option<String>,
    authorization_endpoint: Option<String>,
    token_endpoint: Option<String>,
    jwks_uri: Option<String>,
    #[serde(default)]
    token_endpoint_auth_methods_supported: Option<Vec<String>>,
    #[serde(default)]
    code_challenge_methods_supported: Option<Vec<String>>,
}

/// The token endpoint's answer, read for one field.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct TokenResponse {
    id_token: Option<String>,
}

/// The token endpoint's error body (RFC 6749 §5.2), read for one field.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct TokenError {
    error: Option<String>,
}

/// The provider this deployment's staff may sign in through.
pub struct SsoProvider {
    config: StaffSso,
    /// `dashboard_client.merchant_id`: the tenant every member provisioned by
    /// a first sign-in is bound to. **Never read from a token** (ADR-0019 §
    /// R2, taken by ADR-0030).
    merchant_id: String,
    client: reqwest::Client,
    discovered: RwLock<Option<(Arc<Discovered>, Instant)>>,
}

/// Redacts everything: the client secret is in there, and what an operator
/// wants from a startup log is the issuer the boot line already prints.
impl std::fmt::Debug for SsoProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsoProvider")
            .field("issuer", &self.config.issuer)
            .finish_non_exhaustive()
    }
}

impl SsoProvider {
    /// Builds the provider from a validated `staff_auth.sso` block.
    ///
    /// # Errors
    ///
    /// [`crate::http_client::HttpClientError`] if the outbound client cannot
    /// be built — the fixed TLS stack failing to assemble, which no
    /// configuration can cause.
    pub fn new(
        config: &StaffSso,
        merchant_id: &str,
    ) -> Result<Self, crate::http_client::HttpClientError> {
        Ok(Self {
            config: config.clone(),
            merchant_id: merchant_id.to_owned(),
            client: crate::http_client::client_with_timeouts(CONNECT_TIMEOUT, REQUEST_TIMEOUT)?,
            discovered: RwLock::new(None),
        })
    }

    /// The configured issuer.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.config.issuer
    }

    /// The block this provider was built from.
    pub(crate) fn config(&self) -> &StaffSso {
        &self.config
    }

    /// The tenant a first sign-in is provisioned into.
    pub(crate) fn merchant_id(&self) -> &str {
        &self.merchant_id
    }

    /// The URL to send the browser to.
    ///
    /// Authorization-code flow with S256 PKCE, `state` and `nonce`; `prompt`
    /// is `login` only for the one retry ADR-0030 allows.
    ///
    /// # Errors
    ///
    /// [`SsoError::Idp`] / [`SsoError::Discovery`] if discovery fails.
    pub(crate) async fn authorization_url(
        &self,
        state: &str,
        nonce: &str,
        code_challenge: &str,
        prompt_login: bool,
    ) -> Result<String, SsoError> {
        let discovered = self.discovered().await?;
        let mut url = discovered.authorization_endpoint.clone();
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &self.config.client_id)
                .append_pair("redirect_uri", &self.config.redirect_uri)
                .append_pair("scope", &self.config.scopes.join(" "))
                .append_pair("state", state)
                .append_pair("nonce", nonce)
                .append_pair("code_challenge", code_challenge)
                .append_pair("code_challenge_method", "S256");
            if prompt_login {
                query.append_pair("prompt", "login");
            }
        }
        Ok(url.into())
    }

    /// Redeems an authorization code, and returns the `id_token` it bought.
    ///
    /// # Errors
    ///
    /// [`SsoError::CodeRefused`] when the provider answers `400`/`401`
    /// (a spent, expired or mismatched code, a wrong secret);
    /// [`SsoError::Idp`] for anything else that goes wrong;
    /// [`SsoError::NoIdToken`] if the answer holds none.
    pub(crate) async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<String, SsoError> {
        let discovered = self.discovered().await?;

        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "authorization_code")
            .append_pair("code", code)
            .append_pair("redirect_uri", &self.config.redirect_uri)
            .append_pair("code_verifier", code_verifier)
            .finish();

        let response = self
            .client
            .post(discovered.token_endpoint.clone())
            .header(reqwest::header::AUTHORIZATION, self.basic_credentials())
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| SsoError::Idp("the token endpoint could not be reached"))?;

        let (status, bytes) = vpay_provider::http::bounded_body(response, MAX_BODY_BYTES)
            .await
            .map_err(|_| SsoError::Idp("the token response could not be read"))?;

        if !status.is_success() {
            // RFC 6749 §5.2's `error` is a short registered token
            // (`invalid_grant`, `invalid_client`); it is logged, nothing else
            // from the body is. A provider-chosen string is bounded and
            // restricted to the registered alphabet before it reaches a log.
            let error = serde_json::from_slice::<TokenError>(&bytes)
                .ok()
                .and_then(|body| body.error)
                .filter(|error| {
                    error.len() <= 40
                        && error
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
                });
            tracing::warn!(
                status = status.as_u16(),
                error = error.as_deref().unwrap_or("-"),
                "the SSO token endpoint refused the authorization code"
            );
            return if matches!(status.as_u16(), 400 | 401) {
                Err(SsoError::CodeRefused)
            } else {
                Err(SsoError::Idp(
                    "the token endpoint answered with a failure status",
                ))
            };
        }

        serde_json::from_slice::<TokenResponse>(&bytes)
            .map_err(|_| SsoError::Idp("the token response is not JSON"))?
            .id_token
            .filter(|token| !token.is_empty())
            .ok_or(SsoError::NoIdToken)
    }

    /// Verifies an id_token against this issuer's published keys.
    ///
    /// # Errors
    ///
    /// [`SsoError::Header`], [`SsoError::UnknownKey`], [`SsoError::Token`] as
    /// [`verify`] documents; [`SsoError::Idp`] if the key set is unavailable.
    pub(crate) async fn verify_id_token(&self, id_token: &str) -> Result<IdentityClaims, SsoError> {
        let header = verify::accept_header(id_token)?;
        let discovered = self.discovered().await?;
        let key = discovered
            .keys
            .get_key(&header.kid)
            .await
            .map_err(|_| SsoError::Idp("the issuer's key set is unavailable"))?
            .ok_or(SsoError::UnknownKey)?;

        verify::verify_id_token(
            id_token,
            &header,
            &key,
            Expectations {
                issuer: &self.config.issuer,
                client_id: &self.config.client_id,
                groups_claim: &self.config.groups_claim,
                now: time::OffsetDateTime::now_utc().unix_timestamp(),
            },
        )
    }

    /// `Basic base64(form(client_id):form(client_secret))`, RFC 6749 §2.3.1.
    fn basic_credentials(&self) -> String {
        let encode = |value: &str| {
            url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>()
        };
        let pair = format!(
            "{}:{}",
            encode(&self.config.client_id),
            encode(&self.config.client_secret)
        );
        format!("Basic {}", STANDARD.encode(pair))
    }

    /// The discovery result, fetching when cold or stale.
    async fn discovered(&self) -> Result<Arc<Discovered>, SsoError> {
        {
            let read = self.discovered.read().await;
            if let Some((discovered, at)) = read.as_ref()
                && at.elapsed() < DISCOVERY_TTL
            {
                return Ok(Arc::clone(discovered));
            }
        }

        let mut write = self.discovered.write().await;
        if let Some((discovered, at)) = write.as_ref()
            && at.elapsed() < DISCOVERY_TTL
        {
            return Ok(Arc::clone(discovered));
        }

        match self.discover().await {
            Ok(fresh) => {
                let fresh = Arc::new(fresh);
                *write = Some((Arc::clone(&fresh), Instant::now()));
                Ok(fresh)
            }
            // A refresh that fails keeps the last good document. The first
            // fetch has nothing to fall back on and fails closed.
            Err(error) => match write.as_ref() {
                Some((stale, _)) => {
                    tracing::warn!(
                        %error,
                        "refreshing the SSO discovery document failed; serving the previous one"
                    );
                    Ok(Arc::clone(stale))
                }
                None => Err(error),
            },
        }
    }

    /// Fetches and checks `<issuer>/.well-known/openid-configuration`.
    async fn discover(&self) -> Result<Discovered, SsoError> {
        let issuer = Url::parse(&self.config.issuer)
            .map_err(|_| SsoError::Discovery("the configured issuer is not a URL"))?;

        // OIDC Discovery §4: "any terminating / MUST be removed before
        // appending". The configured string itself is NOT altered — it is the
        // value the document's `issuer` is compared against.
        let well_known = format!(
            "{}/.well-known/openid-configuration",
            self.config.issuer.trim_end_matches('/')
        );
        let response = self
            .client
            .get(&well_known)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| SsoError::Idp("the discovery document could not be fetched"))?;
        let (status, bytes) = vpay_provider::http::bounded_body(response, MAX_BODY_BYTES)
            .await
            .map_err(|_| SsoError::Idp("the discovery document could not be read"))?;
        if !status.is_success() {
            return Err(SsoError::Idp("discovery answered with a failure status"));
        }
        let document: DiscoveryDocument = serde_json::from_slice(&bytes)
            .map_err(|_| SsoError::Discovery("the document is not JSON"))?;

        // THE CHECK THAT MAKES THE DOCUMENT THIS ISSUER'S. Byte for byte, no
        // normalisation, no trailing-slash tolerance.
        if document.issuer.as_deref() != Some(self.config.issuer.as_str()) {
            return Err(SsoError::Discovery(
                "its issuer is not the configured issuer, byte for byte",
            ));
        }

        // `S256` is the only challenge method vpay sends. A provider that
        // advertises a list without it cannot complete the flow.
        if document
            .code_challenge_methods_supported
            .as_ref()
            .is_some_and(|methods| !methods.iter().any(|method| method == "S256"))
        {
            return Err(SsoError::Discovery("it does not advertise PKCE S256"));
        }
        // `client_secret_basic` is the only client authentication vpay sends;
        // an absent list means the default, which is that.
        if document
            .token_endpoint_auth_methods_supported
            .as_ref()
            .is_some_and(|methods| !methods.iter().any(|method| method == "client_secret_basic"))
        {
            return Err(SsoError::Discovery(
                "it does not advertise client_secret_basic",
            ));
        }

        let endpoint = |value: Option<String>, missing: &'static str| {
            let url = value
                .as_deref()
                .and_then(|raw| Url::parse(raw).ok())
                .ok_or(SsoError::Discovery(missing))?;
            if url.scheme() != issuer.scheme() {
                return Err(SsoError::Discovery(
                    "an endpoint's scheme differs from the issuer's",
                ));
            }
            if url.host_str().is_none_or(str::is_empty)
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err(SsoError::Discovery(
                    "an endpoint names no host, or carries credentials or a fragment",
                ));
            }
            Ok(url)
        };

        let authorization_endpoint = endpoint(
            document.authorization_endpoint,
            "it names no usable authorization_endpoint",
        )?;
        let token_endpoint =
            endpoint(document.token_endpoint, "it names no usable token_endpoint")?;
        let jwks_uri = endpoint(document.jwks_uri, "it names no usable jwks_uri")?;

        Ok(Discovered {
            authorization_endpoint,
            token_endpoint,
            keys: KeySetCache::new(jwks_uri.into(), KEY_SET_TTL, self.client.clone()),
        })
    }
}
