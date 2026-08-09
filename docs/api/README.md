# API

vpay exposes two HTTP surfaces, and conflating them is a security bug.

## `/v1` — the merchant API (Stripe-shaped)

Authenticated with `sk_live_` / `sk_test_` keys. Form-encoded bodies, Stripe's
object model, Stripe's error envelope, Stripe's idempotency semantics.

**Status: not implemented.** Only `/healthz` and a Stripe-shaped 404 exist. See
[../STATUS.md](../STATUS.md).

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

Authenticated with **OIDC sessions**, never an API key. Called server-side from
Next.js only.

Keeping these separate is deliberate: `sk_live_` keys are bearer credentials
with full payment authority and no expiry or revocation story. Sessions have
both. See [ADR-0008](../adr/0008-dashboard-scope.md).

**Status: not implemented.**

## `/provider/{code}/callback` — rail callbacks

Public and unauthenticated by necessity. A callback **never changes state** — it
only enqueues a status query. See [../flows/reconciler.md](../flows/reconciler.md).

**Status: not implemented.**
