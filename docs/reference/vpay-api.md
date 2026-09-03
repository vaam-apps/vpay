# `vpay-api` reference

Why the code in `backends/crates/vpay-api` looks the way it does. The crate's
own doc comments say *what* each item is and link here; this page carries the
reasoning, the ports, the measurements and the history that a reader needs
once — not on every `cargo doc` build.

Tier: an [ADR](../adr/) records a decision, a [flow](../flows/) describes a
process, and a reference page like this one explains why a particular piece of
code is shaped the way it is.

- [The router](#the-router)
  - [Route tree](#route-tree)
  - [Middleware order](#middleware-order)
- [The merchant OP (`op/`)](#the-merchant-op-op)
  - [Why vpay writes its own handlers](#why-vpay-writes-its-own-handlers)
  - [The token endpoint speaks RFC 6749, not the Stripe envelope](#the-token-endpoint-speaks-rfc-6749-not-the-stripe-envelope)
  - [The reference copy is not a dependency](#the-reference-copy-is-not-a-dependency)
  - [Deliberate deviations from `axum_token_handler`](#deliberate-deviations-from-axum_token_handler)
- [Resource-server JWT validation (`resource_auth.rs`)](#resource-server-jwt-validation-resource_authrs)
- [The JWKS cache (`jwks_cache.rs`)](#the-jwks-cache-jwks_cachers)
- [The form decoder (`form.rs`)](#the-form-decoder-formrs)
- [The confirm path](#the-confirm-path)
- [Boot (`boot.rs`)](#boot-bootrs)

---

## The router

`vpay_api::router` is the one place the process's HTTP surface is assembled.

### Route tree

Three groups, and which group a path falls into is the whole security boundary
of this process:

| Path | Auth | Why |
|---|---|---|
| `GET /healthz` | none | A probe must answer before anything is configured, and it reveals only whether Postgres is reachable. It is the *readiness* probe; liveness is `/livez` on the observability port. |
| `POST /v1/oauth/token` | none | The credential *is* the request body (RFC 7523 `client_assertion`). Requiring a bearer token to get a bearer token is circular. |
| `GET /v1/oauth/.well-known/openid-configuration` | none | How a client that has never spoken to vpay finds the token endpoint. |
| `GET /v1/oauth/jwks.json` | none | How a verifier that has never spoken to vpay learns the public keys. Same circularity. |
| **anything else under `/v1/oauth`** | none | The OP subtree is public by design; its own `.fallback(not_found)` answers the honest 404 rather than letting the path escape to the outer router. |
| `GET /v1/browser/payment_intents/{id}` | none | A payer's browser has no merchant credential. The payment intent's own `client_secret` is what authorises it — see `vpay_api::browser`. |
| `POST /v1/browser/payment_intents/{id}/confirm` | none | The same. |
| **anything else under `/v1/browser`** | none | Its own `.fallback(not_found)`, for the OP nest's reason: without one the path would match `/v1/{*rest}` and answer 401 to a caller that can never hold a token. |
| **everything else under `/v1`** | `AuthenticatedMerchant` | The merchant API. |
| anything else | none | The honest 404. |

`/livez` and `/metrics` are **not** in that table and are not served by this
router at all. They belong to `vpay_api::observability`, on
`--observability-bind` (default `0.0.0.0:9090`), because `/metrics` names every
rail, route pattern and error code this deployment has and must not be
reachable from whatever fronts the traffic port. The chart's NetworkPolicy
encodes that, and it can only do so because the two are different ports.

`/v1/browser` is the only nest carrying a `CorsLayer`; the merchant `/v1` nest
deliberately carries none.

The `/v1` nest mounts `v1::V1_ROUTES` and a 404 fallback for everything else,
which is the production behaviour and not a placeholder: `/v1/payment_intents`
and `/v1/events` are real, and `/v1/refunds` and `/v1/balance` are not
implemented and are therefore not routed ([status.md](../status.md)). The
boundary is observable in three answers —

- `GET /v1/payment_intents/pi_x` with no bearer token → **401**, the
  `ApiError::Auth` envelope;
- the same request with a valid merchant token, for an id this merchant has no
  intent under → **404**, the `resource_missing` envelope;
- `GET /v1/balance` with a valid token → **404**, the `unknown_route` envelope.

— which is exactly what a merchant integrating against this deployment should
get. Inventing a `/v1/balance` so the third answer could be a `200` is the
failure mode `CLAUDE.md` names first.

The authentication layer is `require_merchant_token` via `from_fn_with_state`
(Step 2's D3 — that function's docs say why it is not
`from_extractor_with_state`), mounted with `Router::layer` on the nested router
so that it wraps that router's fallback too. `route_layer` is the wrong tool and
axum says so: it does not apply to a fallback by design, so an unmatched
`/v1/...` path would answer an *unauthenticated* 404 and tell an anonymous
caller which `/v1` resources exist. When this nest had no routes at all, axum
refused that spelling outright ("Adding a route_layer before any routes is a
no-op"); now that it has routes, the swap would compile and be silently wrong,
which is why the choice is written down rather than left to the compiler —
`an_unauthenticated_v1_request_is_401_not_404` is what actually catches it.

A path under `/v1/oauth` that matches no OP route (say `/v1/oauth/authorize`,
which vpay does not serve) answers an unauthenticated 404 — and does so from
the OP router's **own** `.fallback(not_found)`. That outcome is intended: the
whole `/v1/oauth` subtree is public by design, so a 404 there leaks nothing a
merchant could not learn from the discovery document.

**The fallback is load-bearing, not decoration, and the code's comment used to
say the opposite.** It previously claimed that an unmatched `/v1/oauth/...`
path "falls through to the outer router's fallback and answers an
unauthenticated 404". Measured, it did not: with no fallback on the OP router,
axum flattens that nest's three routes into the outer path table and registers
no `/v1/oauth/{*rest}` entry at all, so `GET /v1/oauth/not_a_route` matched
`/v1/{*rest}` — the *authenticated* nest — and answered **401**. Removing the
`.fallback(not_found)` reproduces it, and `the_oauth_nest_answers_its_own_404`
fails with `left: 401, right: 404`.

A 401 there is the wrong answer twice over: it tells an integrator who mistyped
an OP path to present a bearer token, on the one subtree whose entire purpose is
handing out bearer tokens to callers that do not have one yet — and it made
"which router serves this path" depend on an axum flattening detail rather than
on anything written down. With the fallback the OP router is closed over its own
prefix: every `/v1/oauth/...` path is served by the OP router, unmatched ones
included, and that is a property a test checks rather than an accident of
registration order.

A *known* OP path with the wrong method — `GET /v1/oauth/token` — gets axum's
own bare `405`, not this crate's envelope. Left as-is deliberately: 405 is the
correct status, and turning it into the 404 envelope would tell an integrator
the path does not exist when it does. The gap is that its body is empty rather
than the Stripe envelope; that is worth fixing when a `method_not_allowed`
renderer exists for the whole surface, not one route at a time.

### Middleware order

`ServiceBuilder` applies layers outside-in, so the list below is the order a
*request* traverses them, and the reverse of the order a response does. All five
are load-bearing in that order:

1. `discard_unusable_request_id` — removes a caller's `x-request-id` unless it
   is short and plain enough to carry (`is_usable_request_id`). It must be
   first, and above the minting layer specifically: step 2 only mints when the
   header is *absent*, so this step's removal is exactly what causes a fresh id
   to be minted for a caller whose own id was not usable.
2. `SetRequestIdLayer` — mints an `x-request-id` (a v4 UUID, via
   `MakeRequestUuid`) on the request, **unless the caller already sent one that
   step 1 kept**, in which case theirs is kept. Everything below reads the
   header it sets.
3. `mirror_request_id_header` — copies the id step 2 settled on onto the
   response a second time, as `request-id`, because that is the only spelling
   stripe-node reads. Below step 2 because it reads the request header step 2
   guarantees is there; above step 5 only because nothing makes the order
   between them matter — both take the value from the request, so neither can
   observe the other.
4. `TraceLayer` — opens `make_request_span` around the handler, so the id is on
   the span before any handler, extractor or error renderer runs, and every
   event they emit inherits it.
5. `PropagateRequestIdLayer` — innermost, so it sees the id step 2 set and is
   the first layer to touch the response on the way out; it copies the
   request's id onto the response, which is what makes `Category::Internal`'s
   "Contact support with the request id" a promise a merchant can act on.

Step 1 is `axum::middleware::from_fn` rather than
`tower::util::MapRequestLayer`: `MapRequestLayer` sits behind tower's `util`
feature, which the workspace pin (`tower = "0.5"`, no feature list) does not
enable — it is on today only through feature unification from an unrelated
transitive dependency, so using it would make this stack compile by accident.
`from_fn` needs no feature axum does not already have.

The stack is mounted on the outermost router, so it wraps every group above —
including the 401 an unauthenticated `/v1` request gets, which is the response
most likely to be the one a confused integrator is holding, and which therefore
needs a request id on it more than any other.

---

## The merchant OP (`op/`)

The merchant-facing OAuth2 provider behind `/v1/oauth`
([ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md),
[merchant-auth.md](../flows/merchant-auth.md)). Four pieces, each its own module
so it can be tested on its own:

- `clients` — the `ClientStore` the OP looks merchants up in: the statically
  registered `merchant_clients` from YAML, minus anything in the
  `disabled_clients` kill switch.
- `keys` — the RS256 signing key: loaded from a file at boot, never persisted;
  its `kid` and public JWK are what `oauth_signing_keys` records and
  `/jwks.json` publishes.
- `jwks` — the vpay-owned `/jwks.json`, publishing every key in its rotation
  window from the database rather than the one key this process holds.
- `token` — the two HTTP handlers this crate writes itself: the RFC 6749 token
  endpoint and the discovery document.

`MerchantOp` is the assembly: it holds the one `OpConfig`, the one `OpStore` and
the one `TokenManager` that `token::token_handler` needs, built once at boot
from a validated `Config`, a `keys::LoadedSigningKey` and the repositories.

Nothing in this module serves the dashboard surface: `/dash/v1` login is a
separate, later step, and this OP is deliberately pinned to the one grant `/v1`
uses.

### Why vpay writes its own handlers

`authkestra-axum` ships `axum_token_handler`/`axum_discovery_handler` and vpay
does not use them, for three reasons that are all about *not* serving surface
this deployment does not implement:

1. Its router helpers mount the authorization-code, device and userinfo
   endpoints alongside the token endpoint. vpay serves none of those (see
   `OP_GRANT_TYPES`), and a route that exists only to answer an error is a route
   an integrator can find and misread.
2. `axum_authorize_handler` needs `tower_cookies::Cookies` in the request
   extensions, so mounting that crate's OP routes drags a cookie layer into a
   router whose entire `/v1` surface is cookie-free bearer auth.
3. Its handlers reach their state through `FromRef<AppState>` for
   `Result<Arc<dyn OpStore>, AxumError>` and render their own errors, which
   would put a second error-rendering path next to `ApiError`
   ([ADR-0011](../adr/0011-error-modelling.md) wants one).

What vpay does *not* re-implement is the protocol itself: `token::token_handler`
calls `authkestra_op`'s own `handle_token` directly, and the status mapping it
applies is copied from `authkestra-axum-0.7.1/src/op.rs::axum_token_handler`.

Nothing on this surface reads or sets a cookie. `/v1` is bearer-token only
(ADR-0010), and the cookie-bearing half of authkestra's OP
(`axum_authorize_handler`, which requires `tower_cookies::Cookies` in the
request extensions) is not mounted, which is why `tower-cookies` is not a
dependency of this crate at all.

### The token endpoint speaks RFC 6749, not the Stripe envelope

Every other failure in this crate is rendered by `ApiError` as
`{"error":{"type","code","message"}}` (ADR-0011: one renderer). The token
endpoint is the deliberate exception, and it has to be: it is an OAuth2
authorization server endpoint, and RFC 6749 §5.2 fixes the body as
`{"error":"invalid_client","error_description":"…"}`. Every OAuth client in
existence parses that shape, including vpay's own — `sdks/rust`'s
`Client::fetch_token` tries `TokenErrorResponse` (`error` +
`error_description`) and falls through to `Error::UnexpectedResponse`
otherwise, and `sdks/nodejs` does the same. Rendering the Stripe envelope here
would make every SDK report "unexpected response" instead of "invalid_client",
which is precisely the diagnostic a merchant needs.

So `op::token` renders `authkestra_op`'s own `TokenErrorResponse` verbatim and
does not route through `ApiError`. The status mapping — `invalid_client` → 401,
everything else → 400 — is copied from `axum_token_handler`, so a merchant
integrating against vpay sees the same statuses as against any other authkestra
deployment.

### The reference copy is not a dependency

**`authkestra-axum` is deliberately not in this workspace's dependency graph**
(the three reasons above), so it is in neither `Cargo.lock` nor the local
registry cache: there is nothing on disk to diff a bump against. On an
`authkestra-op` version bump, fetch the matching reference copy yourself —

```text
https://static.crates.io/crates/authkestra-axum/authkestra-axum-<version>.crate
```

— and compare `src/op.rs::axum_token_handler` against `op::token`, whose tail as
of `0.7.1` (lines 239-247) is inlined here so the comparison target is local and
a drift is visible without a download:

```text
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(err) => {
            let status = match err.error.as_str() {
                "invalid_client" => StatusCode::UNAUTHORIZED,
                _ => StatusCode::BAD_REQUEST,
            };
            (status, Json(err)).into_response()
        }
```

`token_handler` and its `token_error_status` are together that, with `resp`/`err`
renamed. Note what is *not* copied: the same file's
`axum_device_authorization_handler` maps `"invalid_client" |
"unauthorized_client"` to 401, and matching *that* arm here would be a bug —
vpay serves no device-authorization endpoint, and RFC 6749 §5.2 makes
`unauthorized_client` a 400 on the token endpoint.

### Deliberate deviations from `axum_token_handler`

`token_handler` is a port of that function (read it: it is ~50 lines). Three
things are left out, each on purpose:

- **No DPoP header handling.** `axum_token_handler` reads the `DPoP` header and
  passes it to `handle_token_with_client_cert`. vpay wires no `DpopReplayStore`
  (`MerchantOp::new`), and `authkestra_op`'s `NoDpopReplayStore` fails closed —
  so a `DPoP` header would be answered `invalid_dpop_proof` rather than
  honoured. Calling `handle_token` (which passes `None`) means a `DPoP` header
  is *ignored* instead, and the client gets a plain Bearer token it can actually
  use. Neither behaviour is DPoP support; ignoring it is the one that does not
  fail a request over an unsupported extension.
- **No mTLS client certificate.** vpay does not terminate TLS in this process
  ([ADR-0004](../adr/0004-musl-mimalloc.md): the image is `FROM scratch` behind
  an ingress), so there is no certificate to bind a token to and RFC 8705
  `cnf.x5t#S256` is not offered.
- **No device, authorize or userinfo route.** See "Why vpay writes its own
  handlers" above.

---

## Resource-server JWT validation (`resource_auth.rs`)

`docs/api/README.md` defines three surfaces with three different protections;
this module builds the validation layer for the two that need one — `/v1`
(merchant, `client_credentials` + `private_key_jwt`) and `/dash/v1` (dashboard,
a staff OIDC session, one read-only scope). See
[ADR-0009](../adr/0009-dashboard-oidc-provider.md) (vpay runs its own Authkestra
OP) and [ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md) (merchant
auth).

**Status:** the merchant half is live and guards real rows.
`vpay_api::require_merchant_token` — which validates through
`MerchantJwtValidator` and hands `AuthenticatedMerchant` its claims — is mounted
in front of the whole `/v1` nest by `vpay_api::router`, and `vpay-server` builds
the validator behind it against this process's own JWKS. Behind that boundary is
`/v1/payment_intents`, whose every query is filtered by the tenant the
middleware resolved. The dashboard half is unmounted — `AuthenticatedDashboard`
exists, no `/dash/v1/*` route does, and nothing constructs a
`DashboardJwtValidator`.

### Validation is local, not a network round trip per request

`vpay_api::jwks_cache::JwksCache` fetches the JWKS once and caches it
(`jwks_refresh_interval`); every call after that looks the key up by `kid` from
memory and verifies the signature locally with `jsonwebtoken`. That cache is a
narrowed port of `authkestra_resource::jwt::JwksCache` (see
[the JWKS cache](#the-jwks-cache-jwks_cachers) for why it had to be ported) and
it keeps the original's refresh policy bar one deliberate change: `get_key` only
calls `refresh()` (an HTTP GET) on a cache miss or once the TTL has elapsed,
never on every validation, and the TTL refresh re-checks the cache once it holds
the write guard, so a boundary crossed by any number of concurrent requests
costs exactly one fetch. That is what makes this safe to put in front of a
payment-processing route.

### …except on an unrecognized `kid`, which is why this module throttles

The other half of `get_key` — carried over from the original unchanged,
deliberately (`jwt.rs` ~181-193): when the cached JWKS does **not** hold the
requested `kid`, it calls `refresh()` unconditionally — "in case of rotation" —
and `refresh()` holds the cache's write lock *across* the HTTP GET. On top of
that, key resolution happens **before** the signature is verified, so nothing
about the token has been checked at that point.

Unthrottled, that makes an unauthenticated request with a random `kid` in its
header a remote control for two things at once: one loopback
`GET /v1/oauth/jwks.json` per request (which in this deployment is a Postgres
`SELECT`, so the amplification lands on the database), and a write lock held
across that round trip, which blocks every *legitimate* validation in the
process for its duration. `JwtValidator::validate` closes both by deciding,
before it delegates, whether this `kid` is one the process has ever seen a valid
token for; `UNKNOWN_KID_REFRESH_INTERVAL`'s own doc comment states the throttle
and the trade-off it makes.

### A sharp edge in `jsonwebtoken`'s default audience validation

`jsonwebtoken::Validation::validate_aud` defaults to `true`, but the check it
gates only runs *if the token has an `aud` claim at all* — confirmed by reading
`jsonwebtoken-11.0.0/src/validation.rs`: the doc comment on `validate_aud`
itself says "Validation only happens if `aud` claim is present", and the
`_ => {}` fallthrough arm of `validate`'s `match (claims.aud, options.aud.as_ref())`
proves it — a token with no `aud` claim reaches that arm and passes regardless of
what `set_audience` was told. A token minted with no audience at all would
therefore sail through unchecked, which is exactly the kind of ambiguity this
module is required to fail closed on (a missing claim, not merely a wrong one).

The fix is not to hand-roll audience comparison: `JwtValidator::new` calls
`set_required_spec_claims(&["exp", "aud", "iss"])`, which makes the `aud`
claim's mere *presence* mandatory — a token with no audience is rejected as a
missing required claim before the comparison logic ever runs — and
`set_audience` continues to do the real membership check with the library's own
(tested) logic. Covered by `a_token_with_no_audience_claim_at_all_is_rejected`.

### `authkestra_resource::jwt::JwtStrategy` was deliberately not used

`JwtStrategy<I>`'s `cache` and `validation` fields are both private with no
accessor — confirmed by reading `jwt.rs`: neither field is `pub`, and no getter
or setter method exists for either. Once a `JwtStrategy` is built via
`ValidationConfig`/`ValidationConfigBuilder`, nothing outside the crate can
inspect or adjust its `Validation` afterwards. That is a real limitation — a
sibling project had to work around it by setting `validation.validate_aud =
false` and hand-rolling the audience check itself — but it does not bite this
module, because this module never constructs a `JwtStrategy` at all. It drives
its own `jwks_cache` (a port of the `pub` `JwksCache`/`validate_jwt_generic`
pair) and builds its own `jsonwebtoken::Validation`, so every field — including
`validate_aud` and `required_spec_claims` — stays under this module's control
from construction onward. `ValidationConfigBuilder` does expose
`.audience()`/`.audiences()`, so the *audience* half of the sibling project's
problem would not necessarily recur through `JwtStrategy` either — but the
`required_spec_claims` fix above has no equivalent builder method at all, which
alone would have forced hand-rolling (or living with the gap) had `JwtStrategy`
been used.

---

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
([ADR-0004](../adr/0004-musl-mimalloc.md)) and has none, so `vpay-server`
panicked at boot inside its own image — see `vpay_provider::http` for the full
account of that failure.

`JwksCache::with_client` (authkestra#301) looks like the seam for this and is
not: it is a consuming builder that *replaces* the client `new` has already
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
`jsonwebtoken::decode`. What is ported is the cache's *policy* — when to
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

## The form decoder (`form.rs`)

The Stripe-style bracket-nested `application/x-www-form-urlencoded` decoder, and
the two extractors (`VpayForm`, `VpayQuery`) that put it in front of a handler.

This is the *reading* half of a wire contract whose writing half already ships in
two SDKs — `sdks/rust/src/form.rs` and `sdks/nodejs/src/form.ts`, which are
byte-for-byte identical to each other by test. It is therefore a deliberate
port, not a general-purpose form parser: every rule below is chosen because it
is what that encoder emits, and the `node_parity` test module decodes the exact
byte strings those SDKs' own parity tests pin. If this file and those disagree, a
merchant's request means one thing to their SDK and another to us.

### Why not `serde_urlencoded` (what `axum::Form` uses)

It is a *flat* decoder: `metadata[order_id]=1234` becomes a key literally
spelled `metadata[order_id]`, and `payment_method_types[0]=a` with
`payment_method_types[1]=b` becomes two unrelated fields. Nesting is the whole
encoding here ([merchant-auth.md](../flows/merchant-auth.md)'s table), so the
shape has to be rebuilt rather than deserialized directly.

### The grammar

A body is `pair(&pair)*`, a pair is `key=value` (a pair with no `=` is a key
with an empty value), and a key is a head segment followed by zero or more
bracket groups:

```text
amount                                   -> ["amount"]
metadata[order_id]                       -> ["metadata", "order_id"]
payment_method_data[mtn_momo][msisdn]    -> ["payment_method_data", "mtn_momo", "msisdn"]
payment_method_types[0]                  -> ["payment_method_types", 0]
payment_method_types[]                   -> ["payment_method_types", next]
```

A bracket group holding only digits, or nothing at all, makes the parent an
**array**; anything else makes it an **object**. Both array spellings are
accepted because both are in the contract: the SDKs send `[0]`, `[1]` (as
`stripe-node`/`stripe-rust` do), and `examples/merchant-curl` and Stripe's own
curl documentation use `[]`.

### Brackets are split before segments are decoded, and that ordering is load-bearing

The encoder escapes a `[` *inside* a key segment as `%5B` precisely so it cannot
be mistaken for nesting, and its own test
`escapes_a_bracket_that_appears_inside_a_key_segment` pins `metadata[a%5Bb]=v`.
Decoding the whole key first would turn that back into `metadata[a[b]` and the
split would then invent a nesting level the merchant never asked for — so the
split runs on the raw key and each segment is percent-decoded afterwards,
yielding the key `a[b`. (Step 2's design states the two operations in the other
order; the example it states in the same sentence is what fixes the order, and it
is the example that is testable. See
`metadata_key_with_an_escaped_bracket_is_one_key`.)

### `+` is a literal plus, never a space

Both SDKs escape with JavaScript's `encodeURIComponent`, which renders a space as
`%20` and leaves `+` alone. Applying the WHATWG
`application/x-www-form-urlencoded` rule (`+` → space) would silently corrupt
exactly the field that can least afford it: an MSISDN written `+237670000000`.
`serde_urlencoded`, and therefore `axum::Form`, does apply that rule — which is
the second reason this decoder exists.

### Everything is a string

Form encoding has no types: `amount=5000` and `description=5000` arrive
identically. So `parse_form` produces `serde_json::Value::String` for every
scalar, and a `T` deserialized through `VpayForm` must take `String` fields (or
carry its own `deserialize_with`). That is not a limitation being tolerated — it
is what lets a handler answer "amount must be a positive integer of minor units"
in vpay's own words, with `param: "amount"`, instead of leaking serde's sentence
for a field the caller can see.

### Bounds

The request body limit is **64 KiB**, applied by a
`tower_http::limit::RequestBodyLimitLayer` on the `/v1` nest rather than in the
decoder: a limit enforced by an extractor has already buffered the body it is
refusing. Over that, the layer answers `413` before this code runs — see
`a_body_over_the_limit_is_refused_by_the_layer`, which mounts the same layer over
these extractors. Nesting is bounded to `MAX_DEPTH` independently, because
64 KiB of `a[a][a][a]…` is cheap to send and recursion is not.

---

## The confirm path

`POST /v1/payment_intents/{id}/confirm` and
`POST /v1/browser/payment_intents/{id}/confirm` share one implementation,
`confirm_once`. Its six steps are the order
[crash-safety.md](../flows/crash-safety.md) requires, and their order is the
whole safety property:

1. load the intent for this merchant (404), refuse a status that forbids a
   confirm (409), refuse an intent that already has a charge (409, and **before**
   any insert — "one charge per intent, forever");
2. resolve the rail from `payment_method_data[type]` and branch on its
   `vpay_provider::Capabilities::flow` only, never on its code
   ([ADR-0002](../adr/0002-provider-port.md));
3. mint the `provider_reference_id` and commit the charge row in `submitting`,
   so the reference is durable before anything is sent;
4. record the attempt in `provider_requests` with no status;
5. call the adapter — `submit` is `async`, so the `.await` is what actually
   sends the request;
6. record what came back and answer.

Each step is a named function in the source (`load_confirmable_intent`,
`resolve_rail` with `payer_instrument`, `open_attempt` covering steps 3 and 4,
`submit_to_rail`, `finish_confirm`); `confirm_once` is the
sequence and nothing else, so "what order do these happen in" is answerable by
reading eleven lines.

Step 6 has three shapes, and which one runs is decided by the *error's* own
classification rather than by anything this file knows about rails:

* **the rail accepted it** — one transaction moves the charge to `submitted`
  with the rail's key material and the intent to `processing`/`requires_action`,
  it commits, and only then is a response built (`persist_submitted`, and
  crash-safety.md's "the commit is the gate on the redirect");
* **the rail declined it** (`ProviderError::Rejected`) — one transaction fails
  the charge with its `failure_code` and stamps `last_payment_error` on the
  intent, which stays `requires_payment_method` because the lifecycle has no
  `failed` status; the merchant gets the `409` `charge_declined`;
* **anything else** — we do not know what the rail did, so *nothing* moves. The
  `submitting` charge row and the status-less `provider_requests` row stay
  behind on purpose: they are exactly the state a crash between steps 4 and 6
  would leave, and are what the recovery pass reads.

`Malformed` is the one arm where "no answer" is not literally true — bytes came
back, they just did not parse — and it is grouped with the unknown cases
deliberately. What the recovery table decides is whether to go and *ask the
rail*, and an unparseable answer is exactly as unknown as a lost one: "every
ambiguity resolves toward 'find out', never 'give up'".

### Why `confirm_once` takes seven loose arguments and not a struct

Each one is a distinct dependency of the six steps — the repositories, the
deployment's rails, the linked adapters, the tenant, the object, the payer's
instrument, and what the response renders — and the two callers pass different
values for every one of them. A parameter struct would put a constructor between
the two surfaces and this function, which is exactly the place a
browser-specific default could hide unnoticed. It sits at clippy's threshold,
deliberately: an eighth would be the signal that the confirm has grown a second
responsibility.

### Idempotency

Every `POST` needs an `Idempotency-Key` (Step 2's D7). The key is claimed
*atomically* — one `INSERT … ON CONFLICT`, in `vpay_db`'s `Idempotency::claim` —
so two concurrent requests carrying one key cannot both proceed, and the loser is
told "in progress" rather than being allowed to create a second intent. A claim
is always ended, on every path: stored (the response is replayable) or released
(the retry must re-execute). "Every path" is meant literally, including the ones
that fail *after* the work was done: a body that cannot be read back, a body
that is not JSON, and a failed write to `idempotency_keys` each release before
returning, because the alternative is the key staying `in_flight` until it
expires and every retry under it being answered "still in progress".

The claim is carried as the `claim_id` the claim minted, never as the key alone:
an expired claim is reclaimable, so addressing the row by key would let a request
that stalled past its window overwrite or delete the claim that replaced it.

The refusal of unsupported Stripe parameters runs *before* the claim, where the
body is decoded — so a refused confirm stores nothing and leaves the key unspent,
exactly as a body that fails to decode already does. Running it first is safe
because the check is config-independent (it reads the body alone), so a genuine
replay, whose body is byte for byte the one that was accepted, can never be
shadowed by it. What the ordering is visible in is the other case: a confirm
reusing a completed key with a newly added refused field answers that field's
`400` rather than `idempotency_key_in_use`.

---

## Boot (`boot.rs`)

`vpay_api::boot` is the one derivation both binaries run before they serve
anything: the linked adapters keyed by `providers.code`, the YAML joined
against them into reference-table seeds, and the connect/migrate/reconcile
sequence in the order [configuration.md](../flows/configuration.md) fixes. See
[vpay-config.md § the boot sequence](vpay-config.md#the-boot-sequence) for the
ordering itself and why each step is where it is.

`vpay-api` is the home because it is the only crate both binaries already link
that also depends on `vpay-config` (the YAML), `vpay-provider` (the port) and
`vpay-db` (the seed types). No new dependency edge exists because of it.

It used to live in both binaries. `vpay-server`'s `main.rs` and
`vpay-worker-bin`'s carried verbatim copies of `adapters_by_code`, `boot_seeds`,
`flow_label` and `display_name_for` — about 150 lines each, with comments
explaining that the duplication was deliberate. It was not safe: the two
processes reconcile the *same two tables* in the same database, so a change to
one copy and not the other is a rollout where `providers.display_name` or
`providers.flow` flips back and forth depending on which binary restarted last,
with nothing to report it. The previous arrangement had no drift guard of any
kind — not a shared test, not a compile-time link.

What stays per-binary is the thing that genuinely *is* per-binary: the four-line
`adapters()` list of linked rails (Step 2's D6). A worker that learned which
rails exist from `vpay-server`'s crate would make its capabilities a function of
the API server, and the two deploy independently.
`cargo xtask verify-no-mocks` walks the dependency graph from each binary root,
so the list has to be reachable from that root to be checked.

### Every adapter comes back wrapped in `Measured`

`adapters_by_code` is where `vpay_provider_requests_total` and
`vpay_provider_request_duration_seconds` get their one seam. The wrap happens
there rather than in either binary's `adapters()` list because that list is
deliberately duplicated per binary and a metric mounted in a duplicated list is
one the copies eventually disagree about. Everything that resolves a rail goes
through that function: both `main`s, and the integration suite's own harness.

`Measured` delegates every method to the adapter it wraps and returns it as a
plain `Box<dyn ProviderAdapter>`, so no caller can tell — or branch on — whether
it is holding a wrapper. It is not a substitute for a rail and adds no code path
that exists only outside production
([ADR-0006](../adr/0006-no-mocks-in-main-processes.md)); it is the shipping
process measuring itself.

The conformance suite constructs adapters directly and is therefore *not*
measured, which is correct: it exercises one adapter against a stub, and its
counts would say nothing about a deployment.
