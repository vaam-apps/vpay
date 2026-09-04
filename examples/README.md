# Examples

Runnable merchant-side integrations.

| Example | What it shows |
|---|---|
| [merchant-demo](merchant-demo/) | **The one the demo runbook runs.** `just demo` boots the stack and drives it: discovery, JWKS, a real token, the 401 boundary, then **six payments on both rails** — succeeded, declined and expired on MTN; succeeded, expired and refused on Orange — each settled by the worker and each webhook signature-verified ([docs/runbooks/demo.md](../docs/runbooks/demo.md)) |
| [merchant-curl](merchant-curl/) | The raw HTTP shape: the `client_credentials` + `private_key_jwt` handshake, form-encoded bodies, idempotency keys, both flow shapes |
| [merchant-node](merchant-node/) | The same flow through vpay's own Node SDK, [`@vpay/sdk`](../sdks/nodejs/), which performs the `private_key_jwt` handshake ([ADR-0010](../docs/adr/0010-merchant-auth-private-key-jwt.md)) |
| [merchant-stripe-node](merchant-stripe-node/) | **The second one that runs against a real vpay.** The same payment through the *official* `stripe` package, authenticated by `@vpay/sdk/stripe` — create, confirm, poll to `succeeded` ([stripe-sdk-compat.md](../docs/flows/stripe-sdk-compat.md)) |
| [`sdks/rust/examples`](../sdks/rust/examples/) | The same flow through the Rust SDK, `vpay-sdk` |
| [webhook-receiver](webhook-receiver/) | Verifying the `Vpay-Signature` header correctly |
| [checkout-browser](checkout-browser/) | **The third one that runs against a real vpay, and the only PAYER-facing one.** Plain HTML + JS on [`@vpay/stripe-js`](../sdks/stripe-js/): confirms an MTN MoMo push and polls to `succeeded`, with no merchant credential ever in the browser ([browser-checkout.md](../docs/flows/browser-checkout.md)) |
| [shop](shop/) | **A merchant SITE, not a script** (Step 9's D11): Next.js + tRPC + ZenStack over Prisma, a five-product catalogue priced in XAF, a cart, and a checkout that creates the PaymentIntent and a hosted Checkout Session **server-side** through [`@vpay/sdk`](../sdks/nodejs/) and redirects the payer to vpay's page — or frames it with `initEmbeddedCheckout`. Its order page says *paid* only once its `/api/vpay/webhook` endpoint has verified a signature and written the row. **Never yet run against a real vpay**: `/v1/checkout/sessions` is built in the same step and its unit tests speak to a local server shaped like the wire contract ([shop/README.md](shop/README.md)) |

**Status, corrected 2026-09-03.** This section used to say "**no `/v1`
business resource exists**", which was true when it was written on
2026-09-02 and false the next day. `/v1/payment_intents` — create, retrieve,
list, confirm, cancel — is served, and a confirm reaches a rail.

`merchant-demo`, `merchant-stripe-node` and `checkout-browser` are the three
examples written against what vpay actually serves, and all three are run
end to end against the compose stack — the first two by a human or CI running
them directly, `checkout-browser` additionally by
[`frontends/tests/e2e/cypress/e2e/checkout.cy.ts`](../frontends/tests/e2e/cypress/e2e/checkout.cy.ts).
`merchant-curl` and `merchant-node` still describe the *intended* API as
pinned down by [`docs/flows/merchant-auth.md`](../docs/flows/merchant-auth.md),
and their refund, event and balance calls still reach a `404 unknown_route`,
because those routes are deliberately not mounted — `/v1/events`, though, **is**
served as of Step 5. `webhook-receiver` describes a delivery vpay now really
sends: the worker signs and POSTs it, and a delivered one has been verified
both with `@vpay/sdk` and with the official `stripe` package's
`constructEvent`. An intent reaches `succeeded` too — `vpay-worker` polls the
rail and settles the charge — but every rail and every receiver involved so
far has been a WireMock host on a compose network, and no money has moved. See
[../docs/status.md](../docs/status.md).
