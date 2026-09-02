//! A time-bounded JWKS cache that takes its HTTP client as an argument.
//!
//! # This is a deliberate port, and why it had to be one
//!
//! Source: `authkestra_resource::jwt::JwksCache` and the free function
//! `validate_jwt_generic`, at the workspace's `=0.7.1` pin —
//! `authkestra-resource-0.7.1/src/jwt.rs`, lines 110–201 and 1299–1319. The
//! caching policy below is theirs; the deviations — including the one place
//! it now behaves differently from upstream at runtime (5) — are listed at
//! the bottom of this comment and nowhere else.
//!
//! The reason for the port is a single line of the original:
//!
//! ```text
//! pub fn new(jwks_uri: String, refresh_interval: Duration) -> Self {
//!     Self { …, client: reqwest::Client::new() }   // jwt.rs:128
//! }
//! ```
//!
//! `reqwest::Client::new()` panics when the builder fails, and under this
//! workspace's reqwest 0.13 pin the builder reads the **platform** trust
//! store eagerly and fails when it is empty. The runtime image is
//! `FROM scratch` ([ADR-0004]) and has none, so `vpay-server` panicked at
//! boot inside its own image — see [`crate::http_client`] for the full
//! account of that failure.
//!
//! `JwksCache::with_client` (authkestra#301) looks like the seam for this and
//! is not: it is a consuming builder that *replaces* the client `new` has
//! already constructed, so `JwksCache::new(..).with_client(..)` still runs
//! the panicking line first. That was verified against the real binary, not
//! inferred — with [`crate::http_client::client`] wired in through
//! `with_client`, `vpay-server` still died with
//! `Client::new(): reqwest::Error { kind: Builder, source: General("No CA
//! certificates were loaded from the system") }`. At 0.7.1 there is no other
//! constructor and the struct is `#[non_exhaustive]`, so no amount of calling
//! avoids that line. The upstream fix — making `new` lazy, or adding a
//! `JwksCache::with_client(uri, ttl, client)` constructor — is not something
//! this repository can apply to a pinned published crate.
//!
//! Everything cryptographic stays where it was. The keys are still
//! authkestra's [`Jwks`]/[`Jwk`], still fetched by authkestra's
//! [`Jwks::fetch_with`], still converted by authkestra's
//! `Jwk::to_decoding_key`, and the signature is still checked by
//! `jsonwebtoken::decode`. What is ported is the cache's *policy* — when to
//! re-fetch — which is a dozen lines and is pinned by
//! `resource_auth`'s existing tests (the fetch-count, TTL, rotation and
//! throttle cases all assert on this behaviour through a real wiremock JWKS).
//!
//! # Deviations from the original, all deliberate
//!
//! 1. **The client is a constructor argument** ([`JwksCache::new`]), not
//!    something built internally. This is the entire point of the port.
//! 2. **`kid` is required by type, not by flag.** The original takes
//!    `Option<&str>` and falls back to "the first key in the JWKS" unless
//!    `require_kid(true)` was set; this takes `&str`. The behaviour is the
//!    one vpay already selected — `resource_auth` passed `require_kid(true)`
//!    and rejects a token with no `kid` before it reaches the cache at all —
//!    but expressing it in the signature means it cannot be un-set by a
//!    future edit, and removes the `MissingKid` arm as unreachable rather
//!    than merely unused.
//! 3. **The header is decoded once, not twice.** The original
//!    `validate_jwt_generic` re-decodes the JWT header to recover the `kid`
//!    that its caller had already decoded; [`validate_with_jwks`] takes the
//!    `kid` as an argument. Same verdict, one less base64+JSON parse per
//!    request. `resource_auth`'s [`crate::resource_auth::JwtValidator::validate`]
//!    documents why it must decode the header before delegating.
//! 4. **No `IssuerTrustMap`/`SingleJwksResolver`/DPoP support.** This
//!    deployment has exactly one issuer (its own OP over loopback), and the
//!    resolver machinery is unreachable from `resource_auth`. Porting unused
//!    generality would be code no test could reach.
//! 5. **[`JwksCache::get_jwks`] re-checks the cache under the write
//!    guard** ([`JwksCache::refresh_if_stale`]). The original drops its
//!    read guard and calls `refresh()`, which takes the write guard and
//!    fetches unconditionally (`jwt.rs:169-177`), so a caller that queued
//!    behind another one re-fetches a JWKS that was just stored. Here it
//!    serves what the earlier waiter stored instead. This is a fix, not a
//!    bug-for-bug carry-over, and it is the one place this port knowingly
//!    behaves differently from upstream at runtime.
//!
//!    Worth stating exactly, because the obvious description of this
//!    ("N concurrent requests at a TTL boundary cost N fetches") is
//!    **wrong** and was measured to be wrong before this was written:
//!    `tokio::sync::RwLock` is write-preferring, so once the first caller
//!    queues on `write()` every later reader blocks at `read()` and then
//!    sees the fresh entry. With the re-check deleted, 32 callers released
//!    together from a `Barrier` on a 32-worker runtime, over 20 TTL
//!    boundaries, cost **one** extra fetch on 17 rounds and two on 3 —
//!    never 32. The re-check removes that residual one, and turns "usually
//!    1, sometimes 2" into "1". Pinned by
//!    [`JwksCache::refresh_if_stale`]'s own test,
//!    `a_caller_that_reaches_the_refresh_with_a_fresh_entry_does_not_fetch_again`,
//!    which tests the rule rather than racing for it — see that test for
//!    why racing for it cannot be made decisive.
//!
//! Not deviations, and worth stating because they look like ones:
//!
//! - **The write lock is still held across the HTTP GET**, exactly as
//!   upstream holds it. Deviation 5 bounds how many GETs a TTL boundary
//!   costs, not how long the lock is held for the one that is made: a JWKS
//!   fetch still blocks every concurrent validation in the process for its
//!   duration. Fetching outside the lock would change much more of the
//!   original's policy (two callers could then store out-of-order results)
//!   than the re-check does, and the fetch is a loopback request here.
//! - **[`JwksCache::get_key`]'s miss path still refreshes
//!   unconditionally**, "in case of rotation". The same re-check there
//!   would suppress precisely the fetch that exists: the `get_jwks` call
//!   immediately above it has just stored a fresh entry, so any "fetched
//!   within the last N ms" test is true by construction and the cache would
//!   stop noticing a key published since its last refresh. That path is
//!   remotely triggerable by anyone who can put a `kid` in a header, and
//!   `resource_auth`'s `UNKNOWN_KID_REFRESH_INTERVAL` — one permitted
//!   delegation per interval per process, test-and-stamp under a single
//!   lock, so a concurrent burst spends one permit between them — remains
//!   its mitigation, bounding it to the two fetches
//!   `a_hundred_unknown_kids_force_at_most_two_jwks_fetches` measures.
//!
//! [ADR-0004]: ../../../docs/adr/0004-musl-mimalloc.md

