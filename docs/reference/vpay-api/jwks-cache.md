# `vpay-api` — the JWKS cache (`jwks_cache.rs`)

_Moved out of [docs/reference/vpay-api.md](../vpay-api.md) on 2026-09-11 by exp57, which split a 1 566-line reference into a page per surface. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a cross-reference pointed at a heading that is now on another page, the page it moved to._

## The JWKS cache (`jwks_cache.rs`)

A time-bounded JWKS cache that takes its HTTP client as an argument.

### This is a deliberate port, and why it had to be one

Source: `authkestra_resource::jwt::JwksCache` and the free function
`validate_jwt_generic`, at the workspace's `=0.7.1` pin —
`authkestra-resource-0.7.1/src/jwt.rs`, lines 110–201 and 1299–1319. The caching
policy is theirs; the deviations — including the one place it now behaves
differently from upstream at runtime (5) — are listed below and nowhere else.

The reason for the port is a single line of the original:

```text
pub fn new(jwks_uri: String, refresh_interval: Duration) -> Self {
    Self { …, client: reqwest::Client::new() }   // jwt.rs:128
}
```

`reqwest::Client::new()` panics when the builder fails, and under this
workspace's reqwest 0.13 pin the builder reads the **platform** trust store
eagerly and fails when it is empty. The runtime image is `FROM scratch`
([ADR-0004](../../adr/0004-musl-mimalloc.md)) and has none, so `vpay-server`
panicked at boot inside its own image — see `vpay_provider::http` for the full
account of that failure.

`JwksCache::with_client` (authkestra#301) looks like the seam for this and is
not: it is a consuming builder that _replaces_ the client `new` has already
constructed, so `JwksCache::new(..).with_client(..)` still runs the panicking
line first. That was verified against the real binary, not inferred — with
`vpay_provider::http::client` wired in through `with_client`, `vpay-server`
still died with `Client::new(): reqwest::Error { kind: Builder, source:
General("No CA certificates were loaded from the system") }`. At 0.7.1 there is
no other constructor and the struct is `#[non_exhaustive]`, so no amount of
calling avoids that line. The upstream fix — making `new` lazy, or adding a
`JwksCache::with_client(uri, ttl, client)` constructor — is not something this
repository can apply to a pinned published crate.

Everything cryptographic stays where it was. The keys are still authkestra's
`Jwks`/`Jwk`, still fetched by authkestra's `Jwks::fetch_with`, still converted
by authkestra's `Jwk::to_decoding_key`, and the signature is still checked by
`jsonwebtoken::decode`. What is ported is the cache's _policy_ — when to
re-fetch — which is a dozen lines and is pinned by `resource_auth`'s existing
tests (the fetch-count, TTL, rotation and throttle cases all assert on this
behaviour through a real wiremock JWKS).

### Deviations from the original, all deliberate

1. **The client is a constructor argument** (`JwksCache::new`), not something
   built internally. This is the entire point of the port.
2. **`kid` is required by type, not by flag.** The original takes
   `Option<&str>` and falls back to "the first key in the JWKS" unless
   `require_kid(true)` was set; this takes `&str`. The behaviour is the one vpay
   already selected — `resource_auth` passed `require_kid(true)` and rejects a
   token with no `kid` before it reaches the cache at all — but expressing it in
   the signature means it cannot be un-set by a future edit, and removes the
   `MissingKid` arm as unreachable rather than merely unused.
3. **The header is decoded once, not twice.** The original
   `validate_jwt_generic` re-decodes the JWT header to recover the `kid` that
   its caller had already decoded; `validate_with_jwks` takes the `kid` as an
   argument. Same verdict, one less base64+JSON parse per request.
   `JwtValidator::validate` documents why it must decode the header before
   delegating.
4. **No `IssuerTrustMap`/`SingleJwksResolver`/DPoP support.** This deployment
   has exactly one issuer (its own OP over loopback), and the resolver machinery
   is unreachable from `resource_auth`. Porting unused generality would be code
   no test could reach.
5. **`JwksCache::get_jwks` re-checks the cache under the write guard**
   (`JwksCache::refresh_if_stale`). The original drops its read guard and calls
   `refresh()`, which takes the write guard and fetches unconditionally
   (`jwt.rs:169-177`), so a caller that queued behind another one re-fetches a
   JWKS that was just stored. Here it serves what the earlier waiter stored
   instead. This is a fix, not a bug-for-bug carry-over, and it is the one place
   this port knowingly behaves differently from upstream at runtime.

   Worth stating exactly, because the obvious description of this ("N concurrent
   requests at a TTL boundary cost N fetches") is **wrong** and was measured to
   be wrong before this was written: `tokio::sync::RwLock` is write-preferring,
   so once the first caller queues on `write()` every later reader blocks at
   `read()` and then sees the fresh entry. With the re-check deleted, 32 callers
   released together from a `Barrier` on a 32-worker runtime, over 20 TTL
   boundaries, cost **one** extra fetch on 17 rounds and two on 3 — never 32.
   The re-check removes that residual one, and turns "usually 1, sometimes 2"
   into "1". Pinned by
   `a_caller_that_reaches_the_refresh_with_a_fresh_entry_does_not_fetch_again`,
   which tests the rule rather than racing for it — see that test for why racing
   for it cannot be made decisive.

Not deviations, and worth stating because they look like ones:

- **The write lock is still held across the HTTP GET**, exactly as upstream
  holds it. Deviation 5 bounds how many GETs a TTL boundary costs, not how long
  the lock is held for the one that is made: a JWKS fetch still blocks every
  concurrent validation in the process for its duration. Fetching outside the
  lock would change much more of the original's policy (two callers could then
  store out-of-order results) than the re-check does, and the fetch is a
  loopback request here.
- **`JwksCache::get_key`'s miss path still refreshes unconditionally**, "in case
  of rotation". The same re-check there would suppress precisely the fetch that
  exists: the `get_jwks` call immediately above it has just stored a fresh
  entry, so any "fetched within the last N ms" test is true by construction and
  the cache would stop noticing a key published since its last refresh. That
  path is remotely triggerable by anyone who can put a `kid` in a header, and
  `resource_auth`'s `UNKNOWN_KID_REFRESH_INTERVAL` — one permitted delegation
  per interval per process, test-and-stamp under a single lock, so a concurrent
  burst spends one permit between them — remains its mitigation, bounding it to
  the two fetches `a_hundred_unknown_kids_force_at_most_two_jwks_fetches`
  measures.

---
