# Integrating vpay's checkout page — hosted and embedded

**Who this is for.** A merchant integrating vpay who does not want to build a
payer page. There are two ways to use vpay's own page, and this runbook walks
both with `examples/shop` — the demo storefront in this repository — as the
worked example, because every snippet below is code that actually runs and is
driven by a browser in CI.

**What this page is not.** It is not the design; that is
[../flows/hosted-checkout.md](../flows/hosted-checkout.md). It is not evidence
that vpay takes real money: every rail behind everything here is a WireMock
host on a compose network ([../status.md](../status.md)).

**Before you start**, on the vpay side, an operator has to have done two things
in the deployment's YAML — both are boot-checked
([../flows/configuration.md](../flows/configuration.md)):

```yaml
checkout:
  # The origin every payer link vpay mints is built on. Absent means this
  # deployment serves no checkout page, and POST /v1/checkout/sessions answers
  # `checkout_not_configured` rather than minting a url that resolves nowhere.
  public_base_url: http://localhost:3080

merchant_clients:
  - client_id: shop-merchant
    merchant_id: shop-merchant-tenant
    # What a payer is told they are paying. Optional — without it the page
    # paints a neutral heading rather than an identifier. The demo overlay
    # `just gen-demo-keys` writes sets none, so `just demo`'s pages show the
    # neutral heading and not this name.
    display_name: "Boutique Acme Cameroun"
    publishable_keys: ["pk_test_shopmerchantsandbox1"]
    # ONLY for embedded mode. Empty (the default) means no site may frame the
    # page. Canonical spellings only: lower-cased host, no default port.
    checkout_origins: ["http://localhost:3001"]
    webhooks:
      - id: shop
        url: http://vpay-shop:3000/api/vpay/webhook
        secrets: ["${SHOP_WEBHOOK_SECRET}"]
```

---

## 1. Hosted: vpay's page on vpay's origin

**The shape.** Your server creates a PaymentIntent, then a Checkout Session
against it, and redirects the payer to `session.url`. vpay collects the payment
and sends the payer back to your `success_url` or `cancel_url`.

**Your server** — `examples/shop/src/server/orders.ts`, `placeOrder`, trimmed
to the two vpay calls:

```ts
const intent = await vpay.paymentIntents.create(
  {
    amount: priced.totalMinor,          // integer minor units — 5000 is 5,000 FCFA
    currency: priced.currency,          // "xaf"
    payment_method_types: ["mtn_momo", "orange_money"],
    metadata: { shop_order_id: order.id },
  },
  { idempotencyKey: `shop-order-${order.id}-intent` },
);

// Persisted BEFORE a session exists, and before anything could send a payer
// anywhere: a crash between these two lines leaves an order naming a real
// intent, which the webhook can still settle.
await store.setPaymentIntentId(order.id, intent.id);

const session = await vpay.checkout.sessions.create(
  {
    payment_intent: intent.id,
    ui_mode: "hosted",
    success_url: `${shopUrl}/orders/${order.id}/return?session_id={CHECKOUT_SESSION_ID}`,
    cancel_url: `${shopUrl}/orders/${order.id}/cancelled`,
  },
  { idempotencyKey: `shop-order-${order.id}-session-hosted` },
);
await store.setCheckoutSessionId(order.id, session.id);

return { url: session.url };   // redirect the payer here
```

Four things in that snippet are decisions, not incidental:

- **The amount is never on the wire from a browser.** The shop's tRPC input
  schema has product ids and quantities and no price field, and the total is
  computed from the catalogue server-side. A browser cannot send a wrong amount
  because it cannot send an amount.
- **`Idempotency-Key`s are derived from the order id**, not random, so every
  retry of the *same* order sends the same key and cannot leave vpay holding two
  intents or two sessions for one order. It does **not** deduplicate two
  separate checkout submissions: pressing "Pay" twice creates two orders with
  two ids, and vpay is right to create both.
- **`{CHECKOUT_SESSION_ID}` is a literal.** vpay substitutes it when it
  forwards the payer (D5). Do not percent-encode it, and do not treat what comes
  back as authority — see §3.
