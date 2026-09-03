//! The OAuth2 client-credentials token, and where to ask for it.
//!
//! Two things live here because they are the two things about Orange's auth
//! that are easy to get quietly wrong: the token endpoint is not under the
//! configured base URL, and the token must not be re-minted per call nor
//! shared between merchants.
//!
//! # Why this cache is duplicated in the MTN adapter
//!
//! `vpay-adapter-mtn-momo`'s `token` module holds a second [`CachedToken`]
//! with the same shape, and that duplication is deliberate for now rather
//! than an oversight. The two token endpoints agree on almost nothing that a
//! shared type could absorb: this one posts OAuth2 client-credentials to the
//! *host root* ([`TOKEN_PATH`], derived by [`token_url`] rather than
//! appended to the configured base URL) and answers a body carrying
//! `expires_in` as a required field, while MTN mints from HTTP Basic
//! (`api_user:api_key`) plus a subscription-key header, on a path *under*
//! its base URL, and may omit the lifetime entirely. A shared `CachedToken`
//! in `vpay-provider` is the right end state, and the follow-up is worth
//! doing once a third rail exists to show which parts are actually common —
//! generalising from two examples would fix the wrong axis.
//!
//! Until then, two properties MUST stay aligned across both modules, because
//! each was a real defect in one of them and a divergence is silent:
//!
//! 1. **`minted_at` is recorded before the token request is sent**, never
//!    from the clock after the response arrives — see
//!    [`CachedToken::new`], which takes it as a parameter for that reason.
//!    This module did not do it until the Step 3 security review.
//! 2. **The secret half of the credentials is in the fingerprint**
//!    ([`fingerprint`] hashes `client_secret`, not just `client_id`), so a
//!    rotation evicts the cache immediately instead of leaving a bearer
//!    minted from a revoked secret in use until it ages out.
//!
//! Change either property here and change it there in the same commit.

use std::fmt;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use vpay_provider::ProviderError;

/// The token endpoint's path, at the host root — *not* under
/// `/orange-money-webpay/{env}`. See [`token_url`].
const TOKEN_PATH: &str = "/oauth/v2/token";

/// How far before the rail's stated expiry a cached token is retired.
///
/// A token that expires in flight is a 401 on a payment call, which costs a
/// round trip and, on `submit`, risks a duplicate on a rail whose idempotency
/// we are reconstructing from community SDKs. Sixty seconds is comfortably
/// more than a request budget ([`vpay_provider::DEFAULT_REQUEST_TIMEOUT`] is
/// 20 s) plus any plausible clock skew between us and Orange.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

/// The token endpoint for a deployment whose `base_url` is
/// `…/orange-money-webpay/{env}`.
///
/// Orange puts the environment in the *path* of the payment API but serves
/// OAuth from the host root, so the two URLs cannot both be derived by
/// appending. Deriving the token URL from the same configured value — rather
/// than adding a second YAML key — means a deployment cannot point its
/// payments at one host and its tokens at another, which is a
/// misconfiguration that would only surface as a 401 storm in production.
///
/// Scheme, host, port and any userinfo are kept; path, query and fragment are
/// replaced. `Url::join` of a path-absolute reference is exactly that
/// operation, per RFC 3986 §5.3.
///
/// # Errors
///
/// [`ProviderError::Config`] if `base` is not an absolute URL — a
/// deployment-time mistake in `providers[].host.url`, which is what
/// `Category::Configuration` and exit 78 exist for.
pub(crate) fn token_url(base: &str) -> Result<String, ProviderError> {
    let base = reqwest::Url::parse(base).map_err(|error| {
        ProviderError::Config(format!(
            "orange_money: base_url {base:?} is not an absolute URL: {error}"
        ))
    })?;
    let token = base.join(TOKEN_PATH).map_err(|error| {
        ProviderError::Config(format!(
            "orange_money: cannot derive the token endpoint from base_url {base}: {error}"
        ))
    })?;
    Ok(token.into())
}

