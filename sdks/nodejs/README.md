# @vpay/sdk

The Node.js merchant SDK for vpay's `/v1` API. Implements the wire contract in
[`docs/flows/merchant-auth.md`](../../docs/flows/merchant-auth.md) exactly —
`private_key_jwt` client assertions, `client_credentials` token exchange and
caching, the form-encoded resource calls, and outbound-webhook verification.

**This package is `private: true` and is not published.** See "Status" below
for what the server it talks to actually serves.

## Install

Inside this workspace:

```bash
pnpm --filter @vpay/sdk build
```

There is nothing to install from a registry yet. Once a server exists and this
package is published, installation will be `npm install @vpay/sdk` (or the
pnpm/yarn equivalent) like any other package. Requires Node.js `>=22.11.0`.

## The handshake, in prose

`/v1` never accepts an API key ([ADR-0010](../../docs/adr/0010-merchant-auth-private-key-jwt.md)).
Every merchant is a statically registered OAuth2 client authenticating with
`client_credentials` (RFC 6749 §4.4) via a signed `private_key_jwt` assertion
(RFC 7523). Concretely:

1. **Mint an assertion.** The SDK signs a short-lived RS256 JWT with your
   private key: `iss`/`sub` are your `client_id`, `aud` is the token
   endpoint, `jti` is a fresh UUIDv4, and `exp` is `now + assertionLifetimeSeconds`
   (default 60s, capped at 300s — the OP refuses anything further out).
2. **Exchange it for an access token.** The SDK `POST`s the assertion to the
   token endpoint alongside `grant_type=client_credentials` and
   `audience=vpay:v1`. No `client_secret` is ever sent — there is nothing to
   send; vpay stores only your **public** key.
3. **Call `/v1` with the token.** Every resource call carries
   `Authorization: Bearer <access_token>`. The SDK caches the token until
   shortly before it expires and reuses it across calls; concurrent calls
   share one in-flight token request.
4. **Re-authenticate on expiry or a 401.** There is no refresh token by
   design. On a `401` from a resource route, the SDK discards the cached
   token, repeats steps 1–2 once, and retries the original request exactly
   once. A second `401` is returned to you as a `VpayApiError`.

## Usage

```ts
import { readFileSync } from "node:fs";
import { VpayClient } from "@vpay/sdk";

const vpay = new VpayClient({
  baseUrl: "https://api.vpay.example",
  clientId: "merchant_a",
  privateKey: readFileSync("./merchant_a.key.pem", "utf8"),
});

const intent = await vpay.paymentIntents.create(
  {
    amount: 5000, // 5,000 FCFA — XAF is zero-decimal, see docs/flows/money.md
    currency: "xaf",
    payment_method_types: ["mtn_momo"],
    metadata: { order_id: "1234" },
  },
  { idempotencyKey: "order_1234_attempt_1" },
);

console.log(intent.id, intent.status); // pi_..., "requires_payment_method"

const confirmed = await vpay.paymentIntents.confirm(intent.id, {
  payment_method_data: {
    type: "mtn_momo",
    mtn_momo: { msisdn: "237670000000" },
  },
});

// `processing` means NOT YET. Wait for a payment_intent.succeeded webhook,
// or poll retrieve(). There is no `failed` status — see payment-lifecycle.md.
console.log(confirmed.status);

// Orange Money is a redirect rail:
const redirectConfirm = await vpay.paymentIntents.confirm(intent.id, {
  payment_method_data: { type: "orange_money" },
  return_url: "https://shop.example/return",
});
if (redirectConfirm.next_action?.type === "redirect_to_url") {
  // redirect the payer's browser to redirectConfirm.next_action.redirect_to_url.url
}

await vpay.paymentIntents.retrieve(intent.id);
await vpay.paymentIntents.cancel(intent.id);
await vpay.paymentIntents.list({ limit: 10 });

await vpay.refunds.create({
  payment_intent: intent.id,
  reason: "requested_by_customer",
});
await vpay.events.list({ type: "payment_intent.succeeded", limit: 20 });
await vpay.balance.retrieve();
```

