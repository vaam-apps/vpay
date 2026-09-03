//! The OAuth token MTN's Collections API wants on every call, and the cache
//! that keeps one request from becoming two.
//!
//! MTN's `POST /collection/token/` mints a bearer token from HTTP Basic
//! credentials (`api_user:api_key`) plus a subscription key, and says how
//! long it lives (`expires_in`, an hour in practice). Minting one per rail
//! call would double every request this adapter makes, so the token is
//! cached in memory — never in the database, because a bearer at rest is a
//! bearer to protect, and re-minting after a restart costs one round trip.
//!
//! # Why the cache is keyed at all
//!
//! [`ProviderAdapter`](vpay_provider::ProviderAdapter) takes
//! `&ProviderConfig` *per call*: one `Adapter` value can therefore be handed
//! two different merchants' credentials for the same rail. A cache keyed by
//! nothing would hand merchant B the token minted for merchant A — a
//! cross-tenant credential leak that no test of a single-merchant deployment
//! would ever show. [`Credentials::fingerprint`] is what makes that
//! structurally impossible: a token is only reused when the SHA-256 of the
//! credentials that minted it matches the ones being used now.
//!
//! # Secrets
//!
//! Nothing in this module renders a credential or a token. [`CachedToken`]'s
//! `Debug` redacts its value, the two header values are marked *sensitive* so
//! `http`'s own logging elides them, and every error message names the
//! configuration *key* that was wrong, never its value.
//!
//! # Why this cache is duplicated in the Orange adapter
//!
//! `vpay-adapter-orange-money`'s `token` module holds a second `CachedToken`
//! with the same shape, and that duplication is deliberate for now rather
//! than an oversight. The two token endpoints agree on almost nothing that a
//! shared type could absorb: MTN mints from HTTP Basic
//! (`api_user:api_key`) *plus* a subscription-key header against a path
//! under the configured base URL and answers `expires_in` as an optional
//! field, while Orange posts OAuth2 client-credentials to the *host root*
//! (`/oauth/v2/token`, see that module's `token_url`) and answers a
//! differently-shaped body. A shared `CachedToken` in `vpay-provider` is the
//! right end state, and the follow-up is worth doing once a third rail
//! exists to show which parts are actually common — generalising from two
//! examples would fix the wrong axis.
//!
//! Until then, two properties MUST stay aligned across both modules, because
//! each was a real defect in one of them and a divergence is silent:
//!
//! 1. **`minted_at` is recorded before the token request is sent**, never
//!    from the clock after the response arrives. The rail's `expires_in`
//!    counts from the rail's mint, so measuring from arrival grants the
//!    token the round trip as extra life — on a slow rail, the whole of the
//!    refresh margin. See [`expiry`] and [`CachedToken::new`].
//! 2. **The secret half of the credentials is in the fingerprint**
//!    ([`Credentials::fingerprint`] hashes `api_key`, not just `api_user`).
//!    A rotated secret must evict the cache immediately; hashing only the
//!    non-secret identifier leaves a bearer minted from a revoked credential
//!    in use until it ages out. Orange's fingerprint had exactly that bug
//!    until the Step 3 security review.
//!
//! Change either property here and change it there in the same commit.

use std::fmt;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderName, HeaderValue};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use vpay_core::FailureCode;
use vpay_provider::{ProviderConfig, ProviderError};

/// MTN's per-product API-gateway key. Lowercase because
/// [`HeaderName::from_static`] requires it; HTTP header names are
/// case-insensitive on the wire.
pub(crate) const SUBSCRIPTION_KEY_HEADER: HeaderName =
    HeaderName::from_static("ocp-apim-subscription-key");

/// `sandbox` or a subsidiary's production name (`mtncameroon`). MTN rejects
/// a request whose environment does not match the one the credentials belong
/// to, with a 500 (see [`crate::mapping::CONFIGURATION_CODES`]).
pub(crate) const TARGET_ENVIRONMENT_HEADER: HeaderName =
    HeaderName::from_static("x-target-environment");

/// How long before a token's stated expiry it is treated as expired.
///
/// A minute, because the clock that matters is MTN's and we cannot see it:
/// the round trip, a retry, and any skew between the two ends all have to
/// fit inside the margin, and the cost of being early is one extra token
/// mint an hour.
const REFRESH_MARGIN: Duration = Duration::from_secs(60);