/// A SHA-256 over **both** halves of the client credentials a token was
/// minted for.
///
/// Two distinct jobs, and it took the Step 3 security review to notice that
/// the second was not being done:
///
/// 1. *Tenancy.* The port hands `&ProviderConfig` to every call, so one
///    `Adapter` value can legitimately serve two merchants on the same rail.
///    Caching a token without recording whose it is would send merchant B's
///    charges under merchant A's credentials — money moving on the wrong
///    account, and no test would see it because both configurations are
///    valid. `client_id` alone answers this one.
/// 2. *Rotation.* A `client_secret` is rotated precisely when the old one
///    must stop working **now**. Hashing only the `client_id` meant the
///    fingerprint still matched after a rotation, so the cache still hit and
///    the bearer minted from the revoked secret kept being sent until it
///    aged out (up to an hour) or the rail answered 401. Including the
///    secret makes the eviction immediate and automatic.
///
/// A digest rather than the values themselves, so the cache holds no
/// credential material at rest — which is what makes hashing a secret in
/// here safe. Each field is length-prefixed so `("ab", "c")` and
/// `("a", "bc")` cannot collide by concatenating differently; the whole
/// point of the key is that distinct credentials are distinct.
pub(crate) fn fingerprint(client_id: &str, client_secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for field in [client_id, client_secret] {
        let length = u64::try_from(field.len()).unwrap_or(u64::MAX);
        hasher.update(length.to_be_bytes());
        hasher.update(field.as_bytes());
    }
    hasher.finalize().into()
}

/// One rail token, in memory only.
///
/// Never persisted: a token is short-lived, re-mintable from the credentials
/// we already hold, and writing it to the database would put a bearer for a
/// merchant's payment account into backups and replicas for no benefit.
pub(crate) struct CachedToken {
    value: String,
    expires_at: Instant,
    fingerprint: [u8; 32],
}

