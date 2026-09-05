//! Resource-server JWT validation for vpay's two protected HTTP surfaces:
//! `/v1` (merchant, ADR-0010) and `/dash/v1` (dashboard, ADR-0009).
//!
//! STATUS: the merchant half is live and guards real rows —
//! [`crate::require_merchant_token`] validates through [`MerchantJwtValidator`]
//! and is mounted in front of the whole `/v1` nest. The dashboard half is
//! unmounted: [`AuthenticatedDashboard`] exists, no `/dash/v1/*` route does,
//! and nothing constructs a [`DashboardJwtValidator`].
//!
//! Validation is local — `crate::jwks_cache` (private) caches the JWKS and every check
//! after the first verifies the signature from memory. The one input that can
//! still force a network fetch is an *unrecognized* `kid`, which is why this
//! module throttles: see `UNKNOWN_KID_REFRESH_INTERVAL` (private, in this file).
//!
//! Why the throttle exists, the `jsonwebtoken` audience edge
//! [`JwtValidator::new`] closes with `set_required_spec_claims`, and why
//! `authkestra_resource::jwt::JwtStrategy` was not used:
//! [docs/reference/vpay-api.md § resource-server JWT validation](../../../../docs/reference/vpay-api.md#resource-server-jwt-validation-resource_authrs).

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use authkestra_resource::jwt::ValidationError;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::header;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use jsonwebtoken::{Algorithm, Validation, decode_header};
use serde::Deserialize;
use vpay_core::{Category, Classify};

use crate::ApiError;
use crate::http_client::HttpClientError;
use crate::jwks_cache::{JwksCache, validate_with_jwks};

/// How long one process waits between letting an *unrecognized* `kid` force
/// a JWKS refresh — at most one forced refresh per interval per process, no
/// matter how many junk tokens arrive.
///
/// Why a throttle exists at all is in this module's header: an unknown `kid`
/// is the one input that turns a bearer token into a remote JWKS fetch plus
/// a write lock held across it, and it is checked *before* any signature is
/// verified, so an unauthenticated caller controls it for free.
///
/// # The trade-off, stated plainly
///
/// Under a burst of junk `kid`s, a token signed by a key that is **not in
/// the JWKS this process currently holds** can be refused for up to 30
/// seconds — it looks exactly like the junk until the refresh that would
/// tell the two apart is allowed to happen. Concretely that is a key
/// published *since* this validator's last JWKS refresh, i.e. a rotation
/// mid-flight; every key already in the cached document is unaffected,
/// throttle or no throttle (see [`JwtValidator::validate`]'s "Why
/// membership of the JWKS" section). That residue is accepted because of
/// how rotation actually works in this deployment:
///
/// - rotation is restart-based (`vpay-server`'s `main` loads one PEM from a
///   Secret mount at boot and calls `ensure_active_in_database`), and the
///   JWKS is fetched fresh by every process that starts, so a replica that
///   rolls out with the new key does not depend on this path at all;
/// - the ordinary `jwks_refresh_interval` (300 s, set by the caller of
///   [`JwtValidator::new`]) keeps running regardless, so the new key becomes
///   known within that window with no unknown-`kid` request needed;
/// - the failure mode it replaces is worse in kind, not merely in degree: an
///   unauthenticated 401 amplified into a database query and a lock that
///   stalls *every* concurrent validation, including the ones that would
///   have succeeded.
///
/// 30 seconds rather than the 300 s cache TTL: long enough that a burst
/// cannot turn into a fetch loop, short enough that a genuine
/// mid-rotation token is refused for a fraction of the window it would
/// otherwise be, and it is a bound on *this* process, so N replicas make at
/// most N such fetches.
const UNKNOWN_KID_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

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
    /// `Merchant` returns [`vpay_config::MERCHANT_AUDIENCE`] rather than its
    /// own copy of the string, because the same value has to be *registered*
    /// in every merchant's `allowed_audiences` for the OP to mint a token
    /// carrying it at all — and a mismatch between the two spellings has no
    /// visible symptom other than a bare `401` on every `/v1` call. Config
    /// owns it; `vpay_config::ConfigError::MerchantMissingV1Audience`
    /// refuses to boot a registration that cannot target it. (This used to
    /// be a local literal marked "provisional"; it is no longer either.)
    ///
    /// `Dashboard` is still a local literal: nothing registers or validates
    /// it yet — `/dash/v1` login is later work — so there is no second party
    /// for it to drift from.
    #[must_use]
    pub fn audience(self) -> &'static str {
        match self {
            Surface::Merchant => vpay_config::MERCHANT_AUDIENCE,
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
#[serde(rename_all = "snake_case")]
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
/// into the Stripe-shaped envelope this crate renders in one place
/// (`error_envelope_with_param`, `pub(crate)` and reached only through
/// [`ApiError`]'s `IntoResponse` — hence no intra-doc link to it from a
/// public item).
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
    /// The token could not be *checked at all*: this process could not
    /// reach, or could not read, the JWKS it verifies signatures against.
    ///
    /// Deliberately **not** [`Self::InvalidToken`], and the difference is
    /// not cosmetic. A 401 tells an SDK its credential is stale, and every
    /// SDK's answer to that is to go back to the token endpoint and
    /// re-authenticate — so a JWKS outage rendered as 401 turns into a
    /// stampede on `/v1/oauth/token` (itself database-backed) at exactly the
    /// moment the deployment is least able to serve it. Classified as
    /// [`Category::Storage`] below, this answers `503` with
    /// `Retry::AfterBackoff` instead, which is what makes an SDK wait rather
    /// than retry a credential that was never the problem.
    ///
    /// The `Display` names the condition and nothing else — no JWKS URL, no
    /// `reqwest` error text. It reaches a log, and an operator who needs the
    /// cause has the JWKS URL in their own configuration and the fetch
    /// failure in the JWKS endpoint's own logs; repeating a URL and a
    /// library error here would put an internal hostname into a line that a
    /// merchant's request produced, for no diagnostic that is not already
    /// available.
    #[error("the JWKS this surface validates against could not be fetched")]
    KeysUnavailable,
}