/// The lifetime assumed when MTN does not state one.
///
/// Deliberately short. Guessing *long* means a stream of 401s once the real
/// expiry passes; guessing short costs a few extra mints and nothing else.
/// MTN documents `expires_in: 3600` and sends it, so this is a fallback for
/// a rail that changed its mind, not the normal path.
const ASSUMED_LIFETIME: Duration = Duration::from_secs(300);

/// The credentials and non-secret settings one MTN call needs, borrowed from
/// a [`ProviderConfig`] rather than copied, so no secret is duplicated into a
/// second allocation that outlives the call.
pub(crate) struct Credentials<'a> {
    /// `credentials.subscription_key` — the Collections product key.
    subscription_key: &'a str,
    /// `credentials.api_key` — the Basic-auth password for the token call.
    api_key: &'a str,
    /// `settings.api_user` — a UUID, and the Basic-auth username. Not a
    /// secret, which is why it lives in `settings`.
    api_user: &'a str,
    /// `settings.target_environment`.
    target_environment: &'a str,
}

impl<'a> Credentials<'a> {
    /// Reads the four values this adapter needs out of a `ProviderConfig`.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Config`] naming the missing key. A missing
    /// credential is a deployment mistake, not a rail failure and certainly
    /// not a decline: `Category::Configuration` is what stops it being
    /// retried against a rail that will keep saying no.
    pub(crate) fn from_config(config: &'a ProviderConfig) -> Result<Self, ProviderError> {
        fn required<'m>(
            map: &'m std::collections::BTreeMap<String, String>,
            key: &str,
            which: &str,
        ) -> Result<&'m str, ProviderError> {
            map.get(key)
                .map(String::as_str)
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| {
                    ProviderError::Config(format!("mtn_momo: {which}.{key} is required"))
                })
        }

        Ok(Self {
            subscription_key: required(&config.credentials, "subscription_key", "credentials")?,
            api_key: required(&config.credentials, "api_key", "credentials")?,
            api_user: required(&config.settings, "api_user", "settings")?,
            target_environment: required(&config.settings, "target_environment", "settings")?,
        })
    }

    /// A digest of the credentials that mint a token, used as the cache key.
    ///
    /// Each field is length-prefixed before hashing so that two different
    /// pairs cannot produce one digest by concatenating differently
    /// (`("ab", "c")` and `("a", "bc")`): the whole point of the key is that
    /// distinct credentials are distinct, and a concatenation collision
    /// would defeat it in exactly the case it exists for.
    ///
    /// `api_key` is hashed in **because** it is the token's password, not
    /// despite it. Leaving it out — as this did until the Step 3 security
    /// review — meant a deployment that rotated only the API key kept
    /// serving calls with the bearer minted from the *old* one: the
    /// fingerprint still matched, so the cache still hit, and the rotation
    /// took effect only when the cached token aged out (up to an hour) or
    /// the rail answered 401. A key is rotated precisely when it must stop
    /// working immediately, which is the case that behaviour broke.
    ///
    /// The digest is what makes that safe to hold: the cache key is a
    /// SHA-256, never the credential, so including a secret here does not
    /// put one anywhere it was not already.
    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for field in [self.subscription_key, self.api_key, self.api_user] {
            let len = u64::try_from(field.len()).unwrap_or(u64::MAX);
            hasher.update(len.to_be_bytes());
            hasher.update(field.as_bytes());
        }
        hasher.finalize().into()
    }

    pub(crate) const fn target_environment(&self) -> &'a str {
        self.target_environment
    }

    /// The subscription-key header, marked sensitive.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Config`] if the configured value is not a legal
    /// header value (a stray newline out of YAML, most likely). The message
    /// names the key and never the value — an error string is the one place a
    /// credential most easily escapes into a log.
    pub(crate) fn subscription_header(&self) -> Result<HeaderValue, ProviderError> {
        let mut value = HeaderValue::from_str(self.subscription_key).map_err(|_| {
            ProviderError::Config(
                "mtn_momo: credentials.subscription_key is not a valid HTTP header value"
                    .to_owned(),
            )
        })?;
        value.set_sensitive(true);
        Ok(value)
    }
}

/// Redacted, for the same reason [`CachedToken`]'s is: this struct is two
/// secrets and two settings, and the only safe rendering names the settings.
impl fmt::Debug for Credentials<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("api_user", &self.api_user)
            .field("target_environment", &self.target_environment)
            .field("subscription_key", &"<redacted>")
            .field("api_key", &"<redacted>")
            .finish()
    }
}