use std::time::{Duration, Instant};

use authkestra_resource::jwt::{Jwk, Jwks, ValidationError};
use jsonwebtoken::{Validation, decode};
use serde::de::DeserializeOwned;
use tokio::sync::{RwLock, RwLockWriteGuard};

/// The JWKS this process validates bearer tokens against, plus when it was
/// last fetched.
///
/// Ported from `authkestra_resource::jwt::JwksCache`; see the module doc for
/// why, and for the list of deviations.
pub(crate) struct JwksCache {
    jwks_uri: String,
    /// `tokio::sync::RwLock`, not `std::sync::RwLock`: the write guard is
    /// held across an `.await` in [`Self::fetch_into`]. Same choice the
    /// original makes, for the same reason.
    jwks: RwLock<Option<(Jwks, Instant)>>,
    ttl: Duration,
    /// Held for the cache's lifetime rather than rebuilt per refresh, so the
    /// connection pool survives across refreshes — and, unlike the original,
    /// supplied by the caller so it can be one that does not read the OS
    /// trust store.
    client: reqwest::Client,
}

impl JwksCache {
    /// `jwks_uri` is re-fetched at most once per `refresh_interval` by
    /// [`Self::get_jwks`], plus once more per [`Self::get_key`] miss.
    ///
    /// `client` is not defaulted, deliberately: a default would have to be
    /// `reqwest::Client::new()`, which is the panic this whole module exists
    /// to route around. [`crate::http_client::client`] is the one every
    /// caller in this crate should pass.
    pub(crate) fn new(
        jwks_uri: String,
        refresh_interval: Duration,
        client: reqwest::Client,
    ) -> Self {
        Self {
            jwks_uri,
            jwks: RwLock::new(None),
            ttl: refresh_interval,
            client,
        }
    }

