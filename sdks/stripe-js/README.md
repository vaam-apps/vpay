# `@vpay/stripe-js`

A Stripe.js-shaped browser client for vpay's payer surface. Zero runtime
dependencies, ESM, TypeScript strict.

## Why this exists rather than `@stripe/stripe-js`

`@stripe/stripe-js` cannot be pointed at another API. Its
`StripeConstructorOptions` has exactly five members — `stripeAccount`,
`apiVersion`, `locale`, `betas`, `developerTools` — and none of them is a host
or base URL. The loader hardcodes `ORIGIN = 'https://js.stripe.com'`, and its
`isStripeJSURL` accepts only `js.stripe.com/v3` or
`js.stripe.com/{v3|[a-z]+}/stripe.js`. Elements iframes and the bundle's own
XHRs are Stripe-origin by construction.

So this is a drop-in-_shaped_ package of vpay's own, speaking vpay's
`/v1/browser` routes. `@stripe/stripe-js` is a devDependency here and only a
devDependency: it is used by `src/compat.test.ts` to pin, at compile time, in
which direction the two packages' types are actually assignable. It is never
loaded at runtime.

## The drop-in claim, scoped honestly

**Compatible**: the payment-intent half of Stripe.js against a
push (mobile-money) or redirect rail.

| Stripe.js                                                          | `@vpay/stripe-js`                                                                      |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------- |
| `loadStripe(pk)`                                                   | `loadStripe(pk, { baseUrl })` — plus a required `baseUrl`; no `<script>` is downloaded |
| `stripe.retrievePaymentIntent(clientSecret)`                       | same signature, same `PaymentIntentResult` shape                                       |
| `stripe.confirmPayment({ clientSecret, confirmParams, redirect })` | same, minus `elements`                                                                 |
| `stripe.handleNextAction({ clientSecret })`                        | same                                                                                   |
| —                                                                  | `stripe.confirmMobileMoneyPayment(clientSecret, { type, msisdn })`                     |
| —                                                                  | `stripe.waitForPaymentIntent(clientSecret, { timeoutMs, intervalMs })`                 |

**Not compatible, ever** — absent by construction, not stubbed, because each
depends on card data, an iframe served from `js.stripe.com`, or a
Stripe-hosted page:

- Elements
- cards
- 3DS
- Payment Element
- Checkout (hosted or embedded)
- Link
- Payment Request / Apple Pay / Google Pay
- `confirmCardPayment`
- `createPaymentMethod`
- ConfirmationTokens
- SetupIntents

A method that is missing is a compile error on the merchant's page. A method
that returned a plausible-looking failure would be a surprise at the till.

### Type compatibility, precisely

Every claim below is a compile-time assertion in `src/compat.test.ts`; the
package's `typecheck` fails if either side moves.

- Stripe's `StripeError` **is** assignable to ours — ours is a widening, so
  existing error-rendering code keeps compiling.
- Ours is **not** assignable to Stripe's: our `type` is an open `string` (so a
  `type` a later vpay version introduces is not a compile error at the
  merchant), and our optional members are written `?: T | undefined`, which
  `exactOptionalPropertyTypes` distinguishes from Stripe's `?: T`.
- Our `PaymentIntent` is **not** assignable to Stripe's `PaymentIntent`.
  Stripe's requires a dozen fields that exist only because it settles cards
  (`capture_method`, `confirmation_method`, `canceled_at`, `payment_method`,
  …), and its `last_payment_error` is a card-decline shape.
- The two fields a checkout page actually branches on — `status` and
  `next_action` — **are** assignable to Stripe's, and `PaymentIntentResult`
  has the same two-member discriminated shape in both packages, so the
  narrowing idiom is portable:

```ts
import type { PaymentIntentResult as StripeResult } from "@stripe/stripe-js";
import type { PaymentIntentResult as VpayResult } from "@vpay/stripe-js";

// The same function body compiles against either alias.
function render(result: VpayResult): string {
  if (result.error) return result.error.message ?? "Payment failed";
  return result.paymentIntent.status;
}
```

## Using it

The integration is Stripe's own: the **server** creates the PaymentIntent with
the merchant SDK and renders the publishable key and the `client_secret` into
the page; the **browser** never holds a merchant API key.

```ts
// server (Node, @vpay/sdk, OAuth2 private_key_jwt)
const intent = await vpay.paymentIntents.create({
  amount: 5000,
  currency: "xaf",
  payment_method_types: ["mtn_momo"],
});
res.render("checkout", {
  publishableKey: process.env.VPAY_PUBLISHABLE_KEY,
  clientSecret: intent.client_secret,
});
```

```ts
// browser
import { loadStripe } from "@vpay/stripe-js";

const stripe = await loadStripe(publishableKey, {
  baseUrl: "https://api.vpay.example",
});

const confirmed = await stripe.confirmMobileMoneyPayment(clientSecret, {
  type: "mtn_momo",
  msisdn: "237690000000",
});
if (confirmed.error) {
  show(confirmed.error.message);
} else {
  // The payer now approves the push on their handset. Poll until the intent
  // stops moving — three minutes by default, every two seconds, jittered.
  const settled = await stripe.waitForPaymentIntent(clientSecret);
  show(settled.error ? settled.error.message : settled.paymentIntent.status);
}
```