impl Classify for AuthRejection {
    /// One category for the three *credential* rejections. Which check
    /// tripped is not the caller's business (see the type's own doc
    /// comment), and [`Category::Authentication`] is what turns that into
    /// 401 + `authentication_error` at every boundary at once.
    ///
    /// [`Self::KeysUnavailable`] is the exception, and it is the one place
    /// this type says something about vpay rather than about the token:
    /// [`Category::Storage`] is 503 + `Retry::AfterBackoff`, i.e. "we could
    /// not answer, come back" rather than "your credential is bad". It says
    /// nothing about the token itself, so it is not the oracle the collapse
    /// above exists to prevent.
    fn category(&self) -> Category {
        match self {
            Self::MissingHeader | Self::MalformedHeader | Self::InvalidToken => {
                Category::Authentication
            }
            Self::KeysUnavailable => Category::Storage,
        }
    }

    /// Per-variant, unlike the message. A code is a stable identifier an SDK
    /// branches on, and the first three are actionable in different ways by
    /// the *legitimate* caller — "you sent no header" and "your token
    /// expired" need different fixes. They reveal nothing about the token's
    /// contents, which is where the oracle risk actually lives;
    /// `InvalidToken` is the single code behind which every validation
    /// failure hides.
    ///
    /// `KeysUnavailable` deliberately takes `Category::Storage`'s default
    /// code rather than inventing one: to a caller it is the same event as
    /// any other "vpay is temporarily unavailable" — the retry policy is
    /// identical and there is nothing a merchant could do differently — and
    /// a public vocabulary that grows a code per internal cause is one SDKs
    /// end up branching on. The distinction that *is* useful is the
    /// operator's, and it lives in the `Display` above, which goes to the
    /// log.
    fn code(&self) -> &'static str {
        match self {
            Self::MissingHeader => "missing_bearer_token",
            Self::MalformedHeader => "malformed_authorization_header",
            Self::InvalidToken => "invalid_token",
            Self::KeysUnavailable => Category::Storage.default_code(),
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
            // "vpay is temporarily unavailable. Retry after a short delay."
            // — the category's own sentence, which is exactly right here and
            // says nothing about the token or about what vpay could not
            // reach.
            Self::KeysUnavailable => Category::Storage.generic_message(),
        }
        .to_owned()
    }
}

