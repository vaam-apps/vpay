# @vaam-apps/vpay-stripe-compat

The **official `stripe` package**, driven against a **real vpay stack**,
asserting that a merchant's existing Stripe integration works once
[`createStripeAuthenticator`](../nodejs/) is plugged into
`config.authenticator`.

This is a conformance suite, not a library. It ships nothing and is
`private: true`.

## Running it

```bash
just demo_port=18080 stripe-compat
```

That recipe generates the demo merchant's throwaway key pair and profile
overlay, brings up Postgres + both WireMock rails + the WireMock webhook
receiver + `vpay-server` + `vpay-worker`, waits for `/healthz`, and runs the
suite. Tear the stack down with `just demo-down`.

`vpay-worker` and the receiver are not optional: the worker is what drives a
confirmed intent to `succeeded`, and the receiver is what records the delivery
`src/webhooks.compat.test.ts` hands to the real `stripe` package.

Against an already-running stack:

```bash
pnpm --filter @vaam-apps/vpay-sdk build     # the suite imports @vaam-apps/vpay-sdk/stripe
VPAY_BASE_URL=http://localhost:18080 \
VPAY_RECEIVER_URL=http://localhost:8083 \
VPAY_MERCHANT_CLIENT_ID=demo-merchant \
VPAY_MERCHANT_PRIVATE_KEY_PATH="$PWD/.e2e/demo-merchant/oauth-signing-key.pem" \
  pnpm --filter @vaam-apps/vpay-stripe-compat compat
```

| Variable | Default |
|---|---|
| `VPAY_BASE_URL` | `http://localhost:18080` |
| `VPAY_MERCHANT_CLIENT_ID` | *required* |
| `VPAY_MERCHANT_PRIVATE_KEY_PATH` | *required* — an absolute path, or one relative to the cwd `pnpm` uses (this package) |
| `VPAY_RECEIVER_URL` | `http://localhost:8083` — the WireMock webhook receiver, read for its request journal |
| `MERCHANT_WEBHOOK_SECRET` | the placeholder `compose.e2e.yml` gives both binaries; a stub value for a stub receiver on a `livemode: false` stack |

## It cannot skip

`src/preflight.ts` is a vitest `globalSetup` that fails the run — never skips
it — when no vpay answers `/healthz`, or when the configured `client_id` and
key are not the pair the stack registered. A conformance suite that skips
itself when the stack is down reports `ok` with zero cases and is
indistinguishable, in a CI summary, from one that passed
([AGENTS.md](../../AGENTS.md)).

The run script is `compat`, not `test`, for the same reason `@vpay/e2e`'s is
`e2e`: it must not be picked up by the `web` job's `pnpm -r test`, which has
no stack. CI runs it in the `e2e (compose)` job, after Cypress.

## Why out of process

Half of what it proves is header behaviour through the real router — the
`request-id` mirror stripe-node reads, `stripe-should-retry`, the two
responses that never reach the error renderer. An in-process harness cannot
see any of that, and a mock in a shipping path is forbidden anyway
([ADR-0006](../../docs/adr/0006-no-mocks-in-main-processes.md)).

## Layout

| File | What it covers |
|---|---|
| `src/env.ts` | Configuration, with no defaults for the two credential-shaped variables |
| `src/preflight.ts` | The skip-proof gate described above |
| `src/client.ts` | The one place a `Stripe` client is built, and the **only** place a cast appears — every cast is a real thing a TypeScript merchant has to write, so the test files stay cast-free |
| `src/lifecycle.compat.test.ts` | create, retrieve, cursor paging, `autoPagingToArray`, cancel, confirm → `processing`, `expand` accepted and ignored through stripe-node's indexed array encoding, and a bounded poll to `succeeded` once the worker has asked the rail |
| `src/webhooks.compat.test.ts` | A delivery the WireMock receiver actually recorded, verified with `stripe.webhooks.constructEvent` — and refused for a tampered body and for a wrong secret |
| `src/errors.compat.test.ts` | The status → error-class mapping, `err.param`, `err.requestId`, the 409 and its retry advisory, the 405/413 collapse, and the parameters that move money elsewhere (`capture_method: "manual"`, `transfer_data`, `application_fee_amount` on confirm) being refused rather than ignored |
| `src/idempotency.compat.test.ts` | stripe-node's auto-generated key, replay, and a reused key with a changed body |
| `src/headers.compat.test.ts` | The `request-id`/`x-request-id` mirror, no `apiVersion`, `Stripe-Version`/`Stripe-Account` accepted and ignored |

## What it does not prove

Listed in full, with reasons, under **Status** in
[`docs/flows/stripe-sdk-compat.md`](../../docs/flows/stripe-sdk-compat.md).
The short version: the rail and the webhook receiver are both WireMock hosts —
MTN has never been called, no merchant endpoint has ever been POSTed to, and
no money has moved; the `stripe-should-retry: true` direction is not stageable
from outside without a test double; `stripe.events.list()` against the routed
`/v1/events` is untested; and nothing pins vpay against a future `stripe`
release.
