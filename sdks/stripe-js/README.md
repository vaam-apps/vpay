# `@vaam-apps/vpay-stripe-js`

A Stripe.js-shaped browser client for vpay's payer surface. Zero runtime
dependencies, ESM, TypeScript strict.

**Not yet on the registry.** `npm view @vaam-apps/vpay-stripe-js` answered
`E404` on 2026-09-05 and no workflow in this repository publishes anything;
the manifest stopped saying `"private": true` on that date so that a release
can happen without editing it. Once published:

```bash
pnpm add @vaam-apps/vpay-stripe-js   # not yet published — see above
```

Inside this workspace it is a `workspace:*` dependency, built with
`pnpm --filter @vaam-apps/vpay-stripe-js build`.

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

| Stripe.js                                                          | `@vaam-apps/vpay-stripe-js`                                                                                                          |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| `loadStripe(pk)`                                                   | `loadStripe(pk, { baseUrl, checkoutBaseUrl? })` — plus a required `baseUrl`; no `<script>` is downloaded                   |
| `stripe.retrievePaymentIntent(clientSecret)`                       | same signature, same `PaymentIntentResult` shape                                                                           |
| `stripe.confirmPayment({ clientSecret, confirmParams, redirect })` | same, minus `elements`                                                                                                     |
| `stripe.handleNextAction({ clientSecret })`                        | same                                                                                                                       |
| —                                                                  | `stripe.confirmMobileMoneyPayment(clientSecret, { type, msisdn })`                                                         |
| —                                                                  | `stripe.waitForPaymentIntent(clientSecret, { timeoutMs, intervalMs })`                                                     |
| `stripe.createEmbeddedCheckoutPage({ fetchClientSecret })`         | `stripe.initEmbeddedCheckout({ fetchClientSecret, onComplete })` — **vpay's own page**, not Stripe's; see "Checkout" below |
| —                                                                  | `stripe.retrieveCheckoutSession(clientSecret)`                                                                             |
| —                                                                  | `stripe.openCheckoutPopup({ fetchCheckoutUrl, onComplete, onCancel })` — vpay's hosted page in a window your page owns   |
| —                                                                  | `notifyCheckoutOpener({ session, status })` — the popup half, called on your own `success_url`                            |

**Not compatible, ever** — absent by construction, not stubbed, because each
depends on card data or on an iframe served from `js.stripe.com`:

- Elements
- cards
- 3DS
- Payment Element
- Link
- Payment Request / Apple Pay / Google Pay
- `confirmCardPayment`
- `createPaymentMethod`
- ConfirmationTokens
- SetupIntents

A method that is missing is a compile error on the merchant's page. A method
that returned a plausible-looking failure would be a surprise at the till.

**Checkout used to be on that list and is not any more.** Until Step 9 this
README said "Checkout (hosted or embedded)" was not compatible, ever. That
sentence was true when it was written and is retracted here, in the same PR
that makes it false — but the replacement is narrower than it looks, and the
distinction matters:

- vpay now serves **its own** checkout page, hosted and embedded
  (`frontends/apps/checkout`, Step 9). `stripe.initEmbeddedCheckout` frames
  that page, and `stripe.retrieveCheckoutSession` reads vpay's own
  `checkout.session` object.
- It is **not** `@stripe/stripe-js`'s Checkout. Stripe's own method is called
  `createEmbeddedCheckoutPage` in the version pinned here, its options are
  not ours, and its session object has `line_items`, `mode` and
  `amount_total`, none of which vpay's has (Step 9's D10). A merchant's
  Stripe Checkout integration does **not** port over by changing an import.
- What _is_ portable is the handle: our `{ mount, unmount, destroy }` is
  assignable to Stripe's `StripeEmbeddedCheckout` in both directions, pinned
  in `src/compat.test.ts`. The mounting plumbing moves; the session model
  does not.

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
- Our embedded-checkout **handle** is assignable to Stripe's
  `StripeEmbeddedCheckout`, and Stripe's to ours: both are exactly
  `mount(string | HTMLElement)`, `unmount()`, `destroy()`.