### Redirect rails

`confirmPayment` follows Stripe.js's rule. When the rail answers with
`next_action.redirect_to_url` and `redirect` is not `'if_required'`, the
browser is navigated with `window.location.assign(url)` and **the returned
promise never settles** — so a caller's `.then`/`.finally` cannot paint a
"payment failed" state during the unload. Pass `redirect: 'if_required'` to
get the intent back and render the hand-off yourself. With no `window` (Node,
SSR, a worker) the result is
`{ error: { type: 'api_error', code: 'redirect_unavailable' } }` rather than an
invented success.

**vpay appends nothing to your `return_url`** (decision D3). Stripe's
`payment_intent`, `payment_intent_client_secret` and `redirect_status` query
parameters are **absent**. A page handling the return trip must carry its own
state — put the `client_secret` in the `return_url` you supply, or key on your
own order id — and then call `retrievePaymentIntent` to learn the outcome.

**Step 5c ships push-only** (decision D4). `confirmPayment` returns the rail's
real `redirect_to_url` and will navigate to it, but the _return_ trip is not
wired: vpay has no `/provider/{code}/callback` route, so a redirect rail sends
the payer to a URL that does not exist yet. Do not ship a redirect-rail
checkout on this package until that route lands. `next_action.redirect_to_url.return_url`
is echoed back as a label; nothing redirects to it.

## Errors

Nothing on the `Stripe` object ever rejects. Every failure is
`{ error: { type, code?, message?, param? } }` — vpay's server envelope
passed through 1:1, or one of the three codes this package originates.

| `type`                  | `code`                 | Origin                                                                                                                                                             |
| ----------------------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `invalid_request_error` | `resource_missing`     | The uniform 404. **Every** credential failure renders it byte-identically: unknown publishable key, wrong `client_secret`, another merchant's key, unknown intent. |
| `invalid_request_error` | `invalid_state`        | The intent has already been confirmed. One charge per intent, forever — retry means a new PaymentIntent.                                                           |
| `invalid_request_error` | `invalid_request`      | A parameter this package refused before sending: a malformed `clientSecret` (`param: 'clientSecret'`), an unencodable `payment_method_data`.                       |
| `api_error`             | `provider_unavailable` | The rail. Retryable.                                                                                                                                               |
| `api_error`             | `polling_timeout`      | **Client.** `waitForPaymentIntent` ran out of budget.                                                                                                              |
| `api_error`             | `redirect_unavailable` | **Client.** A redirect was required and there is no `window`.                                                                                                      |
| `api_error`             | `unexpected_response`  | **Client.** Not vpay's envelope — a proxy's HTML 502, say. Carries the status, never the body.                                                                     |
| `api_connection_error`  | _(none)_               | **Client.** The request never landed. The absence of a code is the signal: vpay's server never sends this type.                                                    |

No message this package builds contains a `client_secret` or a publishable
key, and there is no `console` call in the shipping source — both are pinned
by tests, because the request URL carries both credentials in its query
string and a thrown `fetch` error's `cause` can carry the URL.

`loadStripe` is the one function here that _does_ reject: a blank publishable
key or base URL is an integration mistake visible on the merchant's first page
load, and checking it once there is what lets every other method keep the
never-rejects contract.

## Development

```bash
pnpm --filter @vpay/stripe-js typecheck
pnpm --filter @vpay/stripe-js test
pnpm --filter @vpay/stripe-js build   # tsc → dist/, gitignored
```

Unit tests run against a real `node:http` server standing in for
`/v1/browser` (`src/testing/browser-stub.ts`), not a patched `fetch`: the
contract under test is bytes on the wire. That stub is excluded from `dist`
and imported only from `*.test.ts`, so it is not a test double reachable from
a shipping process in ADR-0006's sense.

## Status

**Verified end to end on the compose stack, not yet in CI.**

- The package itself: implemented, typechecked and unit-tested (87 tests)
  against a `node:http` stub of the surface in
  `docs/plans/2026-09-03-step5c-stripejs.md` §1.
- The server routes it speaks to — `GET /v1/browser/payment_intents/{id}` and
  `POST /v1/browser/payment_intents/{id}/confirm`, publishable keys, the
  `client_secret` column and the CORS layer — landed in the same step (block
  A) and are proven by `backends/tests/integration/tests/browser_checkout.rs`
  against real Postgres and WireMock.
- The end-to-end proof through this package is
  `frontends/tests/e2e/cypress/e2e/checkout.cy.ts`, which drives
  `examples/checkout-browser` to `succeeded` on the compose stack. It has
  passed locally; it is wired into the CI `e2e` job but has not yet been
  observed green in a GitHub Actions run.
- The redirect _return_ trip is a named gap (D4), not an oversight. See
  `docs/status.md` for the repository-wide picture.