- **A hosted session with no `url` is an error, not something to route around.**
  The shop throws rather than redirecting somewhere plausible.

**What the payer sees.** `{checkout.public_base_url}/c/{cs_id}?key={pk}#{secret}`
— the amount, your `display_name` if you configured one, and a rail selector
when the intent offers more than one rail the page can drive. MTN collects a
Cameroon phone number and polls; Orange sends the payer to the rail's own page
and receives them back on vpay's return page. Then vpay forwards them to your
`success_url` or `cancel_url`.

**The credential lives in the URL's `#fragment`,** which a browser never sends
to a server, never writes to an access log and never carries across a redirect.
Do not move it into the query string, and do not log the `url`.

## 2. Embedded: vpay's page in an iframe on yours

**The shape.** Your server creates the PaymentIntent as above but stops there;
your *page* asks your server for a session `client_secret` and hands it to
`@vaam-apps/vpay-stripe-js`, which frames vpay's page.

**Your server** — `examples/shop/src/server/orders.ts`, `embeddedClientSecret`:

```ts
// Created on demand and RETRIEVED thereafter: vpay allows one open session per
// PaymentIntent, and `fetchClientSecret` may legitimately be called twice (a
// remount, a reload).
const session = order.checkoutSessionId
  ? await vpay.checkout.sessions.retrieve(order.checkoutSessionId)
  : await vpay.checkout.sessions.create(
      {
        payment_intent: order.paymentIntentId,
        ui_mode: "embedded",
        return_url: `${shopUrl}/orders/${order.id}/return?session_id={CHECKOUT_SESSION_ID}`,
      },
      { idempotencyKey: `shop-order-${order.id}-session-embedded` },
    );
return { clientSecret: session.client_secret };
```

An embedded session takes `return_url` and **refuses** `success_url` and
`cancel_url`; a hosted one is the reverse. A retrieved session that is no longer
`open` must not be handed back — its `client_secret` still renders, and putting
a dead credential into an iframe leaves the payer looking at a page that cannot
be paid with nothing saying why. The shop refuses with a `409`.

**Your page** — `examples/shop/src/components/embedded-checkout.tsx`:

```tsx
const stripe = await loadStripe(publishableKey, { baseUrl: apiBaseUrl, checkoutBaseUrl });
const checkout = await stripe.initEmbeddedCheckout({
  fetchClientSecret: async () => {
    const result = await trpc.orders.embeddedSecret.mutate({ orderId });
    return result.clientSecret;   // from YOUR server; the browser never sees a merchant credential
  },
  onComplete: () => {
    // A message from an iframe: a CUE, not evidence. Navigate to a page that
    // reads your own database, which only the webhook writes.
    router.push(`/orders/${orderId}/return`);
  },
});
checkout.mount("#vpay-embedded-checkout");
```

and `checkout.destroy()` on unmount.

Three things worth knowing before you debug this at 2 a.m.:

- **The publishable key and the checkout origin should be runtime values, not
  `NEXT_PUBLIC_*`.** The shop passes them down as props from a server
  component, so one image serves the demo stack and a real deployment.
  Next replaces a dotted `process.env.NEXT_PUBLIC_FOO` with a literal at build
  time, which would bake one deployment's URLs into the bundle.
- **The frame starts at `height: 0` and grows on a `vpay:resize` message.** If
  it stays empty, the page never painted — look at the browser console, not at
  a server log.
- **A redirect rail cannot navigate out of the frame.** vpay's page posts
  `{type: 'vpay:redirect', url}` and the SDK performs the top-level
  navigation, because the iframe is sandboxed without `allow-top-navigation`.

## 3. The outcome comes from the webhook, and from nothing else

**Do not mark an order paid from the return page.** The payer arriving back at
your `success_url` means the payer's browser was pointed at it — nothing more.
vpay's own page reads the outcome from an authenticated status query, and the
thing that tells *you* is a signed webhook.