/// A bearer token together with the moment it stops being usable and the
/// fingerprint of the credentials that produced it.
pub(crate) struct CachedToken {
    value: String,
    /// Already reduced by [`REFRESH_MARGIN`]. An [`Instant`], not a wall
    /// clock: a token's remaining life is elapsed time, and a machine whose
    /// wall clock steps backwards must not resurrect an expired token.
    expires_at: Instant,
    fingerprint: [u8; 32],
}

impl CachedToken {
    /// Builds the cache entry for a freshly minted token.
    pub(crate) fn new(
        value: String,
        fingerprint: [u8; 32],
        minted_at: Instant,
        expires_in: Option<u64>,
    ) -> Self {
        Self {
            value,
            expires_at: expiry(minted_at, expires_in),
            fingerprint,
        }
    }

    /// The token as minted, whatever its age.
    ///
    /// Only for the caller that just minted it: everything reading the
    /// *cache* goes through [`CachedToken::usable`], which cannot hand back a
    /// value without having asked both questions first.
    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    /// The token, if it is still valid *and* was minted from these
    /// credentials.
    ///
    /// Returning an `Option<&str>` rather than a `bool` is what keeps the two
    /// checks together: a caller cannot read the value without having asked
    /// both questions.
    pub(crate) fn usable(&self, now: Instant, fingerprint: &[u8; 32]) -> Option<&str> {
        (self.fingerprint == *fingerprint && now < self.expires_at).then_some(self.value.as_str())
    }
}

/// Redacted on purpose: a `Debug` that prints a bearer token turns any
/// `tracing` call that formats the adapter into a credential disclosure.
impl fmt::Debug for CachedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedToken")
            .field("value", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// When a token minted at `minted_at` should stop being used.
///
/// Split out and `pub(crate)` so the arithmetic can be proven without a
/// network: the interesting cases are a lifetime *shorter* than the margin
/// (which must not underflow into a token that never expires) and a missing
/// `expires_in`.
pub(crate) fn expiry(minted_at: Instant, expires_in: Option<u64>) -> Instant {
    let lifetime = expires_in.map_or(ASSUMED_LIFETIME, Duration::from_secs);
    minted_at + lifetime.saturating_sub(REFRESH_MARGIN)
}

/// What MTN answers a token request with. Extra fields are ignored.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<ExpiresIn>,
}

/// `expires_in` as a number *or* as a string.
///
/// MTN documents a number and the sandbox sends one, but OAuth deployments
/// quoting it are common enough that treating a string as "no expiry stated"
/// would silently halve the cache's usefulness on the day it happened.
#[derive(Deserialize)]
#[serde(untagged)]
enum ExpiresIn {
    Seconds(u64),
    Text(String),
}

impl ExpiresIn {
    fn seconds(&self) -> Option<u64> {
        match self {
            Self::Seconds(seconds) => Some(*seconds),
            Self::Text(text) => text.trim().parse().ok(),
        }
    }
}