- Our embedded-checkout **options** are not assignable to Stripe's in either
  direction. Ours requires `fetchClientSecret` where Stripe's makes it
  optional (Stripe also accepts a bare `clientSecret`; vpay does not), and
  our `onComplete` receives the completing session where Stripe's takes no
  argument.
- The two fields a checkout page actually branches on — `status` and
  `next_action` — **are** assignable to Stripe's, and `PaymentIntentResult`
  has the same two-member discriminated shape in both packages, so the
  narrowing idiom is portable:

```ts
import type { PaymentIntentResult as StripeResult } from "@stripe/stripe-js";
import type { PaymentIntentResult as VpayResult } from "@vaam-apps/vpay-stripe-js";

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
// server (Node, @vaam-apps/vpay-sdk, OAuth2 private_key_jwt)
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
import { loadStripe } from "@vaam-apps/vpay-stripe-js";

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

## Checkout (vpay's own)

A Checkout Session (`cs_…`, Step 9's D1) is created by the **merchant's
server** against an existing PaymentIntent, and carries where the payer goes
afterwards. It is a second credential of the same shape as the intent's:
`cs_…_secret_…` authorises reading one session, and nothing else — in
particular it is not the secret that authorises `confirm`.

```ts
// server (Node, @vaam-apps/vpay-sdk)
const session = await vpay.checkout.sessions.create({
  payment_intent: intent.id,
  ui_mode: "embedded",
  return_url: "https://shop.example/order/42",
});
// hand session.client_secret to the browser through your own route
```

```ts
// browser
const stripe = await loadStripe(publishableKey, {
  baseUrl: "https://api.vpay.example",
  checkoutBaseUrl: "https://checkout.vpay.example", // required for embedded
});

const checkout = await stripe.initEmbeddedCheckout({
  fetchClientSecret: async () =>
    (
      await fetch("/create-checkout-session", { method: "POST" }).then((r) =>
        r.json(),
      )
    ).client_secret,
  onComplete: ({ session, status }) => {
    // A message from an iframe, not proof of payment. Re-read the session
    // from your own server before you ship anything.
    checkout.unmount();
    show(session, status);
  },
});
checkout.mount("#checkout");
```

`checkoutBaseUrl` is your deployment's `checkout.public_base_url`. It is
**required** for `initEmbeddedCheckout`, which rejects without it, and it is
also the origin every incoming `message` is pinned against — a wrong value
does not fail loudly, it silently accepts nothing.

### The frame, and what crosses it

The iframe's `src` is
`{checkoutBaseUrl}/e/{cs_id}?key={pk}#{client_secret}`. The publishable key
is in the query string because vpay's page needs it server-side, before any
script runs, to derive the `frame-ancestors` for your origin; the session
secret is in the **fragment**, which no browser ever sends to a server
(decision D6).

Three messages come back from vpay's page, and each is acted on only when
`event.origin` equals `new URL(checkoutBaseUrl).origin` **and**
`event.source` is this frame:

| Message                                      | Effect                                                                                                               |
| -------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `{ type: 'vpay:resize', height }`            | Sets the frame's height. The page owns its own height; until the first one arrives the frame is `0px` tall.          |
| `{ type: 'vpay:complete', session, status }` | Calls your `onComplete`. Both members must be strings or the message is ignored.                                     |
| `{ type: 'vpay:redirect', url }`             | `window.top.location.assign(url)` — the redirect rail's hand-off. Refused unless `url` is an absolute `http(s)` URL. |

This package **posts nothing into the frame**, so there is no target origin
to get wrong (no `postMessage(…, '*')`, which is the mistake the protocol
was designed to preclude). The frame is sandboxed
`allow-scripts allow-same-origin allow-forms` — notably _without_
`allow-top-navigation`, which is why the redirect rail's hand-off is a
message your page acts on rather than something the frame does itself.

### The popup, and why its message comes from *your* page

`openCheckoutPopup` is the third surface: vpay's **hosted** page in a
top-level window your page opened, rather than in an iframe or in the
payer's own tab.

