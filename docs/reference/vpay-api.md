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
- [Checkout Sessions (`v1/checkout_sessions.rs`, `browser/checkout_sessions.rs`)](#checkout-sessions-v1checkout_sessionsrs-browsercheckout_sessionsrs)
  - [The credential ladder](#the-credential-ladder)
  - [`payment_intent` is an id on `/v1` and expanded on `/v1/browser`](#payment_intent-is-an-id-on-v1-and-expanded-on-v1browser)
  - [Which publishable key a session pins](#which-publishable-key-a-session-pins)
  - [`checkout_not_configured` answers 500, not the plan's 503](#checkout_not_configured-answers-500-not-the-plans-503)
  - [What ends a browser read: the clock, and `open`](#what-ends-a-browser-read-the-clock-and-open)
  - [`merchant.name`, and why there is a fallback](#merchantname-and-why-there-is-a-fallback)
- [The rail callback route (`provider_callback.rs`)](#the-rail-callback-route-provider_callbackrs)
  - [Why the only thing it may do is move a `run_at`](#why-the-only-thing-it-may-do-is-move-a-run_at)
  - [What an anonymous caller can and cannot get out of it](#what-an-anonymous-caller-can-and-cannot-get-out-of-it)
  - [Why an unknown reference is a 202 and an unknown rail code is a 404](#why-an-unknown-reference-is-a-202-and-an-unknown-rail-code-is-a-404)
  - [What it deliberately does not repair](#what-it-deliberately-does-not-repair)
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
| `POST /provider/{code}/callback` | **none, and none is possible** | A payment rail telling us something happened. Neither MTN nor Orange signs a callback or sends a shared secret, so there is no credential to check — which is exactly why the handler may not write charge or intent state. See [the rail callback route](#the-rail-callback-route-provider_callbackrs). |
| **anything else under `/provider`** | none | Its own `.fallback(not_found)`, for the OP nest's reason. |
| anything else | none | The honest 404. |

`/livez` and `/metrics` are **not** in that table and are not served by this
router at all. They belong to `vpay_api::observability`, on
`--observability-bind` (default `0.0.0.0:9090`), because `/metrics` names every
rail, route pattern and error code this deployment has and must not be
reachable from whatever fronts the traffic port. The chart's NetworkPolicy
encodes that, and it can only do so because the two are different ports.

`/v1/browser` is the only nest carrying a `CorsLayer`; the merchant `/v1` nest
and the `/provider` callback nest deliberately carry none — the first because
nothing legitimate calls it from a browser and a permissive header there would
invite a merchant to put a bearer token in a page, the second because its
caller is a rail's own backend and there is no origin to allow.

`/provider` sits **outside** `/v1` on purpose. It is not part of the merchant
API — no SDK calls it and it carries no resource version — and mounting it
inside the one prefix whose whole boundary is "everything here needs a bearer
token" would put an unauthenticated route inside it. The path is also not a
free choice: `vpay_config::ProviderHost::effective_callback_url` has derived
`{public_base_url}/provider/{code}/callback` since Step 3, and both adapters
have been sending it to their rails ever since, so this is the route that
address was always pointing at.

The `/v1` nest mounts `v1::V1_ROUTES` and a 404 fallback for everything else,
which is the production behaviour and not a placeholder: `/v1/payment_intents`,
`/v1/events`, `/v1/checkout/sessions` and — since 2026-09-05, issue #45 —
`GET /v1/refunds/{id}` are real, and `POST /v1/refunds` and `/v1/balance` are
not implemented and are therefore not routed ([status.md](../status.md)).
The refund pair is the one place a *read* is mounted without its create, and
`v1::refunds`' own module doc carries the argument: creating a refund needs
`ProviderAdapter::refund`, which no adapter implements, while reading one is
the authoritative read every other money movement on this surface has. The
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

   **1b**, in the same breath and from one read: ask the intent's **checkout
   session**, if it has one, whether this confirm may happen at all — a
   session that is not `open` refuses it with a `409`
   (`checkout_session_expired` / `checkout_session_complete`) — and, when one
   does drive it, where the payer must come back to. It is a step of its own
   rather than a seventh, because it is the same question step 1 asks (may
   this be confirmed?) about a different object, and because `confirm_once`'s
   own doc comment counts six;

2. resolve the rail from `payment_method_data[type]` and branch on its
   `vpay_provider::Capabilities::flow` only, never on its code
   ([ADR-0002](../adr/0002-provider-port.md));
3. mint the `provider_reference_id` and commit the charge row in `submitting`,
   so the reference is durable before anything is sent;
4. record the attempt in `provider_requests` with no status;
5. call the adapter with the `return_url` step 3 committed on the charge row
   — `submit` is `async`, so the `.await` is what actually sends the request.
   (It read the merchant's URL out of the request here until Step 9's lane 1b
   moved the resolution to 1b and the value into `charges.return_url`, so that
   what the rail is told is what a crash would leave behind.);
6. record what came back and answer.

Each step is a named function in the source (`load_confirmable_intent`,
`return_trip::admit_confirm`, `resolve_rail` with `payer_instrument`,
`open_attempt` covering steps 3 and 4, `submit_to_rail`, `finish_confirm`);
`confirm_once` is the sequence and nothing else, so "what order do these
happen in" is answerable by reading a dozen lines.

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

### What the checkout session says, and where the payer comes back to (`v1/return_trip.rs`)

**A confirm on an intent whose checkout session is over is refused before any
charge is opened.** Added 2026-09-05. A session's `status` is a promise vpay
has already made: the hourly sweep emits `checkout.session.expired`, and
`POST /v1/checkout/sessions/{id}/expire` is the merchant's own statement that
the checkout is done. Neither retracts the payer's credential — the intent's
`client_secret` is minted at create and lives as long as the intent — so a
payer holding a stale checkout link could pay anyway, and the settlement's own
`WHERE status = 'open'` guard would then correctly decline to touch the
session, leaving `expired`/`unpaid` under a `succeeded` intent and a merchant
holding a webhook that said the opposite.

* The refusal is `ApiError::CheckoutSessionNotOpen`, `Category::Conflict`
  (409, `invalid_request_error`, `Retry::Never`), with **two codes** chosen by
  the state: `checkout_session_expired` and `checkout_session_complete`. Not
  the category default `invalid_state`, for the reason
  `idempotency_key_in_flight` is not `idempotency_key_in_use` — a merchant
  must be able to tell "your payer walked away" from "this intent is already
  processing". Not one code plus a `param` either: `param` on this API names a
  *request parameter* (`ApiError::param` renders it only for `InvalidParam`),
  and the request that trips this carries no reference to a session at all.
* It fires on **both** surfaces, because it lives in `confirm_once`, which
  both share. That is deliberate rather than incidental: `/v1`'s confirm is
  not authenticated by the payer's `client_secret` at all, so a merchant
  server that kept confirming after its own systems recorded the checkout as
  abandoned would produce exactly the contradiction the browser refusal
  prevents.
* An `open` session past `expires_at` that no sweep has reached is expired
  **on the read**, and the read writes nothing. Same rule and same reasoning
  as `browser::checkout_sessions::authenticate`'s sixth refusal: a worker that
  is down must not be the difference between a payer being able to pay and
  not, and a confirm is the wrong place to repair a row — flipping it here
  would emit no `checkout.session.expired` and would skip the `NOT EXISTS`
  live-charge guard the sweep's transaction carries.
* An intent with **no** session is unaffected, and an intent whose session was
  expired and replaced by a new open one is payable through the new one:
  `CheckoutSessions::find_latest_by_intent` reads the newest row, and
  `checkout_sessions_one_open_per_intent` is what makes "an open session is
  the newest" true rather than hoped for
  ([vpay-db.md](vpay-db.md#find_latest_by_intent--the-same-question-with-the-status-filter-off)).

The refusal and the return URL are **one read**, not two. Asking separately
would race the hourly sweep: a gate that read `open`, followed by a
return-URL lookup running a millisecond after `expire_due` committed, would
admit the confirm and then submit it to the rail with no return URL at all.

`vpay_provider::ChargeRef::return_url` is filled here and nowhere else (Step
9's D2; [rails.md](rails.md) has the rail-side half and what it replaced). The
question has two answers and they belong to different owners, which is why it
is a trait and not two lines inside the confirm:

* a charge driven by a **checkout session** returns to vpay's own return page
  for that session, because vpay has to poll the intent before it can forward
  the payer to the merchant's `success_url` or `cancel_url`;
* every other charge returns to the merchant's own `charges.return_url` — the
  URL they sent on `confirm`, already validated by `checked_return_url` and
  already echoed back to them as `next_action.redirect_to_url.return_url`.
  This is what closes [browser-checkout.md](../flows/browser-checkout.md)'s D4
  for integrations that never create a session.

It runs **before** `open_attempt`, which is what makes both refusals above cost
no charge row, no `provider_requests` row and no job — and after
`load_confirmable_intent`, never before it, because this answer is not the
uniform 404 and asking it first would let a caller learn that some other
tenant's intent has a checkout session on it. (Until Step 9's lane 1b it ran
*after* `open_attempt` and handed its answer straight to the adapter; the
value now goes into `charges.return_url` before the charge is committed, so
what the rail is told is the value that would survive a crash rather than a
second read that could differ from what was made durable.)

`CheckoutSessionGate`'s shipping impl is `SessionGate`, which holds the
repositories *and* `ResourceConfig::checkout_public_base_url()` — both, because
the URL needs a row (`CheckoutSessions::find_latest_by_intent`) and a configured
origin, and `CheckoutSessionRow::return_page_url` is the one place the two are
joined. It was a blanket impl over `dyn Repositories` answering `None` for every
intent while Step 9's lane 2 was ahead of the `checkout_sessions` table; lane 1b
replaced it, and `a_session_driven_confirm_sends_vpays_return_page_to_the_rail`
(`backends/tests/integration/tests/confirm_rails.rs`) is what would fail if it
ever went back — measured: make the branch answer `Ok(None)` and the rail is
told `https://shop.example/order/1234/return` instead of the session's page.

A session on a deployment with **no** `checkout.public_base_url` is refused
(`ApiError::CheckoutNotConfigured`, message
`CHECKOUT_SESSION_WITHOUT_CHECKOUT_APP`) rather than fallen back from. It is
unreachable by any merchant request — `POST /v1/checkout/sessions` refuses
before a session can exist — and reachable only by an operator deleting the key
while sessions are open. Falling back to the merchant's URL there is precisely
the silent failure this seam exists to prevent: the payer is forwarded one step
too early and nothing reports it. The refusal fires for a push rail too, which
would have ignored the URL, because the deployment's checkout page is gone
either way and an outage that depended on which rail a payer picked would be
worse to debug than one that does not. It is reached only *after* the session
has admitted the confirm, so an expired session on a deployment that has lost
its checkout page still answers `checkout_session_expired` — the more specific
and more actionable of the two.

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

## Checkout Sessions (`v1/checkout_sessions.rs`, `browser/checkout_sessions.rs`)

Step 9. Four merchant routes and three payer routes over one object. The
schema reasoning is in [vpay-db.md](vpay-db.md#checkout_sessions); what
follows is what the HTTP layer adds.

The merchant surface is ordinary `/v1`: token-authenticated, tenant-scoped,
`Idempotency-Key` on both POSTs through the same `PostRequest` the payment
intents use — shared rather than copied, because a second claim/finish/release
dance is exactly how one of the two ends up leaving a merchant's key stuck
`in_flight`.

`create` refuses three things with a `409` and not a `400`: an intent that is
not `requires_payment_method`, an intent that already has a charge, and an
intent that already has an open session. All three are facts about an
*object's state* rather than about the request's shape. The third is checked
twice on purpose: `find_open_by_intent` first, so the merchant gets a sentence
naming the session in the way, and then the partial unique index, which is the
actual guard — between that read and the insert a concurrent create can commit
one, and the `UniqueViolation` is turned into the same `409` so a merchant
cannot see two different errors for one situation depending on timing.

The URL rules (`checked_forward_url`) deliberately do **not** parse the URL:
`{CHECKOUT_SESSION_ID}` (D5) is a literal substring a merchant writes, and
`url::Url::parse` percent-encodes the braces. A validator that normalised
would have to either store the normalised form — breaking the substitution the
placeholder exists for — or discard its own parse, proving nothing. So the
rules are a scheme prefix from a closed two-entry list, a character count, and
`https` under `deployment.livemode`; the column's CHECKs in migration `0028`
are the backstop for a writer that forgets, not the primary guard.

### The credential ladder

| Route | Presents | May read |
|---|---|---|
| `GET /v1/checkout/sessions/{id}` | a merchant bearer token | the session, `client_secret` and `url` (which carries `?key=` and `#client_secret`) |
| `GET /v1/browser/checkout/sessions/{id}` | `key` + the **session's** `client_secret` | the session, `payment_intent` expanded **with the intent's own `client_secret`** |
| `GET /v1/browser/checkout/sessions/{id}/return` | `key` + the session's `return_token` | the session, `payment_intent` expanded **without** it |
| `GET /v1/browser/checkout/origins` | `key` alone | the tenant's `checkout_origins` |

The escalation this closes, read upwards: `return_token` (a query-string value
that reaches access logs) → the session read → the intent's `client_secret` →
`confirm`. Every hop is refused, and two of them are worth stating because
they are easy to reintroduce:

* **The two browser reads are separate path patterns, not one handler with
  two optional parameters.** Each builds only the expected value it accepts,
  so a return token cannot open the session read.
* **Neither browser read renders the session's `url`.** It carries the
  session's own `client_secret` in its fragment, so echoing it on the return
  read would hand the weaker credential's holder the stronger one. It costs
  the page nothing: a payer on the session read is already *at* that URL, and
  a payer on the return page has no use for it.

The choice is expressed as a *type* — `ExpandableIntent::Expanded` versus
`::ExpandedWithSecret` — rather than as a field one handler clears, so a route
that wanted to render a credential has to say the word.

`browser::checkout_sessions` reuses `browser::secrets_match`, and therefore
`browser::ct_compare`, rather than introducing a second constant-time compare.
There is one in the crate, proven once, in the place its own doc comment
explains.

The origins route answers `200 {"origins": []}` for an unknown key rather than
a 404, which is the same confidentiality property arrived at from the other
side: an empty list is also what a *registered* tenant with no origins gets,
so the two are indistinguishable and nobody can enumerate a deployment's
merchants by trying keys. It is also the fail-closed answer — no origins means
no embedding.

### `payment_intent` is an id on `/v1` and expanded on `/v1/browser`

Stripe's `expand` shape (`model::ExpandableIntent`, `#[serde(untagged)]`: a
string or an object, no discriminator).

* `/v1/checkout/sessions` — **the id**. A merchant already holds the intent
  they created, so expanding would put a second, possibly stale copy of every
  amount on the wire, multiplied by the page size on the list.
* `/v1/browser/checkout/sessions/{id}` and `.../return` — **the object**.
  vpay's own page has only a session id and a session secret; it needs the
  amount, the currency, the status, `payment_method_types` (which rails to
  offer), `next_action` and `last_payment_error` before it can paint anything,
  and two round trips for that would mean two loading states instead of one.
  The session read adds `client_secret`; the return read does not.

`PaymentIntentObject` itself is untouched by any of it — the twelve-key
tripwire (`every_documented_key_is_present_including_the_null_ones`) still
stands, and `PaymentIntentWithSecret` is still the only wrapper that carries a
credential.

### Which publishable key a session pins

Every URL vpay mints for the checkout app carries `?key={pk}`, because all
three browser routes authenticate by it and the return page cannot use a
fragment — a payer arrives there from a URL the *rail* replays.

`create` takes an optional `publishable_key`. Named and registered to this
tenant → that one; omitted → the tenant's **first configured key**, in the
order the operator wrote it (`first()` and not "any", so a merchant can
predict their own link from their own YAML); named but not theirs → `400`
naming the parameter; **no keys at all** → `checkout_not_configured`.

That last answer shares its `code` with a missing `checkout.public_base_url`
and differs only in the sentence: from a merchant's side both are "this vpay
cannot do hosted checkout for me", and only the message tells whoever they
forward it to which line of YAML to add.

The unregistered-key answer is a `400` and **not** the uniform 404 the browser
surface gives, and the asymmetry is the point. On `/v1/browser` the caller is
an unauthenticated payer and any distinction between "unknown key" and
anything else is an enumeration oracle. Here the caller is the merchant,
authenticated, asking about their own registration: there is nothing to hide
from them, and a uniform refusal would leave them unable to tell a typo from a
key they forgot to register. The key is echoed back for the same reason.

The chosen key is **stored on the row**, not re-derived on read — see
[vpay-db.md](vpay-db.md#publishable_key-is-a-column-and-return_page_url-is-a-method)
for why a key rotation would otherwise strand payers mid-flight.

The hosted `url` is `{base}/c/{cs_id}?key={pk}#{client_secret}`, and the two
halves are on opposite sides of the `#` because they need opposite things: the
page reads `key` **server-side**, in `middleware.ts`, to look up
`checkout_origins` and set `frame-ancestors` before any script runs (D4) — and
a fragment never reaches a server — while the credential must never reach one
(D6).

### `checkout_not_configured` answers 500, not the plan's 503

`ApiError::CheckoutNotConfigured` carries the code the Step 9 plan asks for
and **not** the status. This is a deliberate departure, recorded here because
it is the kind of thing that otherwise looks like an oversight.

ADR-0011 derives the status from the `Category`, never from a call site, and
`Category::Storage` is the only one that answers `503`. Classifying a missing
`checkout.public_base_url` as storage would be wrong twice: it would tell an
operator Postgres was unreachable, and — the part that actually costs
something — `Category::Storage`'s `Retry::AfterBackoff` would tell a
merchant's SDK to retry a request that cannot succeed until someone deploys a
configuration change. `Category::Configuration` says exactly what is true
("the deployment is misconfigured for this operation … fixed by a deploy,
never by retrying") and its status is `500`.

Making it a `503` honestly would mean either a new `Category` or moving
`Category::Configuration` to `503` — both ADR-level changes affecting every
error in the workspace, and both a maintainer's decision rather than a lane's.
The `code` is what an SDK branches on, and it is `checkout_not_configured`
either way.

### What ends a browser read: the clock, and `open`

Two rules that are not the same rule, and both are on the **read**.

**`expires_at` ends both reads, whatever the `status`.** The `return_token`
travels in a query string — it has to; a fragment does not survive a rail's
redirect — so a copy of it is in the rail's own storage, in whatever the rail
logs, and in the checkout app's access logs. D10's 24 hours is the bound on how
long that copy is worth anything, and until Step 9's lane 1b it bounded
nothing: `expires_at` was written at create and read by no one. Past the
horizon both reads answer the uniform 404, byte-identical to a wrong
credential's, so a stale token cannot learn that the session existed.

Deliberately not conditioned on `status`: a `complete` session's return page is
the screen the whole redirect leg exists to reach, and refusing it would break
the successful case. What ends the reads is the clock.

And deliberately **not** left to the expiry sweep
([vpay-worker.md](vpay-worker.md#the-housekeeping-sweep-retires-a-fourth-thing-and-tells-someone-about-it)).
The sweep leaves a session with a live charge `open` on purpose, it runs at
most once an hour, and a deployment whose worker was down would keep answering
these reads for the length of the outage. The sweep makes `status` honest to a
*merchant*; the read is what refuses a payer credential.

**`status = 'open'` gates the intent's `client_secret`, and nothing else.**
That credential is handed over so the page can drive
`POST /v1/browser/payment_intents/{id}/confirm`. Once the session is `complete`
or `expired` there is nothing left to confirm, and re-issuing it on every later
read would keep a live intent credential in circulation for a checkout that is
over — reachable by anyone holding the session secret, which is in the URL the
payer was sent. A settled session still reads `200` with the outcome; the page
loses nothing, because it read the secret on its first call and polls with the
copy it holds.

### `merchant.name`, and why there is a fallback

Both browser reads render `merchant: { name }` — the one fact about the
merchant a payer is shown, and a member `frontends/apps/checkout` *requires*:
its `isSessionEnvelope` guard refuses a session envelope without it, so a
server that rendered nothing made every session read `error.unexpected`. The
field is `name` and not `display_name` because that is what the guard reads;
this is a wire contract with a TypeScript app that cannot be type-checked
against this crate, so an integration test pins the literal.

It rides on `model::CheckoutSessionForPayer`, a wrapper that flattens the
session, rather than on `CheckoutSessionObject` — `CheckoutSessionWithSecret`'s
argument pointing the other way. A field on the object would put a
deployment-configured value on all four `/v1` responses and on every row of the
list, where a merchant reading their own sessions already knows who they are.

The value is `merchant_clients[].display_name`, and when a merchant configured
none the `merchant` member is **absent** — `ResourceConfig::merchant_display_name`
answers `None` and nothing is rendered in its place. ~~The first version of this
lane fell back to the tenant id~~ (retired 2026-09-04 by the integrator, once
lane 3b made the page tolerate a missing `merchant`): a payer is asked to hand
over money on the strength of recognising who they are paying, and a tenant id
or a `client_id` is an internal identifier, not a name. The fix for a nameless
merchant is configuration, and `config/application.yml` says so beside the
field. The page paints a neutral heading for the absent case.

## The account-holder route (`v1/account_holders.rs`)

`GET /v1/account_holders`, mounted 2026-09-05 for
[issue #47](https://github.com/vaam-apps/vpay/issues/47).
[../flows/account-holder-lookup.md](../flows/account-holder-lookup.md) is the
process and the policy; this section is why the *code* is shaped the way it
is.

### The handler matches the port's result rather than `?`-ing it

Every other `/v1` handler that reaches a rail writes
`adapter.submit(..).await?` and lets `ApiError`'s `#[from]` and `Classify`
do the rest. This one matches, and the reason is the counter: `error` and
`not_found` are two *different* outcomes on
`vpay_account_holder_lookups_total`, and a `?` would leave the failure arm
invisible — "a merchant is asking a rail that is not answering" is exactly the
rate an operator wants, and it is the one thing a route with no persistence
leaves no other trace of.

The classification is still not re-decided: the `Err` arm counts, logs a
masked line, and hands the error to `ApiError` unchanged, which is what
derives the status (ADR-0011).

### `MerchantScope` is bound and unused

There is nothing to scope: no query runs, no row is read, and the answer is a
property of the rail. The extractor is bound anyway, because it is what makes
the *authentication* boundary structural rather than remembered —
`MerchantScope::from_request_parts` fails closed with a paging 500 when the
middleware is not mounted, so a refactor that dropped the layer would fail
here instead of serving an unauthenticated identity lookup on a route that
returns a stranger's name. It is also the value an audit log would be keyed on
the day that reserved decision is taken.

### Three refusals, one envelope

An unknown rail, a rail an operator has **disabled**, and a rail whose
`supports_account_holder_lookup` is false all produce the byte-identical `400`
naming `payment_method_type` (`unsupported_rail`, one function, asserted by
`a_disabled_or_unknown_rail_is_the_same_refusal_as_an_incapable_one`).
Telling them apart would let a merchant enumerate which rails a deployment has
configured but switched off, and the fix is the same for all three.

It is a `400` and not the `409` `ProviderError::Unsupported` classifies to,
because the rail is never *called*: ADR-0002 asks the core to branch on the
capability first, and at that point the wrong thing is the merchant's
parameter, which is what `param` should name.

### The MSISDN validator is the first server-side copy of that rule

`frontends/apps/checkout/src/lib/msisdn.ts` had been the only implementation
of "Cameroon E.164, three input spellings, this separator set". The two are
deliberately **not** shared: the browser's is a form affordance that also
formats for display, and this one is a trust boundary — the page can be
bypassed entirely by a merchant calling `/v1` directly, which is the ordinary
way this route is used. Sharing would mean the server trusting a client-side
rule. What is shared is the specification, and both files say so.

Validating at all, rather than letting MTN refuse a malformed number, buys
three things: a rail call on our own credentials is not spent on an input we
could see was not a phone number; the hex WireMock steering numbers
(`237600000f01`) are unreachable through a production-shaped route; and an
arbitrary path segment cannot be interpolated into MTN's API. The adapter
percent-encodes it as well (`vpay_provider::http::path_segment`), belt and
braces, because the port is reachable by any caller in the process.

### The mask, and the column it does not write

`masked` produces `+2376••••200` — the shape
`charges.payer_ref_masked` is documented to hold, with a **fixed** four
bullets rather than one per hidden digit, because a mask whose length revealed
the input's length would be a small oracle for free.

**Nothing writes that column.** The confirm path stores `NULL`
(`open_attempt`), so this is the first producer of the shape in the workspace
and the two are not wired together. That is a gap rather than an oversight:
writing the column is a change to the charge path, not to this route, and
`docs/status.md` says so rather than this function pretending otherwise.

## The rail callback route (`provider_callback.rs`)

`POST /provider/{code}/callback`, mounted since Step 8 lane C. Before it, both
adapters implemented `parse_callback`, nothing in a running vpay called
either, and the `X-Callback-Url`/`notif_url` every submit carried pointed at a
host that answered 404 — so settlement was polling-only and the poll ladder's
first rung is ten seconds (`vpay_worker::poll_delay(0)`).

### Why the only thing it may do is move a `run_at`

AGENTS.md: "Callbacks are hints. `parse_callback` returns identifiers only,
never a status. The authenticated status query is the only thing that moves
money." The port enforces the first half — `CallbackRef` has no status field
to put one in, and `parse_callback` is deliberately synchronous so an adapter
cannot fetch one either (ADR-0002, [provider-port.md](../flows/provider-port.md)).
This route is the second half: it resolves the adapter, parses identifiers,
finds the charge, and then runs **two statements in one transaction** —
`enqueue_in_tx`, which is `ON CONFLICT DO NOTHING` and writes nothing in the
ordinary case, and `vpay_db::TxRepositories::pull_forward_in_tx`, an `UPDATE
jobs SET run_at = now()` on that charge's existing `poll:<charge id>` job.
(This page said "exactly one write" until Step 8's review; the enqueue is
there for what the ladder cannot cover — a job an operator deleted, or one
already finished — and it is a write a reader counting statements against an
unauthenticated route needs to know about.)

That write is new, and it is deliberately **not** `enqueue_in_tx` growing a
`DO UPDATE`. The argument against the upsert
([vpay-db.md](vpay-db.md#enqueue_in_tx-exists-only-in-the-transactional-form))
is unchanged: the backstop scan re-enqueues every live charge's key every ten
minutes, and an upserting enqueue would drag a job scheduled a quarter of an
hour out back to now on every pass — a ladder that silently becomes a hot
loop. So a caller has to ask for the pull-forward, and exactly one does. It
refuses three states, each for its own reason: a **leased** job is being
polled right now and that poll will see the answer; a job already at or before
`now()` needs nothing (which is what makes a burst of duplicate callbacks free
rather than a row-lock queue); and a **parked** job — `run_at = 'infinity'` —
stays parked, because the whole point of a dead letter is that its
`dedupe_key` keeps scans *and callbacks* from re-creating work a human has to
look at first.

Since Step 8's review it refuses a fourth: a job **already due within
`PULL_FORWARD_FLOOR`** — ten seconds, which is the poll ladder's own fastest
rung, `vpay_worker::poll_delay(0)`. The number is written out in
`provider_callback` because `vpay-api` cannot name `poll_delay` (the
dependency runs the other way), and
`the_pull_forward_floor_is_the_poll_ladders_first_rung` in
`backends/tests/integration/tests/provider_callback.rs` is the join that fails
if the two drift.

### What an anonymous caller can and cannot get out of it

This page used to say that everything such a caller could gain was "bounded by
what the ladder was going to do anyway". **That was wrong**, and the review
that found it is the reason the floor exists. What is true:

- A flood of callbacks about one charge is one row, forever: the `dedupe_key`
  carries a unique index.
- A charge the queue is about to ask about anyway now costs a caller nothing.
  A poll due inside the floor is left where it is, so the POST is two
  statements against `jobs`, no row changed, and **no rail request**. That
  covers the common case, because the ladder's first rung is where a charge
  sits immediately after its first poll.
- A charge parked *further out* than the floor is still brought forward by
  every callback, and that is what the route is for. It is also the residual:
  the rungs grow (20 s, 30 s, 45 s, …) while the floor stays at ten, so a
  caller repeating against one live charge can hold it at roughly one
  authenticated `query_status` per worker claim. **There is no rate limit** —
  not per charge, not per source — and [status.md](../status.md) says so.
- What actually stands between the route and rail traffic is therefore: the
  caller must know a v4 `provider_reference_id` for a live charge *on this
  deployment*; the work each accepted POST buys is one authenticated status
  query, which settles the charge the rail names or nothing at all; the body
  is bounded at 16 KiB; and nothing here writes charge or intent state under
  any circumstances.

The cost of the floor is stated where it is paid: a rail's callback arriving
while the charge sits on the ladder's first rung no longer settles it early —
it settles at that rung, up to ten seconds later than it would have before.
`a_callback_does_not_accelerate_a_poll_that_is_already_about_to_run` is that
behaviour, asserted rather than implied, and the headline case parks its job
at a later rung for the same reason.

The read behind it, `Charges::get_by_provider_reference`, is scoped by
`provider_code` as well as by the reference. The rail is named by a path
segment and the reference by a body anyone can write, so a lookup that ignored
the code would let a POST to one rail's callback path name another rail's
charge. Migration `0027` indexes exactly that pair, because this is the one
read in the system an unauthenticated caller can trigger at will and a
sequential scan over `charges` would be a denial-of-service surface that grows
with the deployment's own success.

### Why an unknown reference is a 202 and an unknown rail code is a 404

Both rails retry a non-2xx on their own schedules, so answering `404` to a
reference this deployment has no charge for buys a retry loop that can never
succeed. It would also make the endpoint an oracle for "does this charge
exist", which is the argument [`browser`](#the-router)'s uniform 404 already
makes about an unauthenticated surface. So the two 202s — "queued" and "never
heard of it" — are the same response, and the second is logged at `info` where
an operator debugging a misregistered callback host will find it.

An unknown *rail code* is different and stays a 404: it is a statement about
this deployment's route table, which is public (a merchant learns the same set
from `payment_method_types`), and a rail whose code nobody linked is not a rail
that is going to retry. The handler produces it by calling `crate::not_found`
rather than by building its own `ApiError`, so "byte-identical to a mistyped
path" is structural instead of two literals that happen to agree —
`the_provider_nest_is_unauthenticated_and_its_404_is_the_routers_own` compares
the bodies.

### What it deliberately does not repair

`CallbackRef::ref_extra` is **discarded**. Orange's `parse_callback` carries a
`notif_token`, and sometimes a `pay_token`, out of the notification, and
`docs/flows/adapter-orange-money.md` names repairing a charge whose
`ref_extra` write was lost as a thing a callback *could* do. It is not done
here, and doing it would need the stored `notif_token` compared against the
received one first — which nothing implements. Merging an unauthenticated
request's rail key material onto the row would corrupt the token the next
status query is addressed by, so the honest state is "not built", and
`docs/status.md` says so. `a_callback_writes_no_charge_or_intent_state` is
what holds it there.

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