/// Mints a fresh bearer token. Always a network call — the cache lives in
/// [`crate::Adapter`], which is the only thing that can decide whether one is
/// needed.
///
/// # Errors
///
/// * [`ProviderError::Rejected`] with [`FailureCode::ProviderAccountBlocked`]
///   on a 401/403. That is *our* partner credentials being refused, not a
///   payer's problem, and `docs/flows/failures.md` says it pages. Retrying a
///   mint with the same rejected key would only turn a page into a loop.
/// * [`ProviderError::Transport`] on a timeout, a connection failure or a 5xx.
/// * [`ProviderError::Malformed`] on a 200 that is not the documented body,
///   or any other status.
pub(crate) async fn mint(
    http: &reqwest::Client,
    config: &ProviderConfig,
    credentials: &Credentials<'_>,
) -> Result<CachedToken, ProviderError> {
    let url = format!("{}/collection/token/", crate::base_url(config));
    // Recorded before the call, not after: a token's life starts when MTN
    // mints it, and counting from the response would credit the token with
    // however long the round trip took.
    let minted_at = Instant::now();

    let response = http
        .post(&url)
        // `basic_auth`/`bearer_auth` mark the `Authorization` header
        // sensitive, which is why they are used instead of building the
        // base64 by hand.
        .basic_auth(credentials.api_user, Some(credentials.api_key))
        .header(SUBSCRIPTION_KEY_HEADER, credentials.subscription_header()?)
        // Per-request rather than per-client: one `reqwest::Client` is shared
        // by every rail in the process, so the deadline has to come from this
        // rail's `ProviderConfig` (see its docs).
        .timeout(config.request_timeout)
        .send()
        .await
        .map_err(crate::transport)?;

    // Bounded like every other rail read (`crate::read_body`): a token
    // endpoint that answers with a proxy's error page must not be able to
    // decide how much memory this process allocates.
    let (status, body) = crate::read_body(response).await?;
    if status.is_success() {
        let parsed: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            // The body is *not* included: it contains the token.
            ProviderError::Malformed(format!(
                "mtn_momo: token response is not the documented shape: {e}"
            ))
        })?;
        tracing::debug!(rail = "mtn_momo", "minted a collections access token");
        return Ok(CachedToken::new(
            parsed.access_token,
            credentials.fingerprint(),
            minted_at,
            parsed.expires_in.as_ref().and_then(ExpiresIn::seconds),
        ));
    }

    Err(match status.as_u16() {
        401 | 403 => ProviderError::Rejected {
            code: FailureCode::ProviderAccountBlocked,
            message: format!("mtn_momo: the rail refused our credentials (HTTP {status})"),
        },
        _ if status.is_server_error() => {
            ProviderError::Transport(format!("mtn_momo: token endpoint answered HTTP {status}"))
        }
        // Redirects are not followed (`vpay_provider::http`), so a 3xx on
        // the token call is a refusal, not a hop — and the hop is the one
        // that would have replayed the Basic credentials at whatever host
        // the `Location` named.
        _ if status.is_redirection() => ProviderError::Malformed(format!(
            "mtn_momo: token endpoint answered a redirect (HTTP {status}), which is not \
             followed; check base_url"
        )),
        _ => ProviderError::Malformed(format!(
            "mtn_momo: token endpoint answered an unexpected HTTP {status}"
        )),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use vpay_core::Currency;

    use super::*;

    fn config(
        credentials: BTreeMap<String, String>,
        settings: BTreeMap<String, String>,
    ) -> ProviderConfig {
        ProviderConfig {
            base_url: "http://127.0.0.1:1".to_owned(),
            callback_url: "http://127.0.0.1:1/provider/mtn_momo/callback".to_owned(),
            currency: Currency::Eur,
            settings,
            credentials,
            connect_timeout: Duration::from_millis(50),
            request_timeout: Duration::from_millis(50),
        }
    }

    fn complete() -> ProviderConfig {
        config(
            BTreeMap::from([
                // Distinctive values with no substring in common with the
                // key names, so "did the value leak into this message?" is
                // an assertion that can actually fail.
                ("subscription_key".to_owned(), "sh1bboleth".to_owned()),
                ("api_key".to_owned(), "0pen-sesame".to_owned()),
            ]),
            BTreeMap::from([
                ("api_user".to_owned(), "user".to_owned()),
                ("target_environment".to_owned(), "sandbox".to_owned()),
            ]),
        )
    }

    #[test]
    fn a_missing_credential_names_the_key_and_never_the_value() {
        for key in ["subscription_key", "api_key"] {
            let mut cfg = complete();
            cfg.credentials.remove(key);
            match Credentials::from_config(&cfg) {
                Err(ProviderError::Config(message)) => {
                    assert!(message.contains(key), "{message}");
                    assert!(
                        !message.contains("sh1bboleth") && !message.contains("0pen-sesame"),
                        "the value leaked: {message}"
                    );
                }
                other => panic!("{key}: expected a Config error, got {other:?}"),
            }
        }
        for key in ["api_user", "target_environment"] {
            let mut cfg = complete();
            cfg.settings.remove(key);
            assert!(matches!(
                Credentials::from_config(&cfg),
                Err(ProviderError::Config(_))
            ));
        }
    }

    /// A key present but blank is the shape `${VAR}` expansion leaves behind
    /// when the variable is unset, and it must fail like an absent one rather
    /// than reaching the rail as an empty Basic-auth password.
    #[test]
    fn a_blank_credential_is_a_missing_one() {
        let mut cfg = complete();
        cfg.credentials
            .insert("api_key".to_owned(), "   ".to_owned());
        assert!(matches!(
            Credentials::from_config(&cfg),
            Err(ProviderError::Config(_))
        ));
    }

    #[test]
    fn different_credentials_fingerprint_differently() {
        let a = complete();
        let mut b = complete();
        b.credentials
            .insert("subscription_key".to_owned(), "other".to_owned());
        let mut c = complete();
        c.settings.insert("api_user".to_owned(), "other".to_owned());
        // The rotation case: only the secret half changes. This is what
        // used to collide, because `api_key` was left out of the digest.
        let mut d = complete();
        d.credentials
            .insert("api_key".to_owned(), "r0tated".to_owned());

        let fa = Credentials::from_config(&a)
            .expect("complete")
            .fingerprint();
        let fb = Credentials::from_config(&b)
            .expect("complete")
            .fingerprint();
        let fc = Credentials::from_config(&c)
            .expect("complete")
            .fingerprint();
        let fd = Credentials::from_config(&d)
            .expect("complete")
            .fingerprint();

        assert_ne!(fa, fb, "a second subscription key must not reuse a token");
        assert_ne!(fa, fc, "a second api user must not reuse a token");
        assert_ne!(
            fa, fd,
            "a rotated api_key must evict the token minted from the old one, immediately \
             rather than when it ages out"
        );
        assert_eq!(
            fa,
            Credentials::from_config(&complete())
                .expect("complete")
                .fingerprint(),
            "the same credentials must reuse their token"
        );
    }

    /// The concatenation collision the length prefix exists to prevent: with
    /// a plain `hash(a + b)` these two configurations would share a token.
    #[test]
    fn a_field_boundary_cannot_be_shifted_into_a_collision() {
        let left = config(
            BTreeMap::from([
                ("subscription_key".to_owned(), "ab".to_owned()),
                ("api_key".to_owned(), "key".to_owned()),
            ]),
            BTreeMap::from([
                ("api_user".to_owned(), "c".to_owned()),
                ("target_environment".to_owned(), "sandbox".to_owned()),
            ]),
        );
        let right = config(
            BTreeMap::from([
                ("subscription_key".to_owned(), "a".to_owned()),
                ("api_key".to_owned(), "key".to_owned()),
            ]),
            BTreeMap::from([
                ("api_user".to_owned(), "bc".to_owned()),
                ("target_environment".to_owned(), "sandbox".to_owned()),
            ]),
        );
        assert_ne!(
            Credentials::from_config(&left)
                .expect("complete")
                .fingerprint(),
            Credentials::from_config(&right)
                .expect("complete")
                .fingerprint()
        );
    }

    #[test]
    fn a_token_expires_a_minute_before_the_rail_says_it_does() {
        let now = Instant::now();
        assert_eq!(expiry(now, Some(3_600)), now + Duration::from_secs(3_540));
    }

    /// A stated lifetime shorter than the margin must clamp to "already
    /// expired", not wrap around into a token that outlives the process.
    #[test]
    fn a_lifetime_shorter_than_the_margin_expires_immediately() {
        let now = Instant::now();
        assert_eq!(expiry(now, Some(30)), now);
        assert_eq!(expiry(now, Some(0)), now);
    }

    #[test]
    fn a_rail_that_states_no_lifetime_gets_a_short_one() {
        let now = Instant::now();
        let assumed = expiry(now, None);
        assert!(assumed > now, "an unstated lifetime must still be usable");
        assert!(
            assumed < now + Duration::from_secs(3_600),
            "an unstated lifetime must not be assumed to be an hour"
        );
    }

    #[test]
    fn a_cached_token_is_reused_only_before_expiry_and_only_for_its_own_credentials() {
        let now = Instant::now();
        let mine = [1_u8; 32];
        let theirs = [2_u8; 32];
        let token = CachedToken::new("secret".to_owned(), mine, now, Some(3_600));

        assert_eq!(token.usable(now, &mine), Some("secret"));
        assert_eq!(
            token.usable(now, &theirs),
            None,
            "a second merchant's call must never reuse the first's token"
        );
        assert_eq!(
            token.usable(now + Duration::from_secs(3_540), &mine),
            None,
            "an expired token must be re-minted"
        );
    }

    #[test]
    fn debugging_credentials_does_not_print_them() {
        let config = complete();
        let rendered = format!("{:?}", Credentials::from_config(&config).expect("complete"));
        assert!(!rendered.contains("sh1bboleth"), "{rendered}");
        assert!(!rendered.contains("0pen-sesame"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(
            rendered.contains("sandbox"),
            "the non-secret settings are what makes it useful: {rendered}"
        );
    }

    #[test]
    fn debugging_a_token_does_not_print_it() {
        let rendered = format!(
            "{:?}",
            CachedToken::new("super-secret".to_owned(), [0_u8; 32], Instant::now(), None)
        );
        assert!(!rendered.contains("super-secret"), "{rendered}");
    }
}