```ts
// browser, inside a click handler
try {
  const popup = await stripe.openCheckoutPopup({
    // Your own server's `POST /v1/checkout/sessions` with ui_mode: 'hosted'.
    fetchCheckoutUrl: async () =>
      (await fetch("/checkout-url", { method: "POST" }).then((r) => r.json()))
        .url,
    onComplete: ({ session, status }) => {
      // A message, not proof of payment: re-read the order from your own
      // server, which your webhook is what writes.
      refresh(session, status);
    },
    onCancel: () => show("The payment window was closed."),
  });
} catch (err) {
  if (err instanceof CheckoutPopupBlockedError) {
    window.location.assign(await checkoutUrl()); // no browser blocks this
  }
}
```

```ts
// browser, on the success_url page — which loads *inside* the popup
import { notifyCheckoutOpener } from "@vaam-apps/vpay-stripe-js";

notifyCheckoutOpener({ session: sessionIdFromTheUrl, status: "complete" });
```

Three things about this differ from the embedded surface, and each of them
is a consequence of a popup not being a frame:

1. **The completion message is sent by your return page, not by vpay's.**
   Inside a popup `window.parent === window`, so vpay's checkout page has no
   framer to post to and deliberately says nothing. What closes the loop is
   `success_url` — your page, running in the popup, calling
   `notifyCheckoutOpener`, which posts `{type:'vpay:complete', session,
   status}` to `window.opener` and closes the window.
2. **`completionOrigin` therefore defaults to your own origin**, not to
   `checkoutBaseUrl`. Pinning vpay's checkout origin here would accept
   nothing at all. `checkoutBaseUrl` is not consulted by this method.
3. **`notifyCheckoutOpener` answers `false` and does nothing when there is
   no opener**, which is exactly what happens when the same page is reached
   by an ordinary redirect. One return page serves both integrations
   without branching on a query parameter.

The window is opened **before** `fetchCheckoutUrl` is awaited, because
`window.open` succeeds only inside the user gesture that triggered it;
awaiting first is the usual way a popup integration gets blocked. A window
the browser refused is a `CheckoutPopupBlockedError` — the one failure here
that a correct integration can still hit, and the one you are expected to
fall back from. `location=yes` is in the window features on purpose: a
payment window that hides the address bar is a phishing lesson.

`onCancel` is opt-in, and passing it is what starts a `closed` poll — a
closed window fires no event at its opener in any browser. It fires when the
payer dismissed the window without completing; it is **not** a cancellation
of the charge, which is untouched and may still settle.

`retrieveCheckoutSession(clientSecret)` reads
`GET /v1/browser/checkout/sessions/{id}` and, like everything else on the
`Stripe` object, never rejects.

On that route `payment_intent` is the **expanded intent**, not the `pi_…`
id, `client_secret` included — vpay's checkout page has to confirm and poll
it through the existing browser routes, and a second round trip to fetch it
would need a credential the page does not have yet. On the merchant SDKs
(`@vaam-apps/vpay-sdk`, `vpay_sdk`) the same field stays the id. So:

```ts
const { checkoutSession, error } = await stripe.retrieveCheckoutSession(cs);
checkoutSession?.status; // 'open' | 'complete' | 'expired'
checkoutSession?.payment_intent.status; // the whole intent, not an id
```

**This hands your page a live _confirm_ credential**
(`checkoutSession.payment_intent.client_secret`), which is a wider exposure
than the session's own secret. If all you want is whether the payer paid,
read `status` and `payment_status` and leave the intent alone. The route
also carries the merchant's display name for vpay's own page; this package
does not model it.

## Errors

Nothing on the `Stripe` object ever rejects. Every failure is
`{ error: { type, code?, message?, param? } }` — vpay's server envelope
passed through 1:1, or one of the three codes this package originates.