`examples/shop/src/server/webhook.ts`:

```ts
const event = verifyWebhook({ rawBody, signatureHeader, secret });   // @vaam-apps/vpay-sdk

// `Object.hasOwn`, not a bare index: `event.type` is text this code did not
// produce, and SETTLING_EVENTS["constructor"] is a truthy function.
const nextStatus = Object.hasOwn(SETTLING_EVENTS, event.type)
  ? SETTLING_EVENTS[event.type]
  : undefined;

// Dedupe by event id, write the order, and only then answer 2xx.
```

Read the **raw bytes**. A body that was parsed and re-serialised will not
verify — the signature is over the exact bytes vpay sent. Answer `2xx` only
after the write; a `2xx` before it is a delivery vpay will not retry for an
order you did not update.

The shop's return page (`/orders/{id}/return`) polls its own
`orders.get` — which reads the shop's database and never calls vpay — and shows
"we are confirming your payment" until the webhook lands. The `session_id` vpay
substituted into the URL is printed as a label and decides nothing.

## 4. Buying something in the demo shop

With `just demo` green ([demo.md](demo.md)) and the default ports:

1. **`http://localhost:3001`** — five products priced in FCFA. Add one to the
   cart.
2. **`/cart`** — line totals and the cart total. The number here is for display;
   the price that counts is computed on the server.
3. **"Checkout"** → the e-mail is **optional** (2026-09-06) and the surface
   selector below it starts on **Redirect** → **"Pay on vpay's page"**. Your
   browser is now on `http://localhost:3080/c/cs_…`, and the shop has written
   an `unpaid` order, an intent and a session in that order.
4. **Pay.** MTN: type `237600000100`. Orange: follow the redirect to the stub's
   page and click "Pay". (The digits-only MSISDNs are the ones the page's E.164
   validator accepts — see
   [../flows/adapter-mtn-momo.md](../flows/adapter-mtn-momo.md)'s steering
   table.)

   **To make it fail instead**, use one of the test numbers the checkout page
   now lists: MTN `237600000101` (insufficient funds), `237600000102`
   (timeout), `237600000400` (the rail has no such account), `237600000503`
   (the rail is unavailable). The order then reaches `failed` with a
   `failure_code`, the shop shows a sentence written for that code, and — for
   a payer-actionable one — a "Try again" that places a **new** order.
   Orange's own numbers are listed there too and **do not work from a
   browser**; the panel says why, above the table.
5. **You land on `/orders/{id}/return`.** It says "we are confirming your
   payment" and polls every two seconds — the **shop's** database, not vpay.
6. **Within a few seconds it turns to "Paid"**, because vpay's worker delivered
   `payment_intent.succeeded` to `http://vpay-shop:3000/api/vpay/webhook` and
   the shop verified it. Watch it:

   ```console
   $ DEMO_COMPOSE="-f compose.yml -f compose.e2e.yml -f compose.demo.yml"
   $ docker compose $DEMO_COMPOSE logs vpay-shop | grep 'vpay webhook'
   vpay webhook: payment_intent.succeeded evt_… -> 200 applied
   ```

   A replayed delivery answers `200 duplicate` and writes no second row:

   ```console
   $ docker compose $DEMO_COMPOSE exec postgres \
       psql -U vpay -d shop -c 'SELECT id, status FROM orders' \
                             -c 'SELECT id, type FROM webhook_events'
   ```

7. **The embedded mode**, on any unpaid order:
   `http://localhost:3001/orders/{id}/embedded`. vpay's page renders in an
   iframe served from `http://localhost:3080/e/cs_…`, allowed to frame there
   because `http://localhost:3001` is in `shop-merchant`'s `checkout_origins`.
   Paying inside it produces a `vpay:complete` message; the shop treats that as
   a cue and sends you to the return page, which — again — waits for the
   webhook.

## 5. An unregistered origin is refused, and how to see it

Frame the embedded page from any origin that is not in that merchant's
`checkout_origins` and it will not render. **Two independent mechanisms refuse
it**, and it is worth knowing which one you are looking at:

