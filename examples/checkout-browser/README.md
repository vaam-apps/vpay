# `examples/checkout-browser`

A plain HTML + JavaScript payer page built on
[`@vaam-apps/vpay-stripe-js`](../../sdks/stripe-js/) — no framework, no bundler for
this page itself. It reads a publishable key and a payment intent's
`client_secret` from its own URL, confirms an MTN MoMo push, and polls until
the intent settles, exactly the way `sdks/stripe-js/README.md`'s "Using it"
section describes.

This is the server/browser split Stripe itself uses: a merchant's **server**
creates the PaymentIntent (with a merchant OAuth credential vpay never lets a
browser hold) and renders `pk` + `client_secret` into the page; the
**browser** never sees anything else. `mint.mjs` in this directory plays the
"merchant server" role for the steps below.

## Run it against the demo stack (7 steps)

1. `just demo` — boots the compose stack (server, worker, Postgres, both rail
   stubs) and registers the `demo-merchant` keypair `mint.mjs` needs. Leave it
   running; `just demo` already runs `examples/merchant-demo`'s own
   walkthrough (four steps, the last of which is six payments on both rails)
   and prints the URLs it used — this example is separate from that.
2. `just build-checkout-browser` — builds `@vaam-apps/vpay-stripe-js` and vendors its
   `dist/` into `dist/stripe-js/` here (gitignored, rebuilt on every run).
3. `pnpm install` (once, if you have not already) so this directory's own
   `@vaam-apps/vpay-sdk` dependency is linked.
4. `node examples/checkout-browser/mint.mjs` — mints a 50.00 EUR `mtn_momo`
   PaymentIntent through `demo-merchant`'s credential and prints a ready-to-open
   URL, `http://localhost:4180/?pk=...&client_secret=...&api=http://localhost:8080`.
5. `node examples/checkout-browser/serve.mjs` (a second terminal) — serves
   this directory on `:4180`.
6. Open the URL step 4 printed. It renders the intent's `requires_payment_method`
   status and an MSISDN form.
7. Enter `237600000ce0` (the MTN sandbox number `examples/merchant-demo`
   documents — it selects the WireMock scenario `mtn-e2e-poll`, which answers
   `PENDING` once and `SUCCESSFUL` on the next poll) and submit. The page
   confirms, shows "waiting…", and — once `vpay-worker` settles the charge —
   renders `succeeded`.

Tear down with `just demo-down`.

If `8080`/`18080`–`18083`/`3000` are already taken, run `just
demo_port=18084 demo` instead and pass `&api=http://localhost:18084` on
step 4's URL (`mint.mjs` reads `VPAY_BASE_URL`, so
`VPAY_BASE_URL=http://localhost:18084 node mint.mjs` sets both at once).

## What is and is not proven here

- `frontends/tests/e2e/cypress/e2e/checkout.cy.ts` drives exactly this page
  (served by this directory's own `serve.mjs`, imported rather than
  reimplemented) against the real compose stack. It passes locally and is
  wired into the CI `e2e` job; it has not yet been observed green in a GitHub
  Actions run — see `docs/flows/browser-checkout.md`.
- This page ships **push-only** (decision D4,
  `docs/plans/2026-09-03-step5c-stripejs.md`). `confirmPayment`/redirect
  rails are not exercised here; `@vaam-apps/vpay-stripe-js`'s README covers what would
  happen and what is missing (the Orange return trip has no route yet).
- Nothing here has taken real money. Both rails are WireMock stubs on the
  compose network, same as everywhere else in this repository —
  `docs/status.md`.

## Why `mint.mjs` is plain JavaScript, not TypeScript

For the same reason `examples/merchant-node` is: this is a standalone
example with no build step, so a `.mjs` script is simpler than adding one
just to type-check a few dozen lines. It is not working around a typing
gap — `@vaam-apps/vpay-sdk`'s `PaymentIntent` type (`sdks/nodejs/src/types.ts`) now
declares `client_secret?: string` (`c40a137`, 2026-09-03), matching the
server's `create`/`retrieve` responses since migration `0026`, so
`intent.client_secret` is typed for a TypeScript caller too — see the
comment at the top of `mint.mjs`.

## Files

| File              | What it is                                                                                                                                                  |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `index.html`      | The page: an error region, a status summary (`dl#summary`, `data-status` on `dd#status`), an MSISDN form.                                                   |
| `checkout.js`     | All the logic — `loadStripe`, `retrievePaymentIntent`, `confirmMobileMoneyPayment`, `waitForPaymentIntent`. Plain ESM, imports `./dist/stripe-js/index.js`. |
| `serve.mjs`       | Zero-dependency static file server for this directory (see its own header comment for why not the dashboard container or an npm package).                   |
| `mint.mjs`        | The "merchant server" half — mints an intent with `@vaam-apps/vpay-sdk` and prints a checkout URL.                                                          |
| `dist/stripe-js/` | **Generated, gitignored.** `just build-checkout-browser`'s output; a copy of `sdks/stripe-js/dist/`.                                                        |