impl From<ValidationError> for AuthRejection {
    /// Two outcomes, and the split is "was the *credential* bad" versus
    /// "could we not check it".
    ///
    /// Every claim, signature and key-resolution failure still collapses to
    /// [`AuthRejection::InvalidToken`] — that is the oracle argument on the
    /// type's own doc comment and it is unchanged. Naming the variants that
    /// reach this `From` from [`JwtValidator::validate`], as of the `0.7.1`
    /// pin (`authkestra-resource-0.7.1/src/jwt.rs`):
    ///
    /// | Variant | Maps to | Why |
    /// |---|---|---|
    /// | `Http(reqwest::Error)` | `KeysUnavailable` | The only error `Jwks::fetch_with` can produce: `client.get(uri).send().await?.json::<Jwks>().await?` makes a refused connection, a timeout, a 5xx and an unparseable JWKS body all one `reqwest::Error`. Every one of them is "this process could not obtain keys", none is about the token. |
    /// | `Discovery(AuthError)` | `KeysUnavailable` | Produced by `Jwk::to_decoding_key` when a published JWK cannot be turned into a verifying key, and — upstream — by the discovery half of the same fetch path. Either way this process could not obtain a usable key; neither is about the token. |
    /// | `Jwt(_)`, `InvalidToken(_)`, `Validation(_)` | `InvalidToken` | Signature, expiry, audience, issuer, malformed header — the caller's credential. |
    /// | `KeyNotFound`, `MissingKid`, `MissingIssuer`, `UntrustedIssuer(_)` | `InvalidToken` | The token names a key or an issuer this surface does not accept. A 503 here would let a caller with a made-up `kid` provoke a "vpay is down" answer at will. |
    /// | `Serialization(_)`, `Paseto(_)`, `DpopReplayUnavailable(_)` | `InvalidToken` | Unreachable from this call path — no `serde_json` step, no PASETO, no DPoP proof — and covered by the wildcard below rather than named, since the enum is `#[non_exhaustive]`. |
    ///
    /// The wildcard fails **closed**: a variant added by a future
    /// `authkestra-resource` lands on `InvalidToken` (401, the caller's
    /// problem to fix) rather than on `KeysUnavailable` (503, "retry, it is
    /// us"). Answering "retry" to something that will never succeed is the
    /// worse of the two mistakes, because a retry loop against a payment
    /// gateway costs both parties.
    fn from(error: ValidationError) -> Self {
        match error {
            ValidationError::Http(_) | ValidationError::Discovery(_) => {
                AuthRejection::KeysUnavailable
            }
            _ => AuthRejection::InvalidToken,
        }
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
///
/// `pub(crate)` because the `/v1` boundary is a *middleware* now, not an
/// extractor (Step 2's D3): [`crate::require_merchant_token`] runs this and
/// [`JwtValidator::validate`] exactly once per request and puts the result
/// on the request's extensions. It stays private to the crate — the header
/// parsing and the rejection vocabulary belong together, and a caller
/// outside this crate has no business turning an `Authorization` header into
/// anything but a rejection or a validated [`ResourceClaims`].
pub(crate) fn extract_bearer_token(parts: &Parts) -> Result<&str, AuthRejection> {
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
/// required audience, one required issuer. Cheap to clone (the cache and the
/// two pieces of throttle state are all `Arc`-shared) so it can live in axum
/// state alongside a database pool.
///
/// Cloning shares the throttle rather than copying it, and that is
/// load-bearing: axum clones the router state per request, so a per-clone
/// throttle would be no throttle at all.
#[derive(Clone)]
pub struct JwtValidator {
    cache: Arc<JwksCache>,
    validation: Validation,
    /// Every `kid` this validator has found *in the JWKS it fetched* — a
    /// memo of `crate::jwks_cache::JwksCache::get_jwks` lookups, not a record of accepted
    /// tokens. Nothing an unauthenticated caller sends can add to it (only
    /// a key this deployment's own OP published can), which is what makes
    /// it safe as the "delegating cannot force a refresh" predicate in
    /// [`JwtValidator::validate`], and why it needs no size cap: it is
    /// bounded by the number of keys the OP has actually published over the
    /// process's lifetime, not by request volume.
    known_kids: Arc<RwLock<HashSet<String>>>,
    /// When an unknown `kid` was last allowed through to the JWKS cache.
    /// `None` until the first one. See `UNKNOWN_KID_REFRESH_INTERVAL`.
    last_unknown_kid_refresh: Arc<Mutex<Option<Instant>>>,
}

impl fmt::Debug for JwtValidator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The ported `JwksCache` has no `Debug` impl of its own; the validation policy
        // (audience, issuer, required claims) is the useful, non-sensitive
        // half to show.
        f.debug_struct("JwtValidator")
            .field("validation", &self.validation)
            .finish_non_exhaustive()
    }
}

impl JwtValidator {
    /// `jwks_url` is polled at most once per `jwks_refresh_interval`, plus
    /// at most once per `UNKNOWN_KID_REFRESH_INTERVAL` on an unrecognized
    /// `kid` (to tolerate an in-flight rotation without letting a caller
    /// choose how often that happens) — see the module doc for why this is
    /// not a per-request network call, and
    /// [`JwtValidator::validate`] for the decision it makes before
    /// delegating. `require_kid(true)` on the underlying cache: a token
    /// presented with no `kid` header is rejected rather than silently
    /// matched against the first key in the JWKS, which matters the moment
    /// the JWKS ever holds more than one key during a rotation window.
    ///
    /// # Why the JWKS client is supplied rather than defaulted
    ///
    /// `authkestra_resource::jwt::JwksCache::new` builds its own client with
    /// `reqwest::Client::new()`, which under this workspace's reqwest 0.13
    /// pin reads the **platform** trust store eagerly and panics when it is
    /// empty. The runtime image is `FROM scratch` and has no trust store, so
    /// that default made `vpay-server` die at boot inside its own image —
    /// while passing every test on a machine with `/etc/ssl`, and although
    /// the JWKS URL it was about to fetch is plain `http://` over loopback.
    /// [`crate::http_client::client`] supplies a client with the roots
    /// compiled in, and `crate::jwks_cache::JwksCache` — a port of the
    /// authkestra cache that takes the client as a constructor argument —
    /// is what makes handing it over possible at all. Both module docs have
    /// the full account, including why `JwksCache::with_client` is not the
    /// seam it appears to be.
    ///
    /// # Errors
    ///
    /// [`HttpClientError`] if that client cannot be built. Fallible rather
    /// than `#[must_use] -> Self` because the alternative under this crate's
    /// no-panic lint policy would be to swallow the failure and hand back a
    /// validator that rejects every token for a reason no log explains.
    pub fn new(
        jwks_url: impl Into<String>,
        jwks_refresh_interval: Duration,
        issuer: impl Into<String>,
        surface: Surface,
    ) -> Result<Self, HttpClientError> {
        let cache = JwksCache::new(
            jwks_url.into(),
            jwks_refresh_interval,
            crate::http_client::client()?,
        );

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[issuer.into()]);
        validation.set_audience(&[surface.audience()]);
        // See the module doc's "sharp edge" section: `aud` is only checked
        // by `validate_aud` when the claim is present at all, so its
        // presence has to be required explicitly, separately from its value.
        validation.set_required_spec_claims(&["exp", "aud", "iss"]);

        Ok(Self {
            cache: Arc::new(cache),
            validation,
            known_kids: Arc::new(RwLock::new(HashSet::new())),
            last_unknown_kid_refresh: Arc::new(Mutex::new(None)),
        })
    }

