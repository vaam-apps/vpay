//! The OAuth token MTN's Collections API wants on every call, and this
//! rail's half of the cache that keeps one request from becoming two.
//!
//! `POST /collection/token/` mints a bearer from HTTP Basic credentials
//! (`api_user:api_key`) plus a subscription key. The cache entry itself is
//! [`vpay_provider::token::CachedToken`]; what is MTN's alone is
//! [`REFRESH_MARGIN`], [`ASSUMED_LIFETIME`] (MTN may omit `expires_in`), and
//! the fields [`Credentials::fingerprint`] hashes.
//!
//! `docs/reference/rails.md` has the rest: why the cache is keyed by a
//! credential digest at all, why the margin is per-rail rather than shared,
//! and the two invariants — `minted_at` read before the send, the secret half
//! in the fingerprint — that were each a real defect once.
//!
//! Nothing here renders a credential or a token: the header values are marked
//! *sensitive*, `CachedToken`'s `Debug` redacts its value, and every error
//! message names the configuration *key* that was wrong, never its value.

use std::fmt;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderName, HeaderValue};
use serde::Deserialize;
use vpay_core::FailureCode;
use vpay_provider::token::CachedToken;
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

    /// The cache key: a digest of the credentials that mint a token.
    ///
    /// `api_key` is hashed in **because** it is the token's password, not
    /// despite it — leaving it out meant a deployment that rotated only the
    /// API key kept serving calls with the bearer minted from the old one
    /// until it aged out. `docs/reference/rails.md` records the tenancy
    /// argument for the other fields and why a digest is safe to hold.
    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        vpay_provider::token::fingerprint(&[self.subscription_key, self.api_key, self.api_user])
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

/// The cache entry for a token MTN has just minted.
///
/// Named rather than a bare [`CachedToken::new`] at each call site, because
/// everything MTN-specific about a cached token is here: [`REFRESH_MARGIN`],
/// and [`ASSUMED_LIFETIME`] for the rail that states none. Neither is the
/// shared type's to know.
pub(crate) fn cache_entry(
    value: String,
    fingerprint: [u8; 32],
    minted_at: Instant,
    expires_in: Option<u64>,
) -> CachedToken {
    CachedToken::new(
        value,
        minted_at,
        lifetime(expires_in),
        REFRESH_MARGIN,
        fingerprint,
    )
}

/// The lifetime to credit a token with, from what the rail said — or did not.
const fn lifetime(expires_in: Option<u64>) -> Duration {
    match expires_in {
        Some(seconds) => Duration::from_secs(seconds),
        None => ASSUMED_LIFETIME,
    }
}

