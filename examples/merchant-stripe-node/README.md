# merchant-stripe-node

One payment, driven by the **official `stripe` package** against a real vpay.

vpay accepts no API key ([ADR-0010](../../docs/adr/0010-merchant-auth-private-key-jwt.md)),
so the client is built with an empty key and a `config.authenticator` that
performs vpay's `client_credentials` + `private_key_jwt` handshake:

```js
const stripe = new Stripe("", {
  authenticator: createStripeAuthenticator({
    baseUrl: "http://localhost:18080",
    clientId: "demo-merchant",
    privateKey: readFileSync("./merchant-key.pem", "utf8"),
  }),
  host: "localhost",
  port: "18080",
  protocol: "http",
});
```

Everything after those lines is ordinary stripe-node.

## Run it

Ten steps, from a clean checkout. `18080` is only "a port that is probably
free"; use `8080` if it is free on your machine, and keep the number the same
everywhere — the OP's issuer is derived from it.

1. `pnpm install`
2. `pnpm --filter @vaam-apps/vpay-sdk build` — the example imports
   `@vaam-apps/vpay-sdk/stripe`, which resolves to `sdks/nodejs/dist/`.
3. `just demo_port=18080 gen-demo-keys` — writes a throwaway merchant keypair
   into git-ignored `.e2e/` and the profile overlay that registers its public
   half.
4. `docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml up -d --build postgres wiremock-mtn wiremock-orange vpay-server vpay-worker`
   (with `VPAY_DEMO_PORT=18080` exported).
5. `curl -fsS http://localhost:18080/healthz` until it answers — the server's
   image is `FROM scratch` and has no container healthcheck.
6. `VPAY_BASE_URL=http://localhost:18080 pnpm --filter @vpay-examples/merchant-stripe-node start`
7. Read the four numbered lines it prints: authenticated, created, confirmed,
   settled.
8. Expect it to end on **`succeeded`**, usually a second or two after the
   confirm — see below.
9. `docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml logs vpay-server vpay-worker`
   if any step failed. `vpay-worker` is the one to read if step 4 timed out.
10. `just demo-down` — containers and volumes.

Steps 3–5 are what `just demo_port=18080 stripe-compat` does for the
conformance suite; run that first and this example can start at step 6.

| Variable | Default |
|---|---|
| `VPAY_BASE_URL` | `http://localhost:18080` |
| `VPAY_MERCHANT_CLIENT_ID` | `demo-merchant` |
| `VPAY_MERCHANT_PRIVATE_KEY_PATH` | `.e2e/demo-merchant/oauth-signing-key.pem` |

## How it reaches `succeeded`, and what this file does *not* do

`processing` is a push rail's one success state at confirm time: the rail has
the request and the payer would now approve it on their handset. What moves it
on is the **`vpay-worker` container** (step 4 of the run instructions brings it
up, and it is not optional). It claims the `poll_charge` job the confirm
committed in the charge's own transaction — enqueued with `run_at = now()`, so
the first poll is immediate — asks the MTN WireMock host over HTTP whether the
payer approved, and settles the charge in one transaction when the rail says
yes. `vpay_worker::poll_delay`'s ladder (10 s, 20 s, 30 s, …) is what governs
the *re*-polls after a `PENDING`.

This example only *watches*, through `paymentIntents.retrieve`. That is
deliberate: it is the only thing a merchant integration can see, and a program
that read the database behind the API it exists to demonstrate would be proving
something else. It fails, rather than hanging, if the window closes with the
intent still `processing` — the usual cause being that `vpay-worker` is not
running.

This file used to assert the opposite. Before the worker existed it polled for
ten seconds and **failed if it ever saw `succeeded`**, because on that branch a
`succeeded` could only have been fabricated. Flipping it was the first thing
Step 4 made possible, exactly as [`merchant-demo`](../merchant-demo/)'s step 5
was flipped when the rail adapters landed.

The rail is a WireMock **host** reached over HTTP, the same mechanism a real
rail is reached by ([ADR-0006](../../docs/adr/0006-no-mocks-in-main-processes.md)).
**MTN has never been called by this code**, and no money has moved: the
approval is a stub mapping answering `SUCCESSFUL`.

## What this example is not

- Not a conformance suite. [`sdks/stripe-compat`](../../sdks/stripe-compat/)
  is that, and it asserts things this file only demonstrates.
- Not a demonstration of everything that carries over from Stripe. The
  divergences — no `automatic_payment_methods`, no `confirm: true` on create,
  no `expand`, no Connect, no `client_secret` — are listed in
  [`docs/flows/stripe-sdk-compat.md`](../../docs/flows/stripe-sdk-compat.md).
- Not a reason to prefer stripe-node over [`@vaam-apps/vpay-sdk`](../../sdks/nodejs/).
  Use this if you already have Stripe integration code; use `@vaam-apps/vpay-sdk` if you
  do not.