    /// Validates signature, expiry, issuer and audience, and returns the
    /// claims a handler needs. Fails closed: any ambiguity (unknown `kid`,
    /// missing claim, expired, wrong audience or issuer, bad signature) is
    /// `Err`, never a best-effort `Ok`.
    ///
    /// # Why the header is decoded here, before delegating
    ///
    /// Key resolution happens *before* anything about the token is verified
    /// — so by the time the delegation has looked at the token, it has
    /// already done the expensive, remotely-triggerable part (see this
    /// module's header). This function therefore does the cheap, local part
    /// first and decides whether delegating is free:
    ///
    /// 1. no `kid` in the header (or a header that will not decode at all) →
    ///    [`AuthRejection::InvalidToken`] immediately, with no cache access.
    ///    The cache is built with `require_kid(true)`, so this is the same
    ///    verdict it would reach — reached without touching it;
    /// 2. a `kid` this validator has already seen *in the JWKS* → delegate.
    ///    The key is in the cache, so the delegation is a cache hit and a
    ///    cache hit never forces `get_key`'s rotation refresh. It still
    ///    takes the ordinary TTL refresh when one is due, which is one
    ///    fetch for the whole process however many requests are in flight;
    /// 3. otherwise, consult the cached JWKS once
    ///    (`crate::jwks_cache::JwksCache::get_jwks`,
    ///    which fetches only when the cache is cold or its TTL has elapsed,
    ///    and coalesces concurrent callers onto one fetch when it does —
    ///    bounded by *time*, never by request rate or concurrency). If the
    ///    key is there, it is remembered for step 2 and the delegation proceeds;
    /// 4. only a `kid` the JWKS genuinely does not contain reaches the
    ///    throttle, and is delegated at most once per
    ///    `UNKNOWN_KID_REFRESH_INTERVAL` — that delegation is the one that
    ///    forces `get_key`'s "in case of rotation" refresh — and rejected
    ///    without delegating in between.
    ///
    /// The `kid` decoded here is then *passed* to
    /// `crate::jwks_cache::validate_with_jwks` rather than re-decoded by
    /// it, which is deviation 3 in that module's doc: the original
    /// `validate_jwt_generic` decoded the header a second time, and the
    /// header is now parsed exactly once per request.
    ///
    /// # Why membership of the JWKS, and not "a token that validated"
    ///
    /// The first version of this remembered a `kid` only after a token
    /// bearing it had *fully* validated, which sounds stricter and is
    /// simply wrong: it made the throttle punish the wrong requests.
    /// `backends/tests/integration/tests/merchant_token_flow.rs`'s case (c)
    /// caught it — a token for the wrong audience is presented (correctly
    /// refused, and its `kid` therefore never remembered, but the permit is
    /// spent), and the *next*, entirely valid token signed by the same
    /// published key was refused too. One rejected request would have been
    /// able to deny the next 30 seconds of legitimate ones on the same key.
    ///
    /// Membership of the JWKS is the predicate that actually matches what
    /// the throttle protects: "can delegating this force a fetch". A `kid`
    /// in the cached JWKS cannot, whatever else is wrong with the token, so
    /// it is not throttled and its verdict is decided by the signature and
    /// claims as usual. Nothing an unauthenticated caller sends can add to
    /// the set either — only a `kid` this deployment's own OP published can.
    pub async fn validate(&self, token: &str) -> Result<ResourceClaims, AuthRejection> {
        let kid = decode_header(token)
            .ok()
            .and_then(|header| header.kid)
            .ok_or(AuthRejection::InvalidToken)?;

        if !self.is_published_kid(&kid).await? && !self.claim_unknown_kid_refresh() {
            return Err(AuthRejection::InvalidToken);
        }

        validate_with_jwks::<RawClaims>(token, &kid, &self.cache, &self.validation)
            .await
            .map(ResourceClaims::from)
            .map_err(AuthRejection::from)
    }

    /// Whether `kid` is one the JWKS this validator already holds contains —
    /// i.e. whether delegating a token bearing it can force a refresh.
    ///
    /// Answers from the memoised set first (no allocation, no lock on the
    /// cache) and falls back to the cached JWKS itself. That fallback calls
    /// `crate::jwks_cache::JwksCache::get_jwks`, which performs a network fetch **only** when
    /// the cache is cold or its TTL has elapsed — and, because that refresh
    /// re-checks the entry once it holds the write guard (deviation 5 in
    /// `crate::jwks_cache`'s module doc), exactly one of however many
    /// callers cross the boundary together does the fetching; the rest
    /// serve what it stored. So the bound is one fetch per
    /// `jwks_refresh_interval` per process, whatever the request rate and
    /// whatever the concurrency: the ordinary refresh this module's header
    /// describes, whose *timing* a caller cannot provoke and whose *cost* a
    /// caller cannot multiply.
    ///
    /// The concurrency half of that is worth stating because this line is
    /// reachable before any signature is verified, so an unauthenticated
    /// caller does get to choose when a burst arrives. It cannot choose
    /// what the burst costs: the honest measurement, taken with the
    /// re-check deleted, is that tokio's write-preferring `RwLock` already
    /// held the extra fetches to one or two — not one per request — and the
    /// re-check takes that residue to zero. The numbers, and why the
    /// stampede cannot be raced for decisively in a test, are recorded on
    /// `crate::jwks_cache`'s
    /// `a_caller_that_reaches_the_refresh_with_a_fresh_entry_does_not_fetch_again`.
    ///
    /// A fetch failure here propagates as [`AuthRejection::KeysUnavailable`]
    /// through the same `From` the delegation below uses, so a JWKS outage
    /// answers 503 whether it is discovered on this line or inside
    /// `crate::jwks_cache::validate_with_jwks`.
    async fn is_published_kid(&self, kid: &str) -> Result<bool, AuthRejection> {
        if self.is_remembered_kid(kid) {
            return Ok(true);
        }

        let jwks = self.cache.get_jwks().await?;
        if jwks.find_key(Some(kid)).is_none() {
            return Ok(false);
        }

        self.remember_kid(kid.to_owned());
        Ok(true)
    }

    /// Whether `kid` is in the memoised set — the fast path of
    /// [`Self::is_published_kid`].
    ///
    /// A poisoned lock is recovered from rather than propagated
    /// (`PoisonError::into_inner`): nothing under either guard can panic —
    /// the critical sections are a `HashSet` lookup and an `Instant`
    /// comparison — so poisoning is unreachable, and if it happened anyway,
    /// the honest answer is still "consult the set". The alternative
    /// spellings are an `unwrap` (denied on a request path by ADR-0007) or
    /// treating a poisoned lock as "unknown", which would fail *open* into
    /// exactly the refresh storm this throttle exists to stop.
    fn is_remembered_kid(&self, kid: &str) -> bool {
        self.known_kids
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(kid)
    }

    /// Records `kid` as one this validator has seen in the JWKS, so
    /// subsequent requests bearing it take the fast path.
    fn remember_kid(&self, kid: String) {
        let mut known = self
            .known_kids
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        // Checked first so the common case (an already-known `kid`, i.e.
        // every request after the first for a given key) does not allocate.
        if !known.contains(&kid) {
            known.insert(kid);
        }
    }