    /// The cached JWKS, fetching only when the cache is cold or its TTL has
    /// elapsed — never once per call, and never more than once per TTL
    /// however many callers cross the boundary together. This is what makes
    /// token validation a local operation rather than a network round trip
    /// per request.
    ///
    /// # Errors
    ///
    /// [`ValidationError::Http`] if a fetch was needed and failed;
    /// `resource_auth` maps that to a 503, never a 401, because it is this
    /// process's outage rather than a bad credential.
    pub(crate) async fn get_jwks(&self) -> Result<Jwks, ValidationError> {
        {
            let read_guard = self.jwks.read().await;
            // Collapsed into one `if let` chain; the original spells it as
            // two nested `if`s (jwt.rs:169-175). Same condition.
            if let Some((jwks, last_updated)) = read_guard.as_ref()
                && last_updated.elapsed() < self.ttl
            {
                return Ok(jwks.clone());
            }
        }

        self.refresh_if_stale().await
    }

    /// The key `kid` names, or `None` if the JWKS genuinely does not hold it.
    ///
    /// A miss costs a second fetch, unconditionally — "in case of rotation",
    /// as upstream puts it: the key may have been published between this
    /// cache's last refresh and now. That second fetch is remotely
    /// triggerable by anyone who can send a `kid`, which is why
    /// `resource_auth` throttles the calls that reach here with an
    /// unrecognised one instead of relying on the TTL.
    ///
    /// # Errors
    ///
    /// [`ValidationError::Http`], as [`Self::get_jwks`].
    pub(crate) async fn get_key(&self, kid: &str) -> Result<Option<Jwk>, ValidationError> {
        let jwks = self.get_jwks().await?;
        if let Some(key) = jwks.find_key(Some(kid)) {
            return Ok(Some(key.clone()));
        }

        let jwks = self.refresh().await?;
        Ok(jwks.find_key(Some(kid)).cloned())
    }

    /// Fetches the JWKS and replaces whatever was cached, unconditionally.
    ///
    /// Kept unconditional for [`Self::get_key`]'s rotation path — see the
    /// second "not a deviation" in the module doc for why a re-check there
    /// would defeat the only fetch that can pick up a newly published key.
    /// The TTL path uses [`Self::refresh_if_stale`] instead.
    ///
    /// # Errors
    ///
    /// [`ValidationError::Http`]. `Jwks::fetch_with` collapses a refused
    /// connection, a timeout, a 5xx and an unparseable body into that one
    /// variant — see `resource_auth`'s `From<ValidationError>` table.
    async fn refresh(&self) -> Result<Jwks, ValidationError> {
        let mut write_guard = self.jwks.write().await;
        self.fetch_into(&mut write_guard).await
    }

    /// The TTL refresh, taken by [`Self::get_jwks`] alone: fetches only if
    /// the entry is *still* stale once the write guard is held.
    ///
    /// The re-check is the whole point (deviation 5). Callers that made the
    /// same stale observation before the first of them registered as a
    /// writer are queued here behind it; without the re-check each of them
    /// re-fetches a JWKS that is already fresh, and since the GET is a
    /// loopback `/v1/oauth/jwks.json` that is a redundant Postgres `SELECT`
    /// taken while holding the lock every other validation in the process
    /// needs. How many such callers there can be is bounded by tokio's
    /// write-preferring lock and measured in deviation 5 — one, usually.
    ///
    /// # Errors
    ///
    /// [`ValidationError::Http`], as [`Self::refresh`] — and only when this
    /// caller was the one that had to fetch.
    async fn refresh_if_stale(&self) -> Result<Jwks, ValidationError> {
        let mut write_guard = self.jwks.write().await;
        if let Some((jwks, last_updated)) = write_guard.as_ref()
            && last_updated.elapsed() < self.ttl
        {
            return Ok(jwks.clone());
        }

        self.fetch_into(&mut write_guard).await
    }