`create()` and `retrieve()` responses carry a `client_secret` (`intent.client_secret`,
typed `string | undefined`) — the payer credential `/v1/browser` accepts for a
browser-side confirm. Hand it straight to `@vpay/stripe-js` on the page you
render for the payer; never log it or send it anywhere but the browser. It is
the one field on `PaymentIntent` that is genuinely absent — not `null` — from
every other response shape: a `list()` item and `event.data.object` never
carry it, so a merchant's own listing view or a stored/forwarded webhook body
never receives a live payer credential for intents it did not just create.

## Configuration

| Option                     | Default               | Notes                                                                                                                                                                                                                                                                                        |
| -------------------------- | --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `baseUrl`                  | —                     | Required, e.g. `https://api.vpay.example`.                                                                                                                                                                                                                                                   |
| `clientId`                 | —                     | Required. Your registered OAuth2 `client_id`.                                                                                                                                                                                                                                                |
| `privateKey`               | —                     | Required. PEM text or a `crypto.KeyObject`. Never logged, never serialized.                                                                                                                                                                                                                  |
| `kid`                      | —                     | Required only if you registered more than one JWK.                                                                                                                                                                                                                                           |
| `issuer`                   | `${baseUrl}/v1/oauth` | This default is what the server does: `vpay_api::op::issuer_for` builds the issuer as `{public_base_url}/v1/oauth` from the deployment YAML, and `vpay_api::router` mounts the OP there. Override only for a deployment behind a path prefix.                                                |
| `tokenEndpoint`            | `${issuer}/token`     | The server's token route, and also the assertion's `aud` claim.                                                                                                                                                                                                                              |
| `audience`                 | `vpay:v1`             | The OAuth2 `audience` request parameter. Load-bearing: without it the OP mints a token whose `aud` is the `client_id`, which the resource server rejects. Server-side the same string is `vpay_config::MERCHANT_AUDIENCE`; this package keeps its own copy, so the two must change together. |
| `scope`                    | —                     | Omitted from the token request unless set.                                                                                                                                                                                                                                                   |
| `assertionLifetimeSeconds` | `60`                  | Must be an integer in `1..=300`; anything else throws `VpayConfigError` at construction, not at request time. Keep the default — see the note below the table.                                                                                                                               |
| `timeoutMs`                | `30000`               | Applies to both the token exchange and every resource call.                                                                                                                                                                                                                                  |
| `fetch`                    | global `fetch`        | Injection point for tests or an outbound proxy.                                                                                                                                                                                                                                              |

**Do not raise `assertionLifetimeSeconds` to 300.** 300 is not a comfortable
maximum, it is the OP's exact refusal boundary
(`MAX_CLIENT_ASSERTION_LIFETIME_SECS`), and the OP compares it against _its_
clock. A merchant clock running even one second fast mints an `exp` the OP
reads as 301 seconds out, and every assertion is refused — the failure looks
like `invalid_client`, arrives all at once, and clears up on its own when the
clocks drift back. The default of 60 leaves 240 seconds of headroom for a
value that only has to outlive one HTTP round trip.

## Error handling

Every error extends `VpayError`, so `catch (err) { if (err instanceof VpayError) ... }`
works as a catch-all; narrow further with `instanceof` on a specific subclass:

| Class                         | When                                                                                                                                                                                                                                                                                                                       | Fields                                       |
| ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| `VpayApiError`                | A non-2xx response from a `/v1` resource route.                                                                                                                                                                                                                                                                            | `status`, `type`, `code`, `message`, `param` |
| `VpayAuthError`               | A `400`/`401` from the token endpoint (`invalid_client`, etc). Never retried.                                                                                                                                                                                                                                              | `error`, `errorDescription`                  |
| `VpayUnexpectedResponseError` | A response that isn't the shape this SDK expects — a proxy's HTML 502, an empty body, or a token-endpoint 200 that is not `{ access_token, token_type: "Bearer", expires_in }` (a non-`Bearer` `token_type` lands here, not in `VpayAuthError`: nothing failed to authenticate, the SDK simply cannot present that token). | `status`, `bodyPrefix` (first 500 bytes)     |
| `VpayTransportError`          | DNS, TLS, connection refused, timeout.                                                                                                                                                                                                                                                                                     | `cause` (the underlying error)               |
| `VpayConfigError`             | The SDK was misconfigured — caught at construction where possible.                                                                                                                                                                                                                                                         | —                                            |
| `WebhookSignatureError`       | `verifyWebhook` rejected a signature.                                                                                                                                                                                                                                                                                      | —                                            |