    /// Claims this process's one-per-`UNKNOWN_KID_REFRESH_INTERVAL` permit
    /// to let an unrecognized `kid` reach the JWKS cache, stamping the
    /// instant if it is granted.
    ///
    /// Test-and-stamp under one lock, deliberately: a read followed by a
    /// separate write would let a burst of concurrent requests all observe
    /// the same stale instant and all be granted, which is the amplification
    /// this is here to bound. Returns `false` when the permit is not
    /// available, and the caller must then reject without delegating.
    fn claim_unknown_kid_refresh(&self) -> bool {
        let mut last = self
            .last_unknown_kid_refresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        let granted = last.is_none_or(|previous| {
            now.saturating_duration_since(previous) >= UNKNOWN_KID_REFRESH_INTERVAL
        });
        if granted {
            *last = Some(now);
        }
        granted
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
/// ```text
/// async fn create_payment_intent(
///     AuthenticatedMerchant(claims): AuthenticatedMerchant,
///     // ... other extractors ...
/// ) -> impl IntoResponse {
///     // claims.client_id, claims.scope
/// }
/// ```
///
/// `text` rather than a compiled doctest, deliberately: making this one run
/// means standing up a router, a JWKS server and a signed token, which is an
/// integration test (`backends/tests/integration`) and not an illustration of a
/// signature. A ```ignore``` block would have been the dishonest option — it
/// looks compiled and is not.
///
/// # It reads a validated token; it does not validate one
///
/// The claims come from the request's extensions, where
/// [`crate::require_merchant_token`] — the middleware in front of the whole
/// `/v1` nest — put them after validating the token exactly once (Step 2's
/// D3). This used to validate the token itself, which meant a request to a
/// route using this extractor paid for two validations: one in
/// `from_extractor_with_state`, whose extracted value axum 0.8 discards, and
/// one here. Two validations is not merely wasteful — the JWKS cache can
/// refresh between them, so the boundary and the handler could in principle
/// disagree about the same token.
///
/// **Absent extensions fail closed** with [`ApiError::Internal`]: a 500 that
/// pages, never an `Option<ResourceClaims>` a handler could treat as
/// "unauthenticated but carry on". Reaching a handler with no claims means
/// this extractor is mounted on a route the middleware does not cover, which
/// is a routing bug in *this* crate and not something a caller can fix or
/// should be told about. `router`'s own test walks every `/v1` path and
/// asserts 401 without a token, so that bug cannot ship silently.
///
/// No `FromRef` bound any more: the state is not consulted at all. That is
/// deliberate — an extractor that needed the validator could be tempted to
/// use it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedMerchant(pub ResourceClaims);

/// Extractor for `/dash/v1` handlers. See [`AuthenticatedMerchant`]; requires
/// `FromRef<S> for DashboardJwtValidator` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedDashboard(pub ResourceClaims);

impl<S> FromRequestParts<S> for AuthenticatedMerchant
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<ResourceClaims>()
            .cloned()
            .map(AuthenticatedMerchant)
            .ok_or_else(|| {
                ApiError::Internal(
                    "a /v1 handler asked for AuthenticatedMerchant on a request carrying no \
                     validated claims: require_merchant_token is not mounted in front of this \
                     route"
                        .to_owned(),
                )
            })
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
    use std::sync::LazyLock;
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
    /// exactly what the JWKS cache and `validate_with_jwks` key their lookup on.
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
        .expect("the vendored-roots JWKS client builds")
    }

    fn dashboard_validator(jwks_url: String) -> JwtValidator {
        JwtValidator::new(
            jwks_url,
            Duration::from_secs(300),
            ISSUER,
            Surface::Dashboard,
        )
        .expect("the vendored-roots JWKS client builds")
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
        resource_config: std::sync::Arc<crate::ResourceConfig>,
    }

    impl FromRef<TestState> for MerchantJwtValidator {
        fn from_ref(state: &TestState) -> Self {
            state.merchant.clone()
        }
    }

    /// [`crate::require_merchant_token`] resolves the token's `client_id` to
    /// a tenant through this, exactly as the real router does.
    impl FromRef<TestState> for std::sync::Arc<crate::ResourceConfig> {
        fn from_ref(state: &TestState) -> Self {
            std::sync::Arc::clone(&state.resource_config)
        }
    }

    async fn probe(AuthenticatedMerchant(claims): AuthenticatedMerchant) -> Json<Value> {
        Json(json!({ "client_id": claims.client_id, "scope": claims.scope }))
    }

