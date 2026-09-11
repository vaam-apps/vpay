# `vpay-api` reference

Why the code in `backends/crates/vpay-api` looks the way it does. The crate's
own doc comments say _what_ each item is and link here; this page carries the
reasoning, the ports, the measurements and the history that a reader needs
once — not on every `cargo doc` build.

Tier: an [ADR](../adr/) records a decision, a [flow](../flows/) describes a
process, and a reference page like this one explains why a particular piece of
code is shaped the way it is.

- [The router](#the-router)
  - [Route tree](#route-tree)
  - [Middleware order](#middleware-order)
- [The rest of this reference](#the-rest-of-this-reference) — a page per
  surface, under [`vpay-api/`](vpay-api/)

_This list was a full table of contents of a 1 566-line page until 2026-09-11.
One of its entries had also been wrong since 2026-09-07: it offered "Why the
`client_id` check is not redundant with the audience" as an anchor, and
[ADR-0017](../adr/0017-staff-authentication.md) had renamed that heading to
"What identifies the credential (ADR-0017), and what it replaced". An in-page
anchor that stops resolving is a broken link `verify-links` cannot report,
because it strips the fragment before it checks the path — which is why the
entries below name pages rather than headings._

---

## The router

`vpay_api::router` is the one place the process's HTTP surface is assembled.

### Route tree

Three groups, and which group a path falls into is the whole security boundary
of this process:

| Path                                             | Auth                           | Why                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------------------ | ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /healthz`                                   | none                           | A probe must answer before anything is configured, and it reveals only whether Postgres is reachable. It is the _readiness_ probe; liveness is `/livez` on the observability port.                                                                                                                                                    |
| `POST /v1/oauth/token`                           | none                           | The credential _is_ the request body (RFC 7523 `client_assertion`). Requiring a bearer token to get a bearer token is circular.                                                                                                                                                                                                       |
| `GET /v1/oauth/.well-known/openid-configuration` | none                           | How a client that has never spoken to vpay finds the token endpoint.                                                                                                                                                                                                                                                                  |
| `GET /v1/oauth/jwks.json`                        | none                           | How a verifier that has never spoken to vpay learns the public keys. Same circularity.                                                                                                                                                                                                                                                |
| **anything else under `/v1/oauth`**              | none                           | The OP subtree is public by design; its own `.fallback(not_found)` answers the honest 404 rather than letting the path escape to the outer router.                                                                                                                                                                                    |
| `GET /v1/browser/payment_intents/{id}`           | none                           | A payer's browser has no merchant credential. The payment intent's own `client_secret` is what authorises it — see `vpay_api::browser`.                                                                                                                                                                                               |
| `POST /v1/browser/payment_intents/{id}/confirm`  | none                           | The same.                                                                                                                                                                                                                                                                                                                             |
| **anything else under `/v1/browser`**            | none                           | Its own `.fallback(not_found)`, for the OP nest's reason: without one the path would match `/v1/{*rest}` and answer 401 to a caller that can never hold a token.                                                                                                                                                                      |
| **everything else under `/v1`**                  | `AuthenticatedMerchant`        | The merchant API.                                                                                                                                                                                                                                                                                                                     |
| `GET /dash/v1/payment_intents`                   | `require_dashboard_token`      | The staff dashboard's payments list. Mounted **only** when `dashboard_client` is configured; otherwise the path falls through to the outer 404. See [the dashboard surface](vpay-api/dashboard-surface.md#the-dashboard-surface-dash).                                                                                                |
| `GET /dash/v1/payment_intents/{id}`              | `require_dashboard_token`      | The payment detail: the intent, its charge, its refunds, its event timeline.                                                                                                                                                                                                                                                          |
| **anything else under `/dash/v1`**               | `require_dashboard_token`      | Its own `.fallback(not_found)`, for the OP nest's reason — and every non-read method is refused by the boundary before the router matches.                                                                                                                                                                                            |
| `POST /provider/{code}/callback`                 | **none, and none is possible** | A payment rail telling us something happened. Neither MTN nor Orange signs a callback or sends a shared secret, so there is no credential to check — which is exactly why the handler may not write charge or intent state. See [the rail callback route](vpay-api/provider-callback.md#the-rail-callback-route-provider_callbackrs). |
| **anything else under `/provider`**              | none                           | Its own `.fallback(not_found)`, for the OP nest's reason.                                                                                                                                                                                                                                                                             |
| anything else                                    | none                           | The honest 404.                                                                                                                                                                                                                                                                                                                       |

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
The refund pair is the one place a _read_ is mounted without its create, and
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
`/v1/...` path would answer an _unauthenticated_ 404 and tell an anonymous
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
`/v1/{*rest}` — the _authenticated_ nest — and answered **401**. Removing the
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

A _known_ OP path with the wrong method — `GET /v1/oauth/token` — gets axum's
own bare `405`, not this crate's envelope. Left as-is deliberately: 405 is the
correct status, and turning it into the 404 envelope would tell an integrator
the path does not exist when it does. The gap is that its body is empty rather
than the Stripe envelope; that is worth fixing when a `method_not_allowed`
renderer exists for the whole surface, not one route at a time.

### Middleware order

`ServiceBuilder` applies layers outside-in, so the list below is the order a
_request_ traverses them, and the reverse of the order a response does. All five
are load-bearing in that order:

1. `discard_unusable_request_id` — removes a caller's `x-request-id` unless it
   is short and plain enough to carry (`is_usable_request_id`). It must be
   first, and above the minting layer specifically: step 2 only mints when the
   header is _absent_, so this step's removal is exactly what causes a fresh id
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

## The rest of this reference

`docs/reference/vpay-api.md` was 1 566 lines until 2026-09-11. It is the router
and the overview now; each surface has its own page below, moved **verbatim**.
Nothing was summarised away and no dated measurement or correction was dropped.

| Surface                                             | Page                                                                                   |
| --------------------------------------------------- | -------------------------------------------------------------------------------------- |
| The merchant OP (`op/`)                             | [vpay-api/merchant-op.md](vpay-api/merchant-op.md)                                     |
| Resource-server JWT validation (`resource_auth.rs`) | [vpay-api/resource-auth.md](vpay-api/resource-auth.md)                                 |
| The dashboard surface (`dash/`)                     | [vpay-api/dashboard-surface.md](vpay-api/dashboard-surface.md)                         |
| The JWKS cache (`jwks_cache.rs`)                    | [vpay-api/jwks-cache.md](vpay-api/jwks-cache.md)                                       |
| The form decoder (`form.rs`)                        | [vpay-api/form-decoder.md](vpay-api/form-decoder.md)                                   |
| The confirm path, and the return trip               | [vpay-api/confirm-path.md](vpay-api/confirm-path.md)                                   |
| Checkout Sessions                                   | [vpay-api/checkout-sessions.md](vpay-api/checkout-sessions.md)                         |
| The account-holder route, and Customers             | [vpay-api/account-holders-and-customers.md](vpay-api/account-holders-and-customers.md) |
| The rail callback route (`provider_callback.rs`)    | [vpay-api/provider-callback.md](vpay-api/provider-callback.md)                         |
| Boot (`boot.rs`)                                    | [vpay-api/boot.md](vpay-api/boot.md)                                                   |