```ts
import { VpayApiError, VpayAuthError, VpayError } from "@vpay/sdk";

try {
  await vpay.paymentIntents.create({
    amount: 5000,
    currency: "xaf",
    payment_method_types: ["mtn_momo"],
  });
} catch (err) {
  if (err instanceof VpayApiError) {
    console.error(err.status, err.type, err.code, err.message, err.param);
  } else if (err instanceof VpayAuthError) {
    console.error("authentication failed:", err.error, err.errorDescription);
  } else if (err instanceof VpayError) {
    console.error(err.name, err.message);
  }
}
```

`VpayApiError` is one class for every error envelope: the envelope's `type`
is a _field_, not a subclass. Branch on `code`, which is what the server
treats as the machine-readable answer — two idempotency errors share a status
(`400`) and a `type` (`idempotency_error`) and mean opposite things:

```ts
if (err instanceof VpayApiError) {
  if (err.code === "idempotency_key_in_flight") {
    // The first request under this key has not finished. Wait, then send
    // the same call again — do not mint a new key.
  } else if (err.code === "idempotency_key_in_use") {
    // The key was already used with a different body. Retrying cannot help.
  }
}
```

## Webhook verification

```ts
import { createServer } from "node:http";
import { verifyWebhook, WebhookSignatureError } from "@vpay/sdk";

// Read the secret once, at startup, so a misconfigured deployment fails
// loudly instead of rejecting every delivery as a bad signature. Under
// `noUncheckedIndexedAccess` this is `string | undefined` until you narrow it.
const webhookSecret = process.env["VPAY_WEBHOOK_SECRET"];
if (webhookSecret === undefined) {
  throw new Error("VPAY_WEBHOOK_SECRET is not set");
}

createServer((req, res) => {
  let raw = "";
  req.on("data", (chunk: Buffer) => (raw += chunk.toString("utf8")));
  req.on("end", () => {
    // `IncomingHttpHeaders` types every header as `string | string[]`, because
    // a header may legitimately be repeated. A repeated `Vpay-Signature` is
    // not something vpay sends, and joining or picking one arbitrarily would
    // verify against bytes nobody promised — so reject it.
    const header = req.headers["vpay-signature"];
    if (typeof header !== "string") {
      res.writeHead(400).end("missing or repeated Vpay-Signature");
      return;
    }

    let event;
    try {
      // The RAW request body must be used — parsing and re-stringifying it
      // breaks the HMAC. Do not run a body-parsing middleware before this.
      event = verifyWebhook({
        rawBody: raw,
        signatureHeader: header,
        secret: webhookSecret,
      });
    } catch (err) {
      if (err instanceof WebhookSignatureError) {
        res.writeHead(400).end("bad signature");
        return;
      }
      throw err;
    }

    // Delivery is at-least-once. verifyWebhook does not dedupe by event.id —
    // that is your job. Keep a set (or a unique DB constraint) of processed
    // event.id values and skip anything you've already handled.
    console.log("event", event.id, event.type);
    res.writeHead(200).end("ok");
  });
}).listen(4242);
```

## Using the official Stripe SDK