    /// Performs the GET and stamps the result, given a guard the caller
    /// already holds.
    ///
    /// Takes the guard rather than acquiring one so that
    /// [`Self::refresh_if_stale`]'s re-check and this store are a single
    /// critical section: releasing the lock in between would reopen exactly
    /// the window the re-check closes.
    ///
    /// # Errors
    ///
    /// [`ValidationError::Http`]. A failed fetch leaves the previously
    /// cached entry in place rather than clearing it — upstream's behaviour,
    /// kept: the error is already what makes `resource_auth` answer 503, so
    /// emptying the cache too would buy nothing but a cold start once the
    /// JWKS endpoint returns.
    async fn fetch_into(
        &self,
        write_guard: &mut RwLockWriteGuard<'_, Option<(Jwks, Instant)>>,
    ) -> Result<Jwks, ValidationError> {
        let jwks = Jwks::fetch_with(&self.client, &self.jwks_uri).await?;
        **write_guard = Some((jwks.clone(), Instant::now()));
        Ok(jwks)
    }
}

/// Resolves `kid` against `cache` and verifies `token` with the key it names.
///
/// The port of `authkestra_resource::jwt::validate_jwt_generic`
/// (`jwt.rs:1299`), differing only in taking `kid` from the caller rather
/// than re-decoding the header — see deviation 3 in the module doc.
///
/// # Errors
///
/// [`ValidationError::KeyNotFound`] if the JWKS does not publish `kid`,
/// [`ValidationError::Discovery`] if the JWK cannot be turned into a
/// verifying key, [`ValidationError::Jwt`] if the signature or any claim
/// `validation` requires does not hold, and [`ValidationError::Http`] from
/// the fetch underneath. `resource_auth`'s `From` impl decides which of those
/// is a 401 and which a 503; nothing here makes that decision.
pub(crate) async fn validate_with_jwks<T>(
    token: &str,
    kid: &str,
    cache: &JwksCache,
    validation: &Validation,
) -> Result<T, ValidationError>
where
    T: DeserializeOwned,
{
    let jwk = cache
        .get_key(kid)
        .await?
        .ok_or(ValidationError::KeyNotFound)?;

    let decoding_key = jwk.to_decoding_key()?;
    let token_data = decode::<T>(token, &decoding_key, validation)?;

    Ok(token_data.claims)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// Serves an empty JWKS from a real local HTTP server.
    ///
    /// The *body* is deliberately irrelevant here: these tests measure how
    /// many times the cache goes to the network, and nothing on that path
    /// inspects a key. Real key material, real signatures and real `kid`
    /// resolution are exercised by `crate::resource_auth`'s tests, which
    /// drive this same cache through a wiremock JWKS holding a generated
    /// RSA key — so serving `{"keys": []}` here narrows the test to the one
    /// thing it can decide, rather than restating those.
    async fn jwks_server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [] })))
            .mount(&server)
            .await;
        server
    }

    /// How many JWKS requests the wiremock server actually received — the
    /// only measurement these tests make.
    async fn fetch_count(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .expect("the wiremock server records requests")
            .len()
    }

    fn cache(server: &MockServer, ttl: Duration) -> JwksCache {
        JwksCache::new(
            format!("{}/jwks.json", server.uri()),
            ttl,
            crate::http_client::client().expect("the vendored-roots client builds"),
        )
    }

    /// The rule deviation 5 adds, tested where it is decidable: a caller
    /// that reaches the TTL refresh and finds the entry *already fresh*
    /// must not fetch again.
    ///
    /// That is precisely the state of a caller which observed the entry
    /// stale, queued on `write()` behind another caller doing the same, and
    /// woke up after that one stored a fresh JWKS. Calling
    /// [`JwksCache::refresh_if_stale`] directly *is* that wake-up — the
    /// race only decides who gets here in that state, never what they then
    /// do — so this pins the behaviour without depending on a scheduler.
    ///
    /// # Why the stampede is not reproduced end-to-end instead
    ///
    /// It cannot be made to fail reliably, and a test that fails one run in
    /// five is worse than none. `tokio::sync::RwLock` is write-preferring
    /// (documented, and its FIFO queue is why), so the moment the first
    /// caller queues on `write()` every later reader blocks at `read()` and
    /// goes on to observe the fresh entry that caller stores. The window in
    /// which a second caller can slip its read check in is the few
    /// nanoseconds between the first one dropping its read guard and
    /// registering as a writer. Measured, with the re-check deleted: 32
    /// callers released together from a `Barrier` on a 32-worker runtime,
    /// against a JWKS server delayed 20 ms, over 20 TTL boundaries —
    /// **1 extra fetch on 17 rounds and 2 on 3 rounds, never 32**. The same
    /// burst through `resource_auth::JwtValidator::validate` gave 1 on four
    /// runs of five.
    ///
    /// So the amplification the re-check removes is one redundant fetch,
    /// not one per in-flight request — the write-preferring lock was
    /// already doing most of this. It is still worth removing: that fetch
    /// is a `SELECT` taken while holding the lock every other validation in
    /// the process is queued on, and "usually 1, sometimes 2" is a worse
    /// property to reason about than "1".
    ///
    /// Decisive: deleting the re-check makes this report 2 fetches.
    #[tokio::test]
    async fn a_caller_that_reaches_the_refresh_with_a_fresh_entry_does_not_fetch_again() {
        let server = jwks_server().await;
        let cache = cache(&server, Duration::from_secs(300));

        // The first caller through the boundary: fetches and stores.
        cache.get_jwks().await.expect("the cold fetch succeeds");
        assert_eq!(fetch_count(&server).await, 1, "the cold fetch happened");

        // A caller that had already decided the entry was stale, resuming
        // with the write guard it queued for.
        cache
            .refresh_if_stale()
            .await
            .expect("serving the entry a previous waiter stored cannot fail");

        assert_eq!(
            fetch_count(&server).await,
            1,
            "a waiter that wakes to a JWKS fetched inside the TTL must serve it, not re-fetch it"
        );
    }

    /// The complement, and the reason the re-check is a condition rather
    /// than an early return: an entry *older* than the TTL must still be
    /// re-fetched. An inverted or over-eager re-check would wedge the cache
    /// on its first JWKS forever, and every test above would still pass.
    #[tokio::test]
    async fn an_entry_older_than_the_ttl_is_still_refetched() {
        const TTL: Duration = Duration::from_millis(50);

        let server = jwks_server().await;
        let cache = cache(&server, TTL);

        cache.get_jwks().await.expect("the cold fetch succeeds");
        tokio::time::sleep(TTL * 2).await;
        cache.get_jwks().await.expect("the refresh succeeds");

        assert_eq!(
            fetch_count(&server).await,
            2,
            "an expired entry is re-fetched; key rotation depends on it"
        );
    }

    /// The uncontended fast path, which is what makes validation a local
    /// operation rather than a round trip per request: inside one TTL,
    /// [`JwksCache::get_jwks`] answers from the read guard and never
    /// reaches the refresh at all.
    ///
    /// Pinned separately from the two refresh tests above because it is a
    /// different line of code — the read-guard check in `get_jwks` — and a
    /// change that made every call take the write lock would still pass
    /// them while serialising every validation in the process.
    #[tokio::test]
    async fn a_fresh_entry_is_served_without_going_to_the_network() {
        let server = jwks_server().await;
        let cache = cache(&server, Duration::from_secs(300));

        for _ in 0..8 {
            cache.get_jwks().await.expect("the JWKS server is up");
        }

        assert_eq!(
            fetch_count(&server).await,
            1,
            "eight calls inside one TTL are one fetch"
        );
    }

    /// `get_key`'s "in case of rotation" refresh is *not* coalesced, and
    /// that is deliberate — see the second "not a deviation" in this
    /// module's doc. A test says so, because it is the kind of asymmetry a
    /// later reader would "tidy up" into a bug.
    ///
    /// Two fetches for one `get_key` miss on a cold cache: `get_jwks`
    /// fetches, `find_key` misses, and `refresh` fetches again in case the
    /// key was published since. `resource_auth`'s
    /// `UNKNOWN_KID_REFRESH_INTERVAL` is what bounds how often an
    /// unauthenticated caller can reach this, not the TTL.
    #[tokio::test]
    async fn a_get_key_miss_still_refreshes_unconditionally() {
        let server = jwks_server().await;
        let cache = cache(&server, Duration::from_secs(300));

        let found = cache
            .get_key("never-published")
            .await
            .expect("the JWKS server is up");

        assert!(found.is_none(), "the served JWKS holds no keys at all");
        assert_eq!(
            fetch_count(&server).await,
            2,
            "a miss costs the TTL fetch plus the rotation fetch"
        );
    }
}