/// Hand-written so the bearer cannot reach a log line through a `?token`
/// field on the enclosing `Adapter`'s derived `Debug`.
impl fmt::Debug for CachedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedToken")
            .field("value", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl CachedToken {
    /// `lifetime` is the rail's `expires_in`; [`EXPIRY_MARGIN`] is subtracted
    /// here rather than at the read site so no caller can forget it.
    ///
    /// `minted_at` is the caller's, taken **before** the token request was
    /// sent rather than read from the clock here. The rail's `expires_in`
    /// counts from the moment the rail minted the token, so measuring from
    /// the moment the response finished arriving silently grants it the
    /// round trip as extra life — which on a slow rail is the whole of
    /// [`EXPIRY_MARGIN`]. MTN's adapter has always done this; this one did
    /// not until the Step 3 security review.
    pub(crate) fn new(
        value: String,
        minted_at: Instant,
        lifetime: Duration,
        fingerprint: [u8; 32],
    ) -> Self {
        let usable_for = lifetime.saturating_sub(EXPIRY_MARGIN);
        Self {
            value,
            // `Instant + Duration` panics on overflow, and a rail answering an
            // absurd `expires_in` must not be able to bring down a worker. An
            // overflow yields "already expired", which costs a re-mint.
            expires_at: minted_at.checked_add(usable_for).unwrap_or(minted_at),
            fingerprint,
        }
    }

    /// The token, iff it was minted for these credentials and is still inside
    /// its margin.
    pub(crate) fn usable(&self, fingerprint: &[u8; 32]) -> Option<&str> {
        (self.fingerprint == *fingerprint && Instant::now() < self.expires_at)
            .then_some(self.value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The deployment's own value, compiled in. `include_str!` rather than a
    /// runtime read so moving or renaming the file fails the build here
    /// instead of failing a test with "file not found" — and so this test
    /// cannot pass against a stale copy.
    const APPLICATION_YML: &str = include_str!("../../../../config/application.yml");

    /// The configured `base_url` this adapter must handle today. If
    /// `config/application.yml` stops containing it, the assertion below
    /// fails and whoever changed the YAML learns that the token URL is
    /// derived from it.
    const CONFIGURED_BASE: &str = "http://wiremock-orange:8080/orange-money-webpay/dev";

    #[test]
    fn the_token_url_is_derived_from_the_configured_base_url() {
        assert!(
            APPLICATION_YML.contains(CONFIGURED_BASE),
            "config/application.yml no longer configures {CONFIGURED_BASE}; \
             update this test and check the derivation still holds"
        );
        assert_eq!(
            token_url(CONFIGURED_BASE).expect("a configured base URL parses"),
            "http://wiremock-orange:8080/oauth/v2/token",
            "the environment lives in the payment API's path; OAuth is at the host root"
        );
    }

    #[test]
    fn the_token_url_drops_the_environment_prefix_on_the_real_host_too() {
        assert_eq!(
            token_url("https://api.orange.com/orange-money-webpay/dev")
                .expect("the documented production-shaped base URL parses"),
            "https://api.orange.com/oauth/v2/token"
        );
    }

    /// A trailing slash, a query string and a deeper prefix are all the same
    /// derivation: everything after the authority is replaced.
    #[test]
    fn the_derivation_ignores_everything_after_the_authority() {
        for base in [
            "https://api.orange.com/orange-money-webpay/dev/",
            "https://api.orange.com/orange-money-webpay/cm?v=1",
            "https://api.orange.com/a/b/c/d",
            "https://api.orange.com",
        ] {
            assert_eq!(
                token_url(base).expect("parses"),
                "https://api.orange.com/oauth/v2/token",
                "base {base}"
            );
        }
    }

    #[test]
    fn a_base_url_that_is_not_a_url_is_a_configuration_error() {
        let error = token_url("wiremock-orange:8080").expect_err("no scheme, no derivation");
        assert!(matches!(error, ProviderError::Config(_)), "{error:?}");
    }

    #[test]
    fn a_token_is_only_reused_for_the_credentials_it_was_minted_for() {
        let a = fingerprint("merchant-a-client-id", "merchant-a-secret");
        let b = fingerprint("merchant-b-client-id", "merchant-b-secret");
        let cached = CachedToken::new(
            "secret-bearer".to_owned(),
            Instant::now(),
            Duration::from_secs(3600),
            a,
        );

        assert_eq!(cached.usable(&a), Some("secret-bearer"));
        assert_eq!(
            cached.usable(&b),
            None,
            "a second merchant on the same rail must not borrow the first's token"
        );
    }

    /// The rotation case, and the one the old `client_id`-only fingerprint
    /// got wrong: the id is unchanged and only the secret has been rolled.
    /// A cache that still hits here keeps sending a bearer minted from a
    /// secret the operator has just revoked, for as long as an hour.
    #[test]
    fn rotating_only_the_secret_evicts_the_cached_bearer() {
        let before = fingerprint("client", "old-secret");
        let after = fingerprint("client", "new-secret");
        let cached = CachedToken::new(
            "bearer-from-the-old-secret".to_owned(),
            Instant::now(),
            Duration::from_secs(3600),
            before,
        );

        assert_eq!(cached.usable(&before), Some("bearer-from-the-old-secret"));
        assert_eq!(
            cached.usable(&after),
            None,
            "a rotated client_secret must evict the token minted from the old one on the very \
             next call, not when it ages out"
        );
    }

    /// The length prefix, proven the way MTN's is: with a plain
    /// `hash(id + secret)` these two credential pairs would share a token,
    /// and one merchant's charges would go out under another's account.
    #[test]
    fn a_field_boundary_cannot_be_shifted_into_a_collision() {
        assert_ne!(fingerprint("ab", "c"), fingerprint("a", "bc"));
    }

    /// A token's life is counted from before the request was sent, so a slow
    /// rail cannot silently extend it: a token minted a full lifetime ago is
    /// already unusable, however recently `CachedToken::new` was called.
    #[test]
    fn the_lifetime_is_measured_from_the_send_not_from_the_answer() {
        let fp = fingerprint("client", "secret");
        let long_ago = Instant::now()
            .checked_sub(Duration::from_secs(3_600))
            .expect("a loopback-scale instant arithmetic does not underflow");
        assert_eq!(
            CachedToken::new("t".to_owned(), long_ago, Duration::from_secs(3_600), fp).usable(&fp),
            None,
            "an hour-old token with an hour of stated life is spent, whatever the clock said \
             when the response arrived"
        );
    }

    /// The margin is the point: a token the rail says lives 30 s is already
    /// unusable, because it would expire inside a request budget.
    #[test]
    fn a_token_inside_the_expiry_margin_is_already_stale() {
        let fp = fingerprint("client", "secret");
        let now = Instant::now();
        assert_eq!(
            CachedToken::new("t".to_owned(), now, Duration::from_secs(30), fp).usable(&fp),
            None
        );
        assert_eq!(
            CachedToken::new("t".to_owned(), now, Duration::from_secs(3600), fp).usable(&fp),
            Some("t")
        );
    }

    #[test]
    fn an_absurd_lifetime_does_not_panic() {
        let fp = fingerprint("client", "secret");
        let cached = CachedToken::new("t".to_owned(), Instant::now(), Duration::MAX, fp);
        // Either answer is acceptable; not panicking is the assertion.
        let _ = cached.usable(&fp);
    }

    #[test]
    fn debug_never_prints_the_bearer() {
        let cached = CachedToken::new(
            "super-secret-bearer".to_owned(),
            Instant::now(),
            Duration::from_secs(3600),
            fingerprint("client", "secret"),
        );
        let rendered = format!("{cached:?}");
        assert!(!rendered.contains("super-secret-bearer"), "{rendered}");
    }
}