    /// A one-route app behind the **real** `/v1` boundary.
    ///
    /// The middleware is `crate::require_merchant_token` itself, not a
    /// stand-in: since D3 moved validation out of the extractor, an app that
    /// mounted only the extractor would answer 500 to every request here and
    /// would prove nothing about the boundary that ships. Every assertion in
    /// this module is therefore about the same two pieces of code the router
    /// composes — which is why `test_app` grew a layer rather than the tests
    /// growing an expectation.
    fn test_app(validator: JwtValidator) -> Router {
        let state = TestState {
            merchant: MerchantJwtValidator(validator),
            resource_config: std::sync::Arc::new(
                crate::ResourceConfig::from_config(&crate::test_fixtures::config_with(
                    crate::test_fixtures::PUBLIC_BASE_URL,
                    vec![crate::test_fixtures::merchant(
                        TOKEN_SUBJECT,
                        &["payments:write"],
                    )],
                ))
                .expect("the fixture's rails project onto the port"),
            ),
        };
        Router::new()
            // Both methods on one path, because the scope rule is decided
            // by the *method* (`crate::v1::required_scopes`) and a
            // GET-only app could not tell a scope refusal from a routing
            // one.
            .route("/probe", get(probe).post(probe))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::require_merchant_token::<TestState>,
            ))
            .with_state(state)
    }

    /// The `sub` every valid token in this module carries, and therefore the
    /// one client [`test_app`]'s configuration registers. Named so the two
    /// cannot drift: a token for an unregistered client is 403, not 200, and
    /// that is a different test.
    const TOKEN_SUBJECT: &str = "merchant-123";

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
                crate::SCOPE_PAYMENTS_WRITE,
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
        assert_eq!(
            body.get("scope"),
            Some(&json!([crate::SCOPE_PAYMENTS_WRITE]))
        );
    }

    /// The token is genuine and the client is registered; what it is missing
    /// is a *scope*. That is 403, and specifically **not** 401.
    ///
    /// The distinction is the whole point of the test. A 401 tells a client
    /// to go and get a credential — so an SDK that sees one drops its cached
    /// token and re-authenticates, gets a token identical to the one it just
    /// had, and retries: a loop that never terminates and never says what is
    /// actually wrong. `sdks/rust`'s `Client` does exactly that on a 401.
    /// 403 says "this credential, as issued, may not do this", which is both
    /// true and actionable — the fix is in the merchant's registration.
    #[tokio::test]
    async fn a_token_without_the_required_scope_is_403_not_401() {
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;

        // Three tokens for the same registered client, differing only in
        // what they were granted.
        let token_of = |scope: &str| {
            mint_token(
                encoding_key,
                "test-key-1",
                &valid_claims(Surface::Merchant.audience(), TOKEN_SUBJECT, scope),
            )
        };
        let none = token_of("");
        let read = token_of(crate::SCOPE_PAYMENTS_READ);
        let write = token_of(crate::SCOPE_PAYMENTS_WRITE);

        // (method, token, expected status)
        let cases = [
            ("GET", &none, StatusCode::FORBIDDEN),
            ("POST", &none, StatusCode::FORBIDDEN),
            ("GET", &read, StatusCode::OK),
            // A read-only credential must not be able to take a payment.
            ("POST", &read, StatusCode::FORBIDDEN),
            ("GET", &write, StatusCode::OK),
            ("POST", &write, StatusCode::OK),
        ];

        for (method, token, expected) in cases {
            let app = test_app(merchant_validator(format!("{}/jwks.json", server.uri())));
            let response = app
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri("/probe")
                        .header("Authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .expect("valid request"),
                )
                .await
                .expect("router does not fail to serve");

            assert_eq!(
                response.status(),
                expected,
                "{method} /probe with scope {:?}",
                token_scope(token)
            );
            if expected == StatusCode::FORBIDDEN {
                let body = envelope_of(response).await;
                assert_eq!(error_field(&body, "type"), Some("invalid_request_error"));
                assert_eq!(error_field(&body, "code"), Some("forbidden"));
            }
        }
    }

    /// The `scope` claim of a token, for the assertion messages above only —
    /// decoding without verifying is fine for a diagnostic and would not be
    /// anywhere near the validation path.
    fn token_scope(token: &str) -> String {
        let payload = token.split('.').nth(1).unwrap_or_default();
        let bytes = URL_SAFE_NO_PAD.decode(payload).unwrap_or_default();
        serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|claims| {
                claims
                    .get("scope")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "<none>".to_owned())
    }

    // --- the unknown-`kid` throttle (this module's header, and
    //     `UNKNOWN_KID_REFRESH_INTERVAL`) ---

    /// How many JWKS requests the wiremock server actually received.
    ///
    /// This is the measurement the throttle exists to bound: not "did
    /// validation fail" (it always does for a junk token, throttled or not),
    /// but how much work an unauthenticated caller extracted from the
    /// process while failing.
    async fn jwks_fetch_count(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .expect("the wiremock server records requests")
            .len()
    }

    /// The amplification case, measured.
    ///
    /// 100 requests, each with a well-formed token carrying a `kid` the JWKS
    /// has never advertised — the cheapest thing an unauthenticated caller
    /// can send. Without the throttle each one reaches
    /// `JwksCache::get_key`, misses, and forces a `refresh()` — an HTTP GET
    /// that in this deployment is a loopback request to
    /// `/v1/oauth/jwks.json` and therefore a Postgres `SELECT`, taken while
    /// holding the cache's write lock. With it, the first junk token spends
    /// the one permit and the other 99 are refused with no cache access at
    /// all.
    ///
    /// Two fetches, not one, is the honest bound and is what
    /// `JwksCache::get_key` does with an empty cache: `get_jwks()` fetches
    /// (the cache starts cold), `find_key` misses, and `refresh()` fetches
    /// once more "in case of rotation". Both belong to the *single* permitted
    /// delegation.
    ///
    /// Decisive by construction: with the throttle removed this asserts 2
    /// against ~101.
    #[tokio::test]
    async fn a_hundred_unknown_kids_force_at_most_two_jwks_fetches() {
        const JUNK_REQUESTS: usize = 100;

        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let validator = merchant_validator(format!("{}/jwks.json", server.uri()));

        for n in 0..JUNK_REQUESTS {
            // Signed by a real key and otherwise perfectly well-formed: the
            // only thing wrong with these tokens is a `kid` nobody has
            // published, which is precisely the input that used to be free
            // to amplify.
            let token = mint_token(
                encoding_key,
                &format!("junk-kid-{n}"),
                &valid_claims(Surface::Merchant.audience(), "merchant-123", ""),
            );
            let error = validator
                .validate(&token)
                .await
                .expect_err("a token with an unpublished kid must never validate");
            assert!(
                matches!(error, AuthRejection::InvalidToken),
                "an unknown kid is the caller's problem, not a 503: {error:?}"
            );
        }

        let fetches = jwks_fetch_count(&server).await;
        assert!(
            fetches <= 2,
            "{JUNK_REQUESTS} junk tokens must not be able to force more than the one permitted \
             delegation's fetches, got {fetches}"
        );
    }

    /// The other half of the trade-off: the throttle must not lock out a key
    /// the process is actually serving.
    ///
    /// Order matters and is the point. A valid token is presented *first*,
    /// which is what puts its `kid` in `known_kids`; then the junk burst
    /// spends the unknown-`kid` permit; then the same key's token is
    /// presented again and must still be accepted, because a known `kid`
    /// never consults the throttle at all.
    ///
    /// This also pins the documented cost. A key that has **never** signed
    /// an accepted token here — a brand-new key mid-rotation — is on the
    /// unknown path and *is* refused while the permit is spent; that is
    /// asserted below rather than left implied, so nobody discovers it in
    /// production. See `UNKNOWN_KID_REFRESH_INTERVAL` for why that is the
    /// accepted trade.
    #[tokio::test]
    async fn a_valid_token_still_validates_after_the_unknown_kid_throttle_has_tripped() {
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let validator = merchant_validator(format!("{}/jwks.json", server.uri()));

        let good_token = mint_token(
            encoding_key,
            "test-key-1",
            &valid_claims(Surface::Merchant.audience(), "merchant-123", ""),
        );
        validator
            .validate(&good_token)
            .await
            .expect("the first valid token is accepted and its kid remembered");

        for n in 0..50 {
            let junk = mint_token(
                encoding_key,
                &format!("junk-kid-{n}"),
                &valid_claims(Surface::Merchant.audience(), "merchant-123", ""),
            );
            let _ = validator.validate(&junk).await;
        }

        let claims = validator
            .validate(&good_token)
            .await
            .expect("a known kid must keep validating through a junk burst");
        assert_eq!(claims.client_id, "merchant-123");

        // The documented cost, asserted: a *different* key, published or
        // not, is on the throttled path while the permit is spent.
        let unseen = mint_token(
            encoding_key,
            "a-kid-this-process-has-never-accepted",
            &valid_claims(Surface::Merchant.audience(), "merchant-123", ""),
        );
        assert!(
            matches!(
                validator.validate(&unseen).await,
                Err(AuthRejection::InvalidToken)
            ),
            "an unknown kid inside the throttle window is refused — the trade-off on \
             UNKNOWN_KID_REFRESH_INTERVAL"
        );

        // One fetch for the first valid token (cold cache, key found), one
        // more for the single junk token that got the permit.
        let fetches = jwks_fetch_count(&server).await;
        assert!(
            fetches <= 2,
            "a burst of junk between two valid requests must not multiply fetches, got {fetches}"
        );
    }

    /// A *refused* token must not cost the next legitimate one anything,
    /// when both are signed by a key the JWKS publishes.
    ///
    /// The regression this pins is real and was caught by
    /// `backends/tests/integration/tests/merchant_token_flow.rs`'s case (c)
    /// rather than by anything in this module: with the throttle keyed on
    /// "a token bearing this `kid` has fully validated", the wrong-audience
    /// token below (rightly refused, so its `kid` was never remembered) spent
    /// the one permit, and the perfectly valid token that followed it — same
    /// published key, right audience — was refused as though its `kid` were
    /// junk. One rejected request could deny the next 30 seconds of good
    /// ones on the same key.
    ///
    /// Both tokens carry `test-key-1`, which the JWKS advertises, so under
    /// the correct predicate neither consults the throttle at all. Ordering
    /// is the whole test: the refusal comes first.
    #[tokio::test]
    async fn a_refused_token_does_not_spend_the_permit_for_a_good_one_on_the_same_key() {
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let validator = merchant_validator(format!("{}/jwks.json", server.uri()));

        // Signed by the published key, but addressed to the other surface —
        // exactly case (c)'s first request.
        let wrong_audience = mint_token(
            encoding_key,
            "test-key-1",
            &valid_claims(Surface::Dashboard.audience(), "merchant-123", ""),
        );
        assert!(
            matches!(
                validator.validate(&wrong_audience).await,
                Err(AuthRejection::InvalidToken)
            ),
            "a token for the wrong surface must be refused"
        );

        let good = mint_token(
            encoding_key,
            "test-key-1",
            &valid_claims(Surface::Merchant.audience(), "merchant-123", ""),
        );
        let claims = validator.validate(&good).await.expect(
            "a valid token signed by a published key must not be refused because an earlier \
             token was",
        );
        assert_eq!(claims.client_id, "merchant-123");
    }

    /// A header with no `kid` is refused without touching the JWKS at all —
    /// the cheapest rejection there is, and one that cannot be turned into a
    /// fetch.
    ///
    /// This is the check that makes `crate::jwks_cache`'s deviation 2 —
    /// `get_key` taking `&str` rather than authkestra's
    /// `Option<&str>` + `require_kid(true)` — an equivalent rather than a
    /// weakening: a token with no `kid` never reaches the cache, so the
    /// missing-`kid` arm has nothing to decide. The assertion is on the cost
    /// as well as the verdict: zero requests reached the JWKS server.
    #[tokio::test]
    async fn a_token_with_no_kid_header_is_rejected_without_any_jwks_fetch() {
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let validator = merchant_validator(format!("{}/jwks.json", server.uri()));

        // `Header::new` leaves `kid` at `None`; every other test in this
        // module goes through `mint_token`, which sets it.
        let token = jsonwebtoken::encode(
            &Header::new(Algorithm::RS256),
            &valid_claims(Surface::Merchant.audience(), "merchant-123", ""),
            encoding_key,
        )
        .expect("signing succeeds");

        let error = validator
            .validate(&token)
            .await
            .expect_err("a token with no kid must be rejected");
        assert!(matches!(error, AuthRejection::InvalidToken));
        assert_eq!(
            jwks_fetch_count(&server).await,
            0,
            "a token with no kid must not be able to provoke a JWKS fetch"
        );

        // Same for a value that is not a JWT at all: rejected before any
        // cache access, not after one.
        let error = validator
            .validate("not-a-jwt")
            .await
            .expect_err("a non-JWT bearer value must be rejected");
        assert!(matches!(error, AuthRejection::InvalidToken));
        assert_eq!(jwks_fetch_count(&server).await, 0);
    }

    // --- a JWKS outage is 503, not 401 ---

    /// A URL nothing is listening on, so a fetch fails with a refused
    /// connection rather than a status code.
    ///
    /// The port is bound and immediately released, which is the only way to
    /// name a port that was free a moment ago; the alternative (a hard-coded
    /// number) is the flakier of the two on a shared machine.
    fn unreachable_jwks_url() -> String {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("binding an ephemeral port");
        let addr = listener.local_addr().expect("the bound address");
        drop(listener);
        format!("http://{addr}/jwks.json")
    }

    /// The finding: a JWKS this process cannot fetch is *our* outage, and
    /// answering 401 makes every SDK re-authenticate against the token
    /// endpoint — which is database-backed, and therefore the last thing
    /// that should absorb a retry storm during an outage.
    #[tokio::test]
    async fn a_jwks_that_cannot_be_fetched_is_keys_unavailable_not_invalid_token() {
        let (encoding_key, _jwk) = &*KEYPAIR;
        let validator = merchant_validator(unreachable_jwks_url());

        let token = mint_token(
            encoding_key,
            "test-key-1",
            &valid_claims(Surface::Merchant.audience(), "merchant-123", ""),
        );

        let error = validator
            .validate(&token)
            .await
            .expect_err("a token cannot be validated when the JWKS is unreachable");
        assert!(
            matches!(error, AuthRejection::KeysUnavailable),
            "a fetch failure must not be reported as a bad credential: {error:?}"
        );
        assert_eq!(error.category(), Category::Storage);
        assert_eq!(error.retry(), vpay_core::Retry::AfterBackoff);
    }

    /// The same failure as an SDK sees it: 503 and the `api_error` envelope
    /// the policy table gives [`Category::Storage`], not a 401 that reads as
    /// "your token is stale".
    #[tokio::test]
    async fn a_jwks_outage_is_a_503_envelope_over_the_router() {
        let (encoding_key, _jwk) = &*KEYPAIR;
        let app = test_app(merchant_validator(unreachable_jwks_url()));

        let token = mint_token(
            encoding_key,
            "test-key-1",
            &valid_claims(Surface::Merchant.audience(), "merchant-123", ""),
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

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = envelope_of(response).await;
        assert_eq!(error_field(&body, "type"), Some("api_error"));
        assert_eq!(error_field(&body, "code"), Some("service_unavailable"));
        assert_eq!(
            error_field(&body, "message"),
            Some("vpay is temporarily unavailable. Retry after a short delay."),
            "the message must not tell a merchant to re-authenticate"
        );
    }

    /// The control for the two tests above: a credential that really is bad
    /// must still be 401, or `KeysUnavailable` would have swallowed the
    /// authentication boundary rather than narrowed it.
    #[tokio::test]
    async fn a_bad_signature_is_still_a_401_over_the_router() {
        let (_good_key, good_jwk) = &*KEYPAIR;
        let (forged_key, _forged_jwk) = generate_keypair("test-key-1");
        let server = jwks_server(good_jwk).await;
        let app = test_app(merchant_validator(format!("{}/jwks.json", server.uri())));

        let token = mint_token(
            &forged_key,
            "test-key-1",
            &valid_claims(Surface::Merchant.audience(), "merchant-123", ""),
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

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = envelope_of(response).await;
        assert_eq!(error_field(&body, "type"), Some("authentication_error"));
        assert_eq!(error_field(&body, "code"), Some("invalid_token"));
    }

    // --- audience confusion: the token the OP mints when none is requested ---

    /// The shape `authkestra_op` actually produces when a merchant's token
    /// request carries no `audience` parameter: `aud` defaults to the
    /// client's own `client_id`
    /// (`authkestra-op-0.7.1/src/handlers/token.rs`, "No audience requested;
    /// defaulting client_credentials token audience to client_id"). Such a
    /// token is signed by this deployment's own key and is valid in every
    /// other respect — it simply is not addressed to `/v1`.
    ///
    /// Pinned here as well as in
    /// `backends/tests/integration/tests/merchant_token_flow.rs` (which
    /// proves the OP really mints it that way, end to end) so the property
    /// is checked without Docker.
    ///
    /// # The decisive mutation is *widening* the audience, not deleting it
    ///
    /// Deleting `set_audience` from [`JwtValidator::new`] does not make this
    /// test fail — it makes almost every other test in this module fail
    /// instead, because `jsonwebtoken` 11 fails *closed* on that spelling:
    /// `validation.rs`'s `(TryParse::Parsed(_), None) => return
    /// Err(InvalidAudience)` arm rejects any token that carries an `aud`
    /// claim when no expected audience was configured. (That is the opposite
    /// of the absent-`aud` hole this module's header documents, which is a
    /// different arm — `_ => {}` — and still real.) The mutation this test
    /// is decisive against is the one that matches the finding: widening the
    /// accepted set, e.g. `set_audience(&[surface.audience(),
    /// "some-client-id"])`, under which this is the only test in the module
    /// that fails.
    #[tokio::test]
    async fn a_token_whose_audience_is_the_client_id_is_refused_on_the_merchant_surface() {
        let (encoding_key, jwk) = &*KEYPAIR;
        let server = jwks_server(jwk).await;
        let validator = merchant_validator(format!("{}/jwks.json", server.uri()));

        let token = mint_token(
            encoding_key,
            "test-key-1",
            // `aud` == `sub` == the client id, which is exactly what a token
            // request with no `audience` field comes back with.
            &valid_claims("some-client-id", "some-client-id", "payments:write"),
        );

        let error = validator
            .validate(&token)
            .await
            .expect_err("a token addressed to the client itself must not be accepted on /v1");
        assert!(matches!(error, AuthRejection::InvalidToken));
    }
}
