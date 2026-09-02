# API

vpay exposes two HTTP surfaces, and conflating them is a security bug.

## `/v1` — the merchant API (Stripe-shaped object model, not Stripe-shaped auth)

Authenticated with OAuth2 `client_credentials` + `private_key_jwt` (RFC
7523) — **not** an API key. Each merchant is a statically registered OAuth2
client, configured directly in vpay's YAML config
([ADR-0003](../adr/0003-yaml-configuration.md)), holding its own private
key; vpay stores only the merchant's **public** JWK. There is no
`sk_live_`/`sk_test_` key, no database-stored secret, and no other way to
authenticate on this surface. See
[ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md), which supersedes
the scope-boundary paragraph of
[ADR-0009](../adr/0009-dashboard-oidc-provider.md) that had kept `/v1` on
Stripe-shaped API keys.

Everything else about `/v1` is still Stripe-shaped: form-encoded bodies,
Stripe's object model, Stripe's error envelope, Stripe's idempotency
semantics.

**Errors.** Every non-2xx response is the Stripe envelope
`{ "error": { "type", "code", "message", "param"? } }`, and the status,
`type` and `code` are *derived* from the failing error's classification
(`vpay_core::error::Classify` → `vpay_api::ApiError`), never chosen per
handler — so the same failure always gets the same answer. The full
category → status/type/code table is in
[../flows/errors.md](../flows/errors.md); the decision is
[ADR-0011](../adr/0011-error-modelling.md). `message` is the error's
public message and never contains hosts, tables, library text or
credentials; the full chain goes to the server log.

**Status: the OAuth2 endpoints are served; no resource is.** As of
2026-09-02, `vpay-server` serves three unauthenticated OP endpoints — and
they are unauthenticated by necessity, not omission: requiring a bearer
token to obtain a bearer token is circular, and a client that has never
spoken to vpay has to be able to find the token endpoint and the keys.

| Method | Path | Auth | What it does |
|---|---|---|---|
| POST | `/v1/oauth/token` | none | `client_credentials` + `private_key_jwt` only. RFC 6749 error JSON; `invalid_client` is 401, every other error 400. |
| GET | `/v1/oauth/.well-known/openid-configuration` | none | Hand-built, so it advertises only what this deployment serves — no `/authorize`, no `/userinfo`, no device or refresh grant, `private_key_jwt` as the only client-auth method. |
| GET | `/v1/oauth/jwks.json` | none | Every publishable signing key (the active one plus any retired-but-unexpired key inside the rotation window), `Cache-Control: public, max-age=300`. |

The issuer is `{deployment.public_base_url}/v1/oauth` and is a deployment
setting, not a per-client one. `GET /v1/oauth/token` — the right path with
the wrong method — returns axum's bare `405` with an empty body rather than
the Stripe envelope; the status is correct and a `method_not_allowed`
renderer for the whole surface is the fix, not a special case here.

**Every other `/v1` path requires a merchant access token, and there is
nothing behind it.** An unauthenticated request gets `401`
`authentication_error`/`missing_bearer_token`; an authenticated one gets
`404 unknown_route`. **None of the resources in the table below is
implemented** — that 404 is the honest answer and a `200` would mean someone
invented a resource. See [../status.md](../status.md) for the tests behind
each of those claims, and for the state of the evidence: the integration
suite covering this flow has run once, manually, against a scratch database,
and never under Docker or in CI.

**Client-side, two SDKs implement this surface** — [`sdks/rust`](../../sdks/rust)
(`vpay-sdk`) and [`sdks/nodejs`](../../sdks/nodejs) (`@vpay/sdk`). They do
the `private_key_jwt` handshake, token caching, the form-encoded resource
calls in the table below and webhook verification. The exact wire contract
they implement, and the server must serve, is
[../flows/merchant-auth.md](../flows/merchant-auth.md). They are tested
against HTTP stubs of that contract and against the real Authkestra
assertion verifier. Since 2026-09-02 the **Rust** SDK completes the whole
handshake against a real `vpay_api::router` in
`backends/tests/integration/tests/merchant_token_flow.rs` — and then gets
the honest 404, because no resource exists. The **Node** SDK has still never
spoken to a vpay of any kind.

Planned subset — **none of these is implemented; each returns the 404
envelope to an authenticated caller:**

| Method | Path |
|---|---|
| POST | `/v1/payment_intents` |
| GET | `/v1/payment_intents/:id` |
| POST | `/v1/payment_intents/:id/confirm` |
| POST | `/v1/payment_intents/:id/cancel` |
| GET | `/v1/payment_intents` |
| POST | `/v1/refunds` |
| GET | `/v1/events` |
| GET | `/v1/balance` |

Everything else returns a Stripe-shaped 404 naming vpay honestly, rather than
pretending the route exists.

## `/dash/v1` — the dashboard API

Authenticated with **OIDC sessions** (authorization code + PKCE), never a
merchant credential of any kind. Called server-side from Next.js only.

Keeping these separate from `/v1` is deliberate, even though both surfaces
are OAuth2-shaped now: a merchant's `client_credentials` token authenticates
a *backend service* with standing payment authority over its own account,
while a dashboard session authenticates a *human*, scoped to one read-only
capability, for as long as they stay logged in. Conflating the two would let
a browser-held credential reach payment-authority endpoints, or a merchant
integration reach staff-only state. See
[ADR-0008](../adr/0008-dashboard-scope.md) and
[ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md).

**Status: not implemented.** No `/dash/v1` route, no `/login`, no
`/authorize`, no session store — `authkestra-engine` is pinned without its
`sql-postgres` feature, so none is compiled in. The signing keys and the
JWKS endpoint that landed on 2026-09-02 serve `/v1` and are not a step
toward this surface being reachable; there is also an unresolved audience
problem in the authorization-code grant that must be settled first. Tracked
as Phase 2b in [../roadmap.md](../roadmap.md); see
[../flows/dashboard-auth.md](../flows/dashboard-auth.md).

## `/provider/{code}/callback` — rail callbacks

Public and unauthenticated by necessity. A callback **never changes state** — it
only enqueues a status query. See [../flows/reconciler.md](../flows/reconciler.md).

**Status: not implemented.**