| `type`                  | `code`                 | Origin                                                                                                                                                             |
| ----------------------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `invalid_request_error` | `resource_missing`     | The uniform 404. **Every** credential failure renders it byte-identically: unknown publishable key, wrong `client_secret`, another merchant's key, unknown intent. |
| `invalid_request_error` | `invalid_state`        | The intent has already been confirmed. One charge per intent, forever — retry means a new PaymentIntent.                                                           |
| `invalid_request_error` | `checkout_session_expired` | The checkout session driving this intent is over — swept at its 24-hour horizon, expired by the merchant, or past that horizon and not yet swept. `confirmPayment` is refused before any charge is opened. Render the abandoned-checkout screen; a retry needs a **new** checkout session. |
| `invalid_request_error` | `checkout_session_complete` | The session already finished and its intent is `succeeded`. Not an error to retry: read the session and show the outcome.                                          |
| `invalid_request_error` | `charge_declined`      | The rail refused the payment. The charge is terminal and the intent keeps `requires_payment_method` with `last_payment_error` populated; a retry means a new PaymentIntent.                                                     |
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

`loadStripe` and `initEmbeddedCheckout` are the two functions here that _do_
reject, and for the same reason: a blank publishable key, a `checkoutBaseUrl`
that is not an absolute `http(s)` URL, a missing one where an embedded
checkout needs it, or a `fetchClientSecret` that did not return a
`cs_…_secret_…` are integration mistakes visible on the merchant's first page
load rather than payer-facing failures. A rejection from your own
`fetchClientSecret` is passed through **unchanged** — it is your server call
failing, and a message this package invented on top of it would hide the
fault. Checking these once is what lets every other method keep the
never-rejects contract.

## Development

```bash
pnpm --filter @vaam-apps/vpay-stripe-js typecheck
pnpm --filter @vaam-apps/vpay-stripe-js test
pnpm --filter @vaam-apps/vpay-stripe-js build   # tsc → dist/, gitignored
```

Unit tests run against a real `node:http` server standing in for
`/v1/browser` (`src/testing/browser-stub.ts`), not a patched `fetch`: the
contract under test is bytes on the wire. That stub is excluded from `dist`
and imported only from `*.test.ts`, so it is not a test double reachable from
a shipping process in ADR-0006's sense.

## Status

**Verified end to end on the compose stack, not yet in CI.**

- The payment-intent half: implemented, typechecked and unit-tested against
  a `node:http` stub of the surface in
  `docs/plans/2026-09-03-step5c-stripejs.md` §1. 119 tests pass, none
  skipped or ignored.
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
- The redirect _return_ trip **was** a named gap (D4). It was **closed on
  2026-09-04 (Step 9)**: the rail is now told a per-charge `return_url` and
  vpay serves the page the payer comes back to — see
  `docs/flows/hosted-checkout.md`. See `docs/status.md` for the
  repository-wide picture.
- **Checkout is newer and less proven than the rest of this package.**
  `initEmbeddedCheckout` is exercised against a real `iframe` in jsdom
  (`src/embedded.test.ts`, 20 tests) and `retrieveCheckoutSession` against
  the `node:http` stub (`src/checkout-session.test.ts`, 10 tests). Neither of
  those two suites runs against a live vpay. The end-to-end proof that does is
  `frontends/tests/e2e/cypress/e2e/shop-embedded.cy.ts` (Step 9, lane 6),
  which frames vpay's own checkout page on `examples/shop` and completes a
  payment inside it — **green from nothing in the `vpay-ci` VM on
  2026-09-04**. What that spec cannot see is recorded on its own
  `docs/status.md` row.
- **The popup surface (`openCheckoutPopup` / `notifyCheckoutOpener`) is
  unit-tested and has not been driven by a real browser.** `src/popup.test.ts`
  (26 tests) drives it against stub windows, because jsdom implements
  neither `window.open` nor cross-window `postMessage` — so what is proven
  is the origin check, the source check, the open-before-await ordering, the
  blocked-window error, the cancel poll and the round trip between the two
  halves. What is **not** proven anywhere yet is a real browser opening a
  real window: no Cypress spec covers it, and the hand-run of
  `examples/shop`'s popup mode on 2026-09-06 reached only the **fallback** —
  the automation's synthetic click carries no user activation, so
  `window.open` answered `null` and `CheckoutPopupBlockedError` did its job.
  Recorded on `docs/status.md` and as a dated ⛔ in `docs/sdks/parity.md`.
- The `vpay:complete` payload is read strictly: `session` and `status` must
  both be strings. If vpay's checkout page ever sends the session as an
  object, `onComplete` stops firing rather than firing with fields it cannot
  read — visible immediately, and by design.