/// What MTN answers a token request with. Extra fields are ignored.
///
/// No `#[serde(rename_all = "snake_case")]` here, ever: that attribute is the
/// workspace convention for types that model *vpay's own* wire or config, so
/// a field added as `payTo` fails review instead of shipping. This type
/// models **MTN's** response, which is already camelCase-free by coincidence
/// (`access_token`, `expires_in`) — adding the attribute anyway would read as
/// a claim that these names are ours to normalise, and the day MTN sends a
/// field that is not already snake_case it would quietly rename it away from
/// the rail's own spelling. See `docs/reference/rails.md`'s "serde:
/// `rename_all` is for *our* wire, never a rail's" for the same rule as it
/// applies to `wire.rs` in both adapters.
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
            ProviderError::malformed(format!(
                "mtn_momo: token response is not the documented shape: {e}"
            ))
        })?;
        tracing::debug!(rail = "mtn_momo", "minted a collections access token");
        return Ok(cache_entry(
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
            ProviderError::transport(format!("mtn_momo: token endpoint answered HTTP {status}"))
        }
        // Redirects are not followed (`vpay_provider::http`), so a 3xx on
        // the token call is a refusal, not a hop — and the hop is the one
        // that would have replayed the Basic credentials at whatever host
        // the `Location` named.
        _ if status.is_redirection() => ProviderError::malformed(format!(
            "mtn_momo: token endpoint answered a redirect (HTTP {status}), which is not \
             followed; check base_url"
        )),
        _ => ProviderError::malformed(format!(
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

    /// A fingerprint no `Credentials` in this file produces, for the tests
    /// that are about the clock and not about tenancy.
    const ANY: [u8; 32] = [7_u8; 32];

    /// The deadline `cache_entry` gives a token, read back through the only
    /// thing that consumes it. Asserting on `usable` rather than on the
    /// arithmetic means these tests exercise the path that ships.
    fn is_usable_at(expires_in: Option<u64>, minted_at: Instant, now: Instant) -> bool {
        cache_entry("t".to_owned(), ANY, minted_at, expires_in)
            .usable(now, &ANY)
            .is_some()
    }

    #[test]
    fn a_token_expires_a_minute_before_the_rail_says_it_does() {
        let now = Instant::now();
        assert!(is_usable_at(
            Some(3_600),
            now,
            now + Duration::from_secs(3_540) - Duration::from_millis(1)
        ));
        assert!(!is_usable_at(
            Some(3_600),
            now,
            now + Duration::from_secs(3_540)
        ));
    }

    /// MTN's margin, named and pinned in its own right.
    ///
    /// The margin is the one number the two adapters' token caches must not
    /// share: MTN's and Orange's are separately reasoned constants that
    /// happen to agree today. Every other assertion in this file would still
    /// pass if this adapter silently started using the other one's, so this
    /// test asserts the constant itself, both arithmetic branches that
    /// consume it, and the fact that it is subtracted from the rail's stated
    /// lifetime rather than added to it.
    #[test]
    fn the_refresh_margin_mtn_applies_is_sixty_seconds_of_the_rails_own_lifetime() {
        assert_eq!(REFRESH_MARGIN, Duration::from_secs(60));

        let now = Instant::now();
        let stated = Duration::from_secs(3_600);
        assert!(
            !is_usable_at(Some(3_600), now, now + stated - REFRESH_MARGIN),
            "a stated lifetime is reduced by MTN's margin, never extended by it"
        );
        assert!(
            is_usable_at(
                Some(3_600),
                now,
                now + stated - REFRESH_MARGIN - Duration::from_millis(1)
            ),
            "a millisecond earlier it is still the rail's to honour"
        );
        assert!(
            !is_usable_at(None, now, now + ASSUMED_LIFETIME - REFRESH_MARGIN),
            "the assumed lifetime is reduced by the same margin"
        );
        assert!(is_usable_at(
            None,
            now,
            now + ASSUMED_LIFETIME - REFRESH_MARGIN - Duration::from_millis(1)
        ));
    }

    /// A stated lifetime shorter than the margin must clamp to "already
    /// expired", not wrap around into a token that outlives the process.
    #[test]
    fn a_lifetime_shorter_than_the_margin_expires_immediately() {
        let now = Instant::now();
        assert!(!is_usable_at(Some(30), now, now));
        assert!(!is_usable_at(Some(0), now, now));
    }

    #[test]
    fn a_rail_that_states_no_lifetime_gets_a_short_one() {
        let now = Instant::now();
        assert!(
            is_usable_at(None, now, now),
            "an unstated lifetime must still be usable"
        );
        assert!(
            !is_usable_at(None, now, now + Duration::from_secs(3_600)),
            "an unstated lifetime must not be assumed to be an hour"
        );
    }

    #[test]
    fn a_cached_token_is_reused_only_before_expiry_and_only_for_its_own_credentials() {
        let now = Instant::now();
        let mine = [1_u8; 32];
        let theirs = [2_u8; 32];
        let token = cache_entry("secret".to_owned(), mine, now, Some(3_600));

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
            cache_entry("super-secret".to_owned(), [0_u8; 32], Instant::now(), None)
        );
        assert!(!rendered.contains("super-secret"), "{rendered}");
    }
}