vpay's `/v1` surface is Stripe-shaped, and the official
[`stripe`](https://www.npmjs.com/package/stripe) Node SDK can talk to it —
with an **empty API key** and a `config.authenticator` that performs the
`private_key_jwt` handshake above. `@vpay/sdk/stripe` is that authenticator:

```js
import { readFileSync } from "node:fs";
import Stripe from "stripe";
import { createStripeAuthenticator } from "@vpay/sdk/stripe";

const authenticator = createStripeAuthenticator({
  baseUrl: "http://localhost:8080",
  clientId: "acme-cameroon",
  privateKey: readFileSync("./merchant-key.pem", "utf8"),
  kid: "acme-cameroon-2026-08",
});

const stripe = new Stripe("", {
  authenticator,
  host: "localhost",
  port: "8080",
  protocol: "http",
  maxNetworkRetries: 2,
  timeout: 30_000,
  telemetry: false,
});

const intent = await stripe.paymentIntents.create({
  amount: 5000,
  currency: "xaf",
  payment_method_types: ["mtn_momo"],
});
```

`host`, `port` and `protocol` are not optional. The authenticator is bound to
`baseUrl`'s origin: a request addressed anywhere else throws `VpayConfigError`
before a token is minted. Omit them and stripe-node addresses
`api.stripe.com:443`, which is exactly the case this refusal exists to stop.
Behind a reverse proxy, or on split-horizon DNS, set `baseUrl` to the origin
this client actually connects to rather than the public one — both the token
exchange and the binding follow `baseUrl`, so a mismatch either mints the
token against an unreachable endpoint or refuses every request as addressed
elsewhere.

`stripe` is an **optional peer dependency** of this package: install it
yourself if you want this entry point, and `@vpay/sdk`'s core entry never
imports it. This package itself still has **zero runtime dependencies**.

The empty first argument is not a trick — stripe-node's `_setAuthenticator`
refuses only _both_ an API key and an authenticator, or _neither_. The token
cache, its single-flight refresh and its `expires_in` safety margin are the
same ones `VpayClient` uses, not a second implementation.

`authenticator.invalidate()` discards the cached token. You need it because an
authenticator runs _before_ a request and its promise settles before any
status code exists — it cannot see a `401` and re-auth the way `VpayClient`
does:

```js
try {
  await stripe.paymentIntents.retrieve(id);
} catch (err) {
  if (err instanceof Stripe.errors.StripeAuthenticationError) {
    authenticator.invalidate();
  }
  throw err;
}
```

### Where vpay diverges from Stripe

- **No API keys, ever.** `sk_live_…` means nothing here; authentication is the
  OAuth2 `client_credentials` + `private_key_jwt` handshake above
  ([ADR-0010](../../docs/adr/0010-merchant-auth-private-key-jwt.md)). There is
  no publishable key and no Connect equivalent.
- **`Idempotency-Key` is mandatory on every `/v1` POST**, not optional as at
  Stripe. This costs you nothing: stripe-node puts a
  `stripe-node-retry-<uuid>` key on every v1 POST unconditionally — _including_
  when `maxNetworkRetries` is `0`. Pass your own with
  `{ idempotencyKey: "order_1234_attempt_1" }` when you want the retry window
  to span more than one process.
- **XAF and EUR only.** XAF is zero-decimal: `amount: 5000` is 5,000 FCFA, not
  50.00 (see [docs/flows/money.md](../../docs/flows/money.md)).
- **`payment_method_types` is required and non-empty**, and must name an
  enabled vpay rail. A copy-pasted `automatic_payment_methods: { enabled: true }`
  gets a `400` naming `payment_method_types`.
- **No `confirm: true` on create, no `search`, no Connect.** Confirm with a
  second call to `stripe.paymentIntents.confirm(id, …)`: `confirm: true` on
  create is **refused** with `param: "confirm"` and a message naming the
  confirm route. `stripe.paymentIntents.search(…)` and every `stripeAccount` /
  Connect parameter have no vpay counterpart.
- **The fields that decide where or when money moves are refused, not
  ignored.** `capture_method` with any value other than `automatic`,
  `application_fee_amount`, `transfer_data` and `on_behalf_of` come back as a
  `400` naming the field in `error.param`. vpay has no authorise-now /
  capture-later split — confirming _is_ the charge — and no Connect, so
  ignoring any of them would move a merchant's money somewhere, or at a time,
  they did not ask for and could not see in the response.

  Everything else Stripe sends and vpay does not implement is **accepted and
  ignored**, because none of it changes that: `setup_future_usage`,
  `confirmation_method`, `receipt_email`, `statement_descriptor`, `customer`,
  `expand` and `metadata` (`metadata` is actually stored) all leave the
  payment exactly as requested. Nothing is expandable, so an `expand` request
  simply produces no expanded field — visible in the response itself.

- **Rail codes are vpay's, not Stripe's.** `payment_method_types` entries and
  `payment_method_data.type` are `mtn_momo` / `orange_money`; stripe-node's
  TypeScript types enumerate Stripe's own rails and reject them, so a cast is
  needed at those call sites — the values are right, the _type_ is Stripe's:

  ```ts
  await stripe.paymentIntents.confirm(id, {
    payment_method_data: {
      type: "mtn_momo",
      mtn_momo: { msisdn: "237670000000" },
    },
  } as unknown as Stripe.PaymentIntentConfirmParams);
  ```

- **`429` is never emitted.** Nothing in vpay constructs a rate-limit
  category, so `Stripe.errors.StripeRateLimitError` cannot occur. Do not build
  a backoff path around it.
- **`client_secret` is present, but only on `create`/`retrieve`.** vpay's
  `create()` and `retrieve()` responses carry it — see "Usage" above — for
  `@vpay/stripe-js`'s browser-side confirm flow. `amount_received`,
  `capture_method` and `confirmation_method` stay genuinely absent, though:
  stripe-node's TypeScript types declare all three as present on every
  response shape — it casts responses, it never validates them — so these
  read as `undefined` at runtime while the type says otherwise, on every
  route including `create`/`retrieve`.
- **`Stripe-Version`, `Stripe-Account`, `Stripe-Context` and the
  `X-Stripe-Client-*` headers are accepted and ignored.** vpay advertises no
  dated API version, so `obj.lastResponse.apiVersion` is `undefined` and
  pinning `apiVersion` has no effect.

### Webhooks

vpay signs outbound deliveries with the same construction Stripe uses —
`t=<unix>,v1=<lowercase hex HMAC-SHA256 of "<t>.<raw body>">` — so
`stripe.webhooks.constructEvent(rawBody, header, secret)` verifies a vpay
delivery byte for byte. Read the header value, not the request object, and
give it the **raw** body:

```js
const event = stripe.webhooks.constructEvent(
  rawBody,
  req.headers["stripe-signature"],
  process.env.VPAY_WEBHOOK_SECRET,
);
```

**Either header works, and that snippet is observed rather than argued.**
`vpay_worker`'s deliverer sends `Vpay-Signature` and `Stripe-Signature`
carrying the same string, byte for byte, and
`sdks/stripe-compat`'s `webhooks.compat.test.ts` takes a delivery out of a
WireMock receiver's request journal and puts it through
`stripe.webhooks.constructEvent` exactly as written above — then requires the
same call to throw `StripeSignatureVerificationError` for a body with one byte
flipped and for the right body with the wrong secret.

`Stripe-Signature` exists only so that a copy-pasted Stripe recipe needs no
edit at all. Code written from scratch should read the authoritative name
instead — the same call, one word changed, verifying the same bytes:

```js
const event = stripe.webhooks.constructEvent(
  rawBody,
  req.headers["vpay-signature"],
  process.env.VPAY_WEBHOOK_SECRET,
);
```

**`Vpay-Signature` is the authoritative name** whatever else is added beside
it — prefer it in code you write from scratch, and use this package's own
`verifyWebhook` if you are not otherwise using stripe-node.

### A stripe-node defect you will hit if your key is wrong

**Measured against `stripe@22.6.1`, not inferred.** When the vpay handshake
fails — a bad key, an unregistered `client_id`, vpay unreachable — the
authenticator rejects, as it must. stripe-node builds the right error from it
(`StripeError: Unable to authenticate the request`, with the underlying
`VpayAuthError` at `err.raw.exception`) but **throws it inside a detached
promise chain that never calls its own callback**. The result: the error
arrives as a process-level `unhandledRejection`, and the promise you awaited
**never settles at all** — not even after `timeout`, because no HTTP request
was ever started for a timeout to fire against.

Two of the ways a key can be wrong no longer reach that path at all. A
`privateKey` that `node:crypto` cannot read, and a `baseUrl` that is not an
absolute URL, are both `VpayConfigError` thrown by `createStripeAuthenticator`
**at construction** — before a `Stripe` instance exists, on the line the
merchant wrote. What is left for the warning below is the set of failures only
the OP can report: an unregistered `client_id`, a key whose public half vpay
does not hold, a clock too far out, vpay unreachable.

Until stripe-node fixes this, do not rely on `try`/`catch` around a call to
surface a handshake failure. Two things that do work:

- **Verify the handshake at startup**, where a rejection is yours to catch.
  The authenticator is a plain function, so call it once against a throwaway
  request object and let it throw before you serve traffic — a `VpayAuthError`
  there names the OAuth2 reason (`invalid_client`, …) directly:

  ```js
  await authenticator({ headers: {} });
  ```

- **Install a process-level guard** so a mid-flight handshake failure is at
  least logged and alerted on rather than silently hanging a request:
  `process.on("unhandledRejection", …)`.

`src/stripe-auth.test.ts` pins this behaviour, so if a future `stripe` release
routes the failure to the caller the test fails and this warning gets deleted.

### Status

`createStripeAuthenticator` is **built and tested**. Its tests run against a
real `node:http` server and, for the end-to-end case, drive the **real
`stripe` package**: `new Stripe("", { authenticator, host, port, protocol })`
creating a PaymentIntent, with the stub asserting the `Authorization` header
the authenticator minted, the form-encoded body, and stripe-node's
auto-generated `Idempotency-Key`. The type-level assertion that the returned
function is assignable to `Stripe.StripeConfig["authenticator"]` is checked by
`pnpm --filter @vpay/sdk typecheck`.

What is **not** proven here:

- **Nothing in this package has run against a real `vpay-server`.** The
  end-to-end test's server is a stub in this repository. What _has_ run
  against a real one is
  [`sdks/stripe-compat`](../stripe-compat/) — the official `stripe` package
  driven through this authenticator against a live compose stack, 25 cases,
  run by CI's `e2e (compose)` job. It is a separate package on purpose: it
  needs a stack, and a suite that needs a stack must not be able to report
  green in a job that has none.
- **Every webhook verified so far was delivered to a WireMock receiver on a
  compose network.** The _signature_ is now proven three ways — against this
  package's own `verifyWebhook`, against `vpay_sdk::webhooks::verify_at`, and
  against the official `stripe` package's `constructEvent` — but **no merchant
  endpoint has ever been POSTed to**, and there is no SSRF protection on the
  destination ([`docs/flows/webhooks.md`](../../docs/flows/webhooks.md)).
- **The `stripe` peer range (`^22.6.1`) is asserted, not derived.** It is the
  current major at the time of writing, tested against exactly `22.6.1`.
  Nothing pins vpay against a future `stripe` release; a dependency bump is
  what will surface a break.
- **The server-side compatibility work is not in this package**, though it
  has now landed beside it: the `request-id` response header stripe-node
  reads, `stripe-should-retry` derived from `Classify::retry`, and refusing
  `confirm: true` on create rather than ignoring it are all in `vpay-api`,
  with their own unit and integration tests, and are exercised end to end by
  `sdks/stripe-compat`. See
  [`docs/flows/stripe-sdk-compat.md`](../../docs/flows/stripe-sdk-compat.md).

## `scripts/mint-assertion.mjs`

Mints one assertion and prints it, alongside the public JWK derived from the
same private key, as JSON on stdout — used to feed a real OP verifier (e.g. a
Rust example driving `authkestra_op::client_assertion::verify_client_assertion`
directly) without standing up a server:

```bash
pnpm --filter @vpay/sdk build
VPAY_CLIENT_ID=merchant_a \
VPAY_PRIVATE_KEY_FILE=./merchant_a.key.pem \
VPAY_AUDIENCE=https://api.vpay.example/v1/oauth/token \
  node sdks/nodejs/scripts/mint-assertion.mjs
```

## Status

**Corrected 2026-09-03.** This section used to say "what it does **not**
have is a single `/v1` resource route" — true when it was written on
2026-09-02, and false since Step 2 landed the next day.

`vpay-server` mounts the merchant OP — `POST /v1/oauth/token`,
`GET /v1/oauth/jwks.json`,
`GET /v1/oauth/.well-known/openid-configuration` — puts a bearer-token
boundary in front of every other `/v1` path, and **serves four
payment-intent routes past it**: create, retrieve and list on
`/v1/payment_intents`, plus `/{id}`, `/{id}/confirm` and `/{id}/cancel`. A
confirm reaches a real rail adapter over HTTP and moves the intent to
`processing`, and `vpay-worker` then polls the rail and settles it, so an
intent does reach **`succeeded`** — `sdks/stripe-compat` watches one do so
through `paymentIntents.retrieve` and nothing else. `/v1/events` is served
(Step 5); `/v1/refunds` and `/v1/balance` are still the honest
`404 unknown_route`. **This package has no client method for any of that
beyond the payment-intent routes it already wraps.**

**No test in _this package_ has ever talked to a vpay** — every server in
these tests is a `node:http` stub started by the test. What has talked to a
real one is `sdks/stripe-compat`, which drives the official `stripe` package
through this package's own `createStripeAuthenticator` against a live
compose stack (25 cases, CI's `e2e (compose)` job), and the Rust SDK's
`backends/tests/integration/tests/merchant_token_flow.rs`. See
[`docs/status.md`](../../docs/status.md),
[`docs/flows/merchant-auth.md`](../../docs/flows/merchant-auth.md) and
[`docs/flows/stripe-sdk-compat.md`](../../docs/flows/stripe-sdk-compat.md).

What the tests in this package **do** prove. Everything that touches HTTP
runs against a real `node:http` server started by the test (never a mocked
`fetch`) and asserts the actual bytes sent on the wire; the form encoder,
the webhook verifier and the error-body bounding are pure unit tests, since
none of them touches the network:

- The client assertion this SDK mints has every claim and header field the
  wire contract specifies, is a fresh `jti` per mint, and its RS256 signature
  verifies with `node:crypto` against the matching public key — and fails
  against a different one.
- The full `client_credentials` + `private_key_jwt` token exchange: exact
  form fields, content type, and the resulting `Authorization: Bearer`
  header on the following resource call.
- Token caching (`expires_in` minus the safety margin), single-flight
  refresh under concurrent callers, and the single 401-triggered re-auth
  retry (including that a second consecutive 401 surfaces as `VpayApiError`
  without a third token request).
- Every resource method's exact HTTP method, path, headers, and
  bracket-encoded body — including that a caller-supplied `Idempotency-Key`
  is honoured and one is generated when omitted.
- That `create()` and `retrieve()` surface `client_secret` typed (`string`)
  when the server sends it, and that a `list()` item without one reads back
  `undefined` rather than a cast or a runtime crash.
- The form encoder's nested-object/array/boolean/percent-encoding rules, and
  that a non-integer `amount` throws `TypeError` before any request is sent.
- Error mapping for a Stripe-shaped `400`, an unexpected `502` HTML body, a
  connection failure, and a token-endpoint `401` (which is never retried).
- Webhook verification: valid/invalid signatures, timestamp tolerance,
  multi-signature secret rotation, malformed headers, a one-byte body change,
  and that a `Buffer` body and its string form verify identically.
- That `util.inspect(client)` and `JSON.stringify(client)` never contain the
  private key, and that neither the client nor any error this SDK throws
  carries an access token into `util.inspect` output.

**Separately — and this is _not_ one of the tests above — the assertion has
been checked against the real OP verifier by hand.** An assertion minted by
this SDK was fed directly to
`authkestra_op::client_assertion::verify_client_assertion` at the pinned
`authkestra-op = "=0.7.1"`, against a `ClientRegistration` holding the
corresponding public JWK, and was accepted. `just sdk-conformance-node`
reproduces it: the recipe mints an assertion via `scripts/mint-assertion.mjs`
and pipes it into `cargo run -p vpay-sdk --example verify_assertion`. It is a
manual check, run on demand — it needs a Rust toolchain, it is not part of
`pnpm test`, and no CI job gates on it. Nothing in the list above depends on
it having been run.

What this does **not** prove: that any of this works against a real vpay
deployment. Nothing in this package has been run against `vpay-server`; the
`issuer`/`tokenEndpoint` defaults agree with `vpay_api::op::issuer_for` by
inspection, not by a test in this repository that exercises them together,
and the resource routes every method here calls do not exist server-side at
all — see [`docs/status.md`](../../docs/status.md).
