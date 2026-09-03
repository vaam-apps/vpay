//! The OAuth2 client-credentials token, and where to ask for it.
//!
//! Two things live here because they are the two things about Orange's auth
//! that are easy to get quietly wrong: the token endpoint is at the host root
//! and not under the configured base URL ([`token_url`]), and the token must
//! not be re-minted per call nor shared between merchants ([`cache_entry`],
//! [`fingerprint`]).
//!
//! The cache entry itself is [`vpay_provider::token::CachedToken`]; what is
//! Orange's alone is [`EXPIRY_MARGIN`] and the two fields the fingerprint
//! hashes. `docs/reference/rails.md` has the rest: why the cache is keyed by
//! a credential digest, why the margin is per-rail rather than shared, and
//! the two invariants — `minted_at` read before the send, the secret half in
//! the fingerprint — that were each a real defect in this module once.

use std::time::{Duration, Instant};

use vpay_provider::ProviderError;
use vpay_provider::token::CachedToken;

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
/// appending. Deriving both from one configured value — rather than adding a
/// second YAML key — is what stops a deployment pointing its payments at one
/// host and its tokens at another. Scheme, host, port and userinfo are kept;
/// path, query and fragment are replaced, which is exactly `Url::join` of a
/// path-absolute reference (RFC 3986 §5.3).
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

/// The cache key: a digest over **both** halves of the client credentials a
/// token was minted for.
///
/// `client_secret` is in it and not just `client_id`, which is the half that
/// took a security review to notice: a rotated secret must evict the bearer
/// minted from the old one on the *next* call, not when it ages out.
/// `docs/reference/rails.md` records the tenancy argument for the other half.
pub(crate) fn fingerprint(client_id: &str, client_secret: &str) -> [u8; 32] {
    vpay_provider::token::fingerprint(&[client_id, client_secret])
}

/// The cache entry for a token Orange has just minted.
///
/// Named rather than a bare [`CachedToken::new`] at the one call site,
/// because the one Orange-specific thing about a cached token is
/// [`EXPIRY_MARGIN`] — subtracted here rather than at the read site so no
/// caller can forget it. `minted_at` is the caller's and must be read
/// **before** the token request was sent; `docs/reference/rails.md` says what
/// goes wrong when it is not.
pub(crate) fn cache_entry(
    value: String,
    minted_at: Instant,
    lifetime: Duration,
    fingerprint: [u8; 32],
) -> CachedToken {
    CachedToken::new(value, minted_at, lifetime, EXPIRY_MARGIN, fingerprint)
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
        let cached = cache_entry(
            "secret-bearer".to_owned(),
            Instant::now(),
            Duration::from_secs(3600),
            a,
        );

        assert_eq!(cached.usable(Instant::now(), &a), Some("secret-bearer"));
        assert_eq!(
            cached.usable(Instant::now(), &b),
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
        let cached = cache_entry(
            "bearer-from-the-old-secret".to_owned(),
            Instant::now(),
            Duration::from_secs(3600),
            before,
        );

        assert_eq!(
            cached.usable(Instant::now(), &before),
            Some("bearer-from-the-old-secret")
        );
        assert_eq!(
            cached.usable(Instant::now(), &after),
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
            cache_entry("t".to_owned(), long_ago, Duration::from_secs(3_600), fp)
                .usable(Instant::now(), &fp),
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
            cache_entry("t".to_owned(), now, Duration::from_secs(30), fp)
                .usable(Instant::now(), &fp),
            None
        );
        assert_eq!(
            cache_entry("t".to_owned(), now, Duration::from_secs(3600), fp)
                .usable(Instant::now(), &fp),
            Some("t")
        );
    }

    /// Orange's margin, named and pinned in its own right.
    ///
    /// The margin is the one number the two adapters' token caches must not
    /// share: Orange's and MTN's are separately reasoned constants that
    /// happen to agree today. Every other assertion in this file would still
    /// pass if this adapter silently started using the other one's, so this
    /// test asserts the constant itself and the boundary it puts on a token
    /// minted with a known lifetime.
    ///
    /// The five-second step is what makes this a statement about *sixty*
    /// seconds rather than about any margin at all: at the boundary the token
    /// is spent, five seconds earlier it is not.
    #[test]
    fn the_expiry_margin_orange_applies_is_sixty_seconds_of_the_rails_own_lifetime() {
        assert_eq!(EXPIRY_MARGIN, Duration::from_secs(60));

        let fp = fingerprint("client", "secret");
        let lifetime = Duration::from_secs(3_600);
        let spent_at = Instant::now()
            .checked_sub(lifetime - EXPIRY_MARGIN)
            .expect("a process-scale instant does not underflow");
        assert_eq!(
            cache_entry("t".to_owned(), spent_at, lifetime, fp).usable(Instant::now(), &fp),
            None,
            "the margin is subtracted from the rail's stated lifetime, never added to it"
        );

        let inside = spent_at
            .checked_add(Duration::from_secs(5))
            .expect("a process-scale instant does not overflow");
        assert_eq!(
            cache_entry("t".to_owned(), inside, lifetime, fp).usable(Instant::now(), &fp),
            Some("t"),
            "five seconds before the boundary the token is still the rail's to honour"
        );
    }

    #[test]
    fn an_absurd_lifetime_does_not_panic() {
        let fp = fingerprint("client", "secret");
        let cached = cache_entry("t".to_owned(), Instant::now(), Duration::MAX, fp);
        // Either answer is acceptable; not panicking is the assertion.
        let _ = cached.usable(Instant::now(), &fp);
    }

    #[test]
    fn debug_never_prints_the_bearer() {
        let cached = cache_entry(
            "super-secret-bearer".to_owned(),
            Instant::now(),
            Duration::from_secs(3600),
            fingerprint("client", "secret"),
        );
        let rendered = format!("{cached:?}");
        assert!(!rendered.contains("super-secret-bearer"), "{rendered}");
    }
}