1. **The header.** vpay serves the embedded page with
   `Content-Security-Policy: frame-ancestors <the merchant's list>`, and a
   browser refuses the frame before a pixel is painted. Check it directly:

   ```console
   $ curl -sS -D- -o /dev/null 'http://localhost:3080/e/cs_…?key=pk_test_shopmerchantsandbox1'
   content-security-policy: frame-ancestors http://localhost:3001
   referrer-policy: no-referrer
   cache-control: no-store
   ```

   A merchant with no `checkout_origins` — the default — gets
   `frame-ancestors 'none'`, and so does a deployment whose checkout app cannot
   reach vpay to ask. It fails **closed**.

2. **The page itself.** Independently of the header, the page resolves its
   framer from `document.referrer` against the same list and refuses before it
   reads any credential — so the refusal cannot be used to probe which half of a
   link is wrong. This is what you see rendered: "This page will not load here."

**Which of the two you have actually observed matters.** Everything this
repository has *measured* is the second one: Cypress strips
`Content-Security-Policy` from every document it proxies, so
`shop-embedded.cy.ts` asserts the header **as the server sends it** (with
`cy.request`) and then watches the page's own origin check refuse an
unregistered framer — proven origin-driven by registering that origin in the
overlay and watching the same page render. **No browser has been observed
refusing a frame because of vpay's CSP.** If you are validating a deployment,
check the header with `curl` and the refusal in a real browser, and treat them
as two facts.

**Adding an origin is a config change**, which is the point
([ADR-0003](../adr/0003-yaml-configuration.md)): put it in the merchant's
`checkout_origins`, canonically spelled (lower-cased host, IDNA to ASCII,
default port elided), and restart. `https://Shop.example` and
`https://shop.example:443` are refused at boot rather than normalised, because
the alternative was the page dropping them silently and leaving the merchant
unable to embed with nothing to read.

## 6. When something goes wrong

| Symptom | Cause |
|---|---|
| `POST /v1/checkout/sessions` answers `checkout_not_configured` | The deployment has no `checkout.public_base_url`. It answers **500**, not 503 — see [../flows/hosted-checkout.md](../flows/hosted-checkout.md)'s last section for why that is a maintainer's decision and not an oversight |
| `create` is a `400` naming `payment_intent` | The intent must be `requires_payment_method`, have no charge, and have no other open session. One open session per intent is a database index, not just a check |
| The payer's page says "invalid link" | The `url` lost its `#fragment` — something copied it through a redirect, a logger, or a link with a `?query` and no fragment |
| The embedded iframe is an empty box | The page never painted, so no `vpay:resize` arrived. Browser console, not server log |
| The framed page says "This page will not load here" | The framing origin is not in that merchant's `checkout_origins`. §5 |
| `session.url` resolves to nothing | `checkout.public_base_url` and the page's actual origin disagree. Nothing logs a port; compare the two by hand |
| The order never turns `paid` although vpay says `succeeded` | Your webhook endpoint. Check the signature secret on both sides, and that you are verifying the **raw** bytes |
| Every token request is `invalid_client` and everything else is right | Your assertion's `aud`. It must be vpay's own token endpoint, not the URL you POST to, if your server reaches vpay by an internal name — [../flows/merchant-auth.md](../flows/merchant-auth.md) |

## Status

Written 2026-09-04 with Step 9, from `examples/shop`'s own source. Every
snippet is code in this repository, and §4's **buying flow** is driven end to
end in a real browser by `frontends/tests/e2e/cypress/e2e/shop-hosted.cy.ts`
and `shop-embedded.cy.ts`, green from nothing in the `vpay-ci` VM — **its
`docker compose` and `psql` verifications are not**: no spec runs them, and
they are here for a human to run by hand.

**What has not happened:** no human has followed this page against a deployment
of vpay, because no deployment exists; no real rail has been called; and the
§5 CSP check has not been performed by anyone against a browser — only the
page's own origin refusal has. See [../status.md](../status.md).
