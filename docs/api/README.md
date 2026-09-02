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

**Status: not implemented server-side.** Only `/healthz` and a Stripe-shaped
404 exist — that includes the OAuth2 token endpoint this auth model needs,
which also does not exist yet. See [../status.md](../status.md).

**Client-side, two SDKs implement this surface** — [`sdks/rust`](../../sdks/rust)
(`vpay-sdk`) and [`sdks/nodejs`](../../sdks/nodejs) (`@vpay/sdk`). They do
the `private_key_jwt` handshake, token caching, the form-encoded resource
calls in the table below and webhook verification. The exact wire contract
they implement, and the server must serve, is
[../flows/merchant-auth.md](../flows/merchant-auth.md). They are tested
against HTTP stubs of that contract and against the real Authkestra
assertion verifier; they have never completed a request against a running
vpay, because none serves `/v1`.

Planned subset:

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

**Status: not implemented.**

## `/provider/{code}/callback` — rail callbacks

Public and unauthenticated by necessity. A callback **never changes state** — it
only enqueues a status query. See [../flows/reconciler.md](../flows/reconciler.md).

**Status: not implemented.**
