# @vaam-apps/vpay-sdk

The Node.js merchant SDK for vpay's `/v1` API. Every call is a COSE_Sign1
message signed with your Ed25519 key, every answer is opened under vpay's
pinned response keys, and the signing is **the Rust SDK's own**, compiled to
WebAssembly (`sdks/rust/wasm`) — see "How a call is signed" below. The wire
contract is [`docs/flows/signed-transport.md`](../../docs/flows/signed-transport.md);
the resource calls, the form-encoded bodies and outbound-webhook verification
are as documented in [`docs/flows/merchant-auth.md`](../../docs/flows/merchant-auth.md).

**On the registry since 2026-09-19.** `release.yml`'s `publish-node-sdk` job
(added 2026-09-18) publishes it on every `v*` tag, and `npm view
@vaam-apps/vpay-sdk versions` listed `0.2.1` (the first, 2026-09-19) through
`0.4.1` on 2026-09-23. _(This paragraph read "Not yet on the registry …
`E404` on 2026-09-05 and no workflow in this repository publishes anything"
until 2026-09-23. The workflow half was wrong from 2026-09-18, and the
registry half from 2026-09-19.)_ Publishing it proves nothing about what the
server it talks to serves; see "Status" below.

## Install

Inside this workspace (needs Rust, the `wasm32-unknown-unknown` target and
`wasm-bindgen-cli` at the version `Cargo.lock` pins — `just wasm-build` says
which to install if it is missing):

```bash
pnpm --filter @vaam-apps/vpay-sdk build
```

From the registry (the published package ships the compiled `.wasm`; nothing to
build):

```bash
pnpm add @vaam-apps/vpay-sdk
```

Requires Node.js `>=22.11.0`.

## How a call is signed

`/v1` accepts no API key and no token ([ADR-0035](../../docs/adr/0035-v1-is-a-signed-message-not-a-token.md)).
A merchant is a registered **Ed25519 public key** (`merchant_clients[].cose_keys`
in vpay's configuration); vpay finds you by the key that signed. Concretely:

1. **Seal.** For each call the SDK builds a COSE_Sign1 message over the
   payload and binds the audience, method, route template, path parameters,
   canonical query, the operation's contract digest, the payload type and the
   `Idempotency-Key`. The message **is the request body** — even for a `GET`.
2. **Send.** On `node:http`/`node:https`, because `fetch` refuses a body on
   `GET`. One attempt: a signed request is never retried by the SDK.
3. **Open.** The answer is a COSE_Sign1 message too. The SDK verifies it under
   `serverKeys.current` or `serverKeys.next`, and against _this_ request and
   status, before it reads a byte of JSON. An answer that is not sealed at all
   (vpay's coarse `401`, a proxy's error page) is `VpayUnsignedResponseError`;
   one that claims to be and does not verify is `VpayUnverifiedResponseError`.

There is no token to cache and no re-authentication. The sealing is not
reimplemented in TypeScript: `wasm/` is `sdks/rust/wasm` built with
`wasm-bindgen --target nodejs`, i.e. the same code path `vpay-sdk` (Rust) runs.
Your private key either goes in as a seed (the module signs) or **stays in
Node** behind a `crypto.KeyObject` or a `signer` callback, in which case the
module only ever sees the bytes to sign.

## Usage

```ts
import { readFileSync } from "node:fs";
import { VpayClient } from "@vaam-apps/vpay-sdk";

const vpay = new VpayClient({
  baseUrl: "https://api.vpay.example",
  clientId: "merchant_a", // kept for logs; vpay finds you by your key
  // Your Ed25519 seed (32 bytes, unpadded base64url — `cargo xtask
  // gen-envelope-key` writes it), a PKCS#8 PEM or a crypto.KeyObject.
  privateKey: readFileSync("./merchant_a.seed", "utf8"),
  // The two response keys vpay publishes (it logs them at boot): the one it
  // signs with now and the one it will after the next rotation.
  serverKeys: {
    current: readFileSync("./vpay-current.pub", "utf8").trim(),
    next: readFileSync("./vpay-next.pub", "utf8").trim(),
  },
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
// vpay.balance.retrieve() is in the surface but no route serves it: it is
// refused locally (VpayInvalidParamsError) before anything is signed.

// Checkout Sessions (Step 9). A session references an intent you already
// created; it never creates one, and it carries where the payer goes next.
const hosted = await vpay.checkout.sessions.create({
  payment_intent: intent.id,
  ui_mode: "hosted",
  // vpay substitutes {CHECKOUT_SESSION_ID} when it forwards the payer, and
  // appends nothing else. Omit it and the return has no correlation.
  success_url: "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
  cancel_url: "https://shop.example/cancel",
});
// Send the payer to hosted.url — the session secret is in its fragment.

const embedded = await vpay.checkout.sessions.create({
  payment_intent: intent.id,
  ui_mode: "embedded",
  return_url: "https://shop.example/order/42",
});
// Hand embedded.client_secret to @vaam-apps/vpay-stripe-js's initEmbeddedCheckout.

await vpay.checkout.sessions.retrieve(hosted.id);
await vpay.checkout.sessions.list({ payment_intent: intent.id, limit: 10 });
await vpay.checkout.sessions.expire(hosted.id); // open -> expired
```

`create()` and `retrieve()` responses carry a `client_secret` (`intent.client_secret`,
typed `string | undefined`) — the payer credential `/v1/browser` accepts for a
browser-side confirm. Hand it straight to `@vaam-apps/vpay-stripe-js` on the page you
render for the payer; never log it or send it anywhere but the browser. It is
the one field on `PaymentIntent` that is genuinely absent — not `null` — from
every other response shape: a `list()` item and `event.data.object` never
carry it, so a merchant's own listing view or a stored/forwarded webhook body
never receives a live payer credential for intents it did not just create.

A Checkout Session's own `client_secret` (`cs_…_secret_…`) is a **different**
credential from the intent's: it authorises reading that session, never
confirming the intent. Like the intent's, it is present on `create()` and
`retrieve()` only and absent from every `list()` item. Unlike the intent's, it
does not print: every `CheckoutSession` this SDK returns redacts it — and the
fragment of a hosted session's `url`, which holds the same value — from
`util.inspect`, so `console.log(session)` cannot leak it. `JSON.stringify` is
left faithful, because an embedded integration has to serialise the secret to
get it to the browser at all.

## Configuration

| Option       | Default          | Notes                                                                                                                                                                                                                                                                                                                                         |
| ------------ | ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `baseUrl`    | —                | Required, e.g. `https://api.vpay.example`.                                                                                                                                                                                                                                                                                                    |
| `clientId`   | —                | Required. Your registered `client_id`. Not sent: vpay resolves the merchant from the signing key.                                                                                                                                                                                                                                             |
| `privateKey` | —                | Exactly one of this and `signer`. An Ed25519 key as the 32-byte seed (bytes, or unpadded base64url text), PKCS#8 PEM text, or a `crypto.KeyObject`. An RSA or P-256 key is a `VpayConfigError` at construction. Never logged, never serialized.                                                                                               |
| `signer`     | —                | `{ publicKey, sign(tbs) }` for a key this process cannot read (a KMS, an HSM). `sign` receives the exact bytes to sign and returns the 64-byte pure-Ed25519 signature (`crypto.sign(null, tbs, key)`). A signature that does not verify under `publicKey` is refused before it is sent, with the cause, instead of becoming the coarse `401`. |
| `serverKeys` | —                | Required. `{ current, next? }`, each 32 bytes (bytes or unpadded base64url). An answer sealed by any other key is refused (`VpayUnverifiedResponseError`). Pin `next` before vpay rotates so a rotation is not a flag day.                                                                                                                    |
| `audience`   | `baseUrl`        | What every signature is addressed to: vpay's `deployment.public_base_url`. Set it when your server reaches vpay by a different URL than payers do — see below.                                                                                                                                                                                |
| `timeoutMs`  | `30000`          | Applies to every resource call.                                                                                                                                                                                                                                                                                                               |
| `agent`      | the global agent | An `http.Agent`/`https.Agent` for a proxy or a pool. (There is no `fetch` option: a sealed `GET` has a body.)                                                                                                                                                                                                                                 |

### Reaching vpay by an internal URL

Two things look like one and are not:

- **`baseUrl`** — where this process sends the HTTP request. It has to be
  reachable from _your_ server: a compose service name, a private DNS name, a
  service-mesh address.
- **`audience`** — what vpay calls _itself_. The signature binds
  `deployment.public_base_url` as the server was configured with it, and a
  server compares the string byte for byte. The URL you happened to POST to is
  never consulted.

They coincide for a merchant on the public internet, which is why `audience`
defaults to `baseUrl`. They do not coincide the moment your server reaches
vpay by an internal name, and then every request is vpay's coarse unsigned
`401` — the key, the signature and the `client_id` are all correct and the
response says nothing about audiences.

```ts
// The demo stack: the shop's container reaches vpay at the compose service
// name, but vpay's `deployment.public_base_url` is the published host port.
const vpay = new VpayClient({
  baseUrl: "http://vpay-server:8080",
  audience: "http://localhost:8080",
  clientId: "shop-merchant",
  privateKey,
  serverKeys,
});
```

## Error handling

Every error extends `VpayError`, so `catch (err) { if (err instanceof VpayError) ... }`
works as a catch-all; narrow further with `instanceof` on a specific subclass:

| Class                         | When                                                                                                                                                                                                                                                                                                            | Fields                                       |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| `VpayApiError`                | A non-2xx response from a `/v1` resource route.                                                                                                                                                                                                                                                                 | `status`, `type`, `code`, `message`, `param` |
| `VpayUnsignedResponseError`   | vpay refused the call before it was handled: the answer is not sealed (an unregistered or disabled key, a replayed message, the wrong audience — all the same bytes by design — or a proxy's error page). Never retried.                                                                                        | `status`                                     |
| `VpayUnverifiedResponseError` | The answer claims to be sealed and does not verify under `serverKeys` (a stale pin after a rotation, or the wrong deployment).                                                                                                                                                                                  | —                                            |
| `VpayUnexpectedResponseError` | A verified answer that isn't the shape this SDK expects — an empty body, or JSON that is not the Stripe-shaped error envelope on a non-2xx status.                                                                                                                                                              | `status`, `bodyPrefix` (first 500 bytes)     |
| `VpayTransportError`          | DNS, TLS, connection refused, timeout (the cause is a `TimeoutError` when the timeout fired).                                                                                                                                                                                                                   | `cause` (the underlying error)               |
| `VpayConfigError`             | The SDK was misconfigured — caught at construction where possible.                                                                                                                                                                                                                                              | —                                            |
| `VpayInvalidParamsError`      | A request parameter this SDK refuses to send, caught before anything is sent: an amount outside `0..=2^53-1`, a value the form encoder cannot put on the wire exactly (a non-safe-integer number, a `Date`, a `Map`), an empty or all-dots path `id`, a URL beside `outOfBand`. Not a `TypeError` (2026-10-21). | `param`, `message`                           |
| `WebhookSignatureError`       | `verifyWebhook` rejected a signature.                                                                                                                                                                                                                                                                           | —                                            |

```ts
import {
  VpayApiError,
  VpayError,
  VpayUnsignedResponseError,
} from "@vaam-apps/vpay-sdk";

try {
  await vpay.paymentIntents.create({
    amount: 5000,
    currency: "xaf",
    payment_method_types: ["mtn_momo"],
  });
} catch (err) {
  if (err instanceof VpayApiError) {
    console.error(err.status, err.type, err.code, err.message, err.param);
  } else if (err instanceof VpayUnsignedResponseError) {
    console.error("vpay refused the signature:", err.status);
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
import { verifyWebhook, WebhookSignatureError } from "@vaam-apps/vpay-sdk";

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
with a placeholder API key and a `config.httpClient` that seals each request
and opens the answer. `@vaam-apps/vpay-sdk/stripe` is that client:

```js
import { readFileSync } from "node:fs";
import Stripe from "stripe";
import { createStripeHttpClient } from "@vaam-apps/vpay-sdk/stripe";

const stripe = new Stripe("unused-placeholder", {
  httpClient: createStripeHttpClient({
    baseUrl: "http://localhost:8080",
    clientId: "acme-cameroon",
    privateKey: readFileSync("./merchant.seed", "utf8"),
    serverKeys: {
      current: readFileSync("./vpay-current.pub", "utf8").trim(),
      next: readFileSync("./vpay-next.pub", "utf8").trim(),
    },
  }),
  host: "localhost",
  port: "8080",
  protocol: "http",
  maxNetworkRetries: 0,
  telemetry: false,
});

const intent = await stripe.paymentIntents.create({
  amount: 5000,
  currency: "xaf",
  payment_method_types: ["mtn_momo"],
});
```

**Why `httpClient` and not `authenticator`.** The signature is the request
_body_; stripe-node's `authenticator` is a hook that can add headers and
nothing else. `httpClient` is the seam that is handed the finished request
(`host`, `port`, `path`, `method`, `headers`, `requestData`, `protocol`,
`timeout`) and returns a response. The client seals it, sends it, opens the
answer, and returns the verified JSON as the response body, so everything above
it — resource methods, typed results, `Stripe.errors.*` — behaves as it does
against Stripe.

`host`, `port` and `protocol` are not optional. The client is bound to
`baseUrl`'s origin: a request addressed anywhere else throws `VpayConfigError`
before anything is signed. Omit them and stripe-node addresses
`api.stripe.com:443`, which is exactly the case this refusal exists to stop.

The first argument is a placeholder stripe-node insists on and that is never
sent: the client forwards **only** the `Idempotency-Key` of what stripe-node
set (`Authorization`, `Stripe-Version`, `X-Stripe-Client-User-Agent` and the
rest are dropped; the signature binds the payload type itself). stripe-node
only keys POSTs, and vpay requires a key on every write, so a `DELETE` is given
a fresh one.

`stripe` is an **optional peer dependency** of this package: install it
yourself if you want this entry point, and `@vaam-apps/vpay-sdk`'s core entry
never imports it.

### When vpay refuses

stripe-node retries every _rejection_ from an `httpClient` (`maxNetworkRetries`
defaults to **2** on Node) and its public API cannot exempt one, so a refusal is
not a rejection. A refused call (the unsigned `401`), an answer that does not
verify and a body over 8 MiB are returned as Stripe-shaped error responses with
`stripe-should-retry: false` and surface **once**: `StripeAuthenticationError`
(`code: "unsigned_response"`) for the `401`, `StripeAPIError`
(`"unverified_response"`, `"response_too_large"`) otherwise, the cause in
`message`. A gateway's unsigned `5xx` stays retryable.

Two more are answered the same way, and neither can resolve as a result or be
retried. A request the contract has no operation for (a `GET` on a `POST`-only
path, a path outside `/v1`) is never sent: `StripeInvalidRequestError`, status
`404`, `code: "unknown_operation"`. A sealed `4xx`/`5xx` that is not in the
Stripe error envelope (the body bound's `413` is the one you can meet) is an
error with its status and `code: "unexpected_error_response"`, or
`"request_too_large"` for the `413` (a `StripeAPIError`, as stripe-node has no
class for it); the `413` carries vpay's `request-id`.

A real transport failure (DNS, TLS, a reset, a timeout) still rejects, is a
`StripeConnectionError` with the original at `err.detail`, and is retried up to
`maxNetworkRetries` times, re-sealed each time. A `POST` keeps the
`Idempotency-Key` stripe-node chose; a `DELETE` has none from stripe-node and
gets a new one per attempt, so set `maxNetworkRetries: 0` if a repeated `DELETE`
answering `404` is not acceptable.

### Where vpay diverges from Stripe

- **No API keys, ever.** `sk_live_…` means nothing here; authentication is a signed
  message under your Ed25519 key
  ([ADR-0035](../../docs/adr/0035-v1-is-a-signed-message-not-a-token.md)). There is
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
  `@vaam-apps/vpay-stripe-js`'s browser-side confirm flow. `amount_received`,
  `capture_method` and `confirmation_method` stay genuinely absent, though:
  stripe-node's TypeScript types declare all three as present on every
  response shape — it casts responses, it never validates them — so these
  read as `undefined` at runtime while the type says otherwise, on every
  route including `create`/`retrieve`.
- **`Stripe-Version`, `Stripe-Account`, `Stripe-Context` and the
  `X-Stripe-Client-*` headers are dropped before sealing.** vpay advertises no
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

### Status

`createStripeHttpClient` is **built and tested**. Its tests run against a real
`node:http` server that opens the sealed request under the merchant's key and
seals its answer (cratestack's own server envelope, built from
`sdks/rust/wasm`'s `responder` feature) and drive the **real `stripe`
package**: `new Stripe("…", { httpClient, host, port, protocol })` creating,
listing, retrieving and deleting. The type-level assertion that the returned
client is assignable to `Stripe.StripeConfig["httpClient"]` is checked by
`pnpm --filter @vaam-apps/vpay-sdk typecheck`.

What is **not** proven here:

- **Nothing in this package has run against a real `vpay-server`** from its
  own tests. What _has_ is [`sdks/stripe-compat`](../stripe-compat/) — the
  official `stripe` package driven through this client against a live compose
  stack, run by CI's `e2e (compose)` job. It is a separate package on purpose:
  it needs a stack, and a suite that needs a stack must not be able to report
  green in a job that has none.
- **Every webhook verified so far was delivered to a WireMock receiver on a
  compose network.** The _signature_ is proven three ways — against this
  package's own `verifyWebhook`, against `vpay_sdk::webhooks::verify_at`, and
  against the official `stripe` package's `constructEvent` — but **no merchant
  endpoint has ever been POSTed to**.
- **The `stripe` peer range (`^22.6.1`) is asserted, not derived.** It is the
  current major at the time of writing, tested against exactly `22.6.1`.
- **The server-side compatibility work is not in this package**, though it
  lives beside it: the `request-id` response header stripe-node reads,
  `stripe-should-retry` derived from `Classify::retry`, and refusing
  `confirm: true` on create rather than ignoring it are all in `vpay-api`. See
  [`docs/flows/stripe-sdk-compat.md`](../../docs/flows/stripe-sdk-compat.md).

## `scripts/vpay-call.mjs`

One signed `/v1` call from a shell — what `curl` cannot do, because the request
is a COSE message and not a header on an ordinary one. The path is under `/v1`;
the verified JSON goes to stdout, the HTTP status to stderr, and the exit code is
0 for a 2xx:

```bash
pnpm --filter @vaam-apps/vpay-sdk build
export VPAY_BASE_URL=http://localhost:8080
export VPAY_MERCHANT_SIGNING_SEED_PATH=.e2e/demo-merchant/signing.seed
export VPAY_SERVER_KEY_CURRENT_PATH=.e2e/envelope/current.pub
node sdks/nodejs/scripts/vpay-call.mjs GET /payment_intents --query 'limit=3'
node sdks/nodejs/scripts/vpay-call.mjs POST /payment_intents \
  --data 'amount=5000&currency=xaf&payment_method_types[0]=mtn_momo' --key order-42
```

## Status

**Breaking, 2026-10-26 (VP-7): `/v1` is signed, and this package signs it.**
The `private_key_jwt` assertion, the token exchange, the token cache and the
401 re-authentication retry are gone, with `mintClientAssertion`,
`VpayAuthError`, `scripts/mint-assertion.mjs`, the `kid`, `issuer`,
`tokenEndpoint`, `assertionAudience`, `audience` (now the signature's
audience, a different thing), `scope`, `assertionLifetimeSeconds` and `fetch`
options, and `createStripeAuthenticator` (replaced by `createStripeHttpClient`,
for the reason given above). A client now needs an Ed25519 key and
`serverKeys`; RSA keys are refused at construction. New errors:
`VpayUnsignedResponseError`, `VpayUnverifiedResponseError`. The requests go out
on `node:http(s)` rather than `fetch` (a sealed `GET` has a body), and the
package needs `sdks/rust/wasm` compiled at build time. Nothing about a
resource method's signature, a request's form body or a response's shape
changed. `balance.retrieve()` is now refused locally: no route serves it and
the operation table has no row to bind (it answered `404 unknown_route`
before).

**Breaking, 2026-10-21: every local refusal is a `VpayInvalidParamsError`,
not a `TypeError`.** An amount outside `0..=2^53-1`, a value the form
encoder cannot put on the wire exactly, an empty or all-dots path `id` and a
URL beside `outOfBand` on `invoices.pay` now reject with the new exported
`VpayInvalidParamsError`. It extends `VpayError`, so the README's
`instanceof VpayError` catch-all finally catches it, and its `param` names
the parameter, like `sdks/rust`'s `Error::InvalidParams { param, .. }`. Its
`message` is that variant's `Display` text,
`invalid request parameter {param}: {detail}`. **A caller that tested
`instanceof TypeError` for these no longer matches**, and cannot be made to:
a class has one superclass, and making `VpayError` a `TypeError` would make
every error here one, a network failure included. Switch to
`instanceof VpayInvalidParamsError`. No signature changed and nothing that
was sent before is refused now.

**Behaviour change, 2026-10-21: an empty or all-dots `id` is refused before
any request.** Every one of the twenty-one methods that put an `id` in the
path now rejects with a `TypeError` for `""`, `.`, `..` or any longer run of
dots, without minting a token or sending anything. No signature changed —
each already returned a `Promise` — but the outcome for those inputs did.
They used to be sent, and `fetch`'s URL parser resolved them before the
request left, so `customers.del("..")` was a `DELETE /v1/` and
`paymentIntents.retrieve("")` a `GET /v1/payment_intents/`; escaping cannot
help, because the parser reads `%2E%2E` as `..` too. What vpay answered for
each is on
[the dated verification page](../../docs/status/verification/2026-10-21-sdk-dot-segment-ids.md).
A caller catching `VpayApiError` for those inputs now gets a `TypeError`,
and so does one narrowing on `VpayError`: like the amount refusal, it is not
one (a dated gap in [docs/sdks/parity.md](../../docs/sdks/parity.md)).
`sdks/rust` refuses the same ids with `Error::InvalidParams`. _(Corrected
later the same day: the refusal is a `VpayInvalidParamsError` now, so a
`VpayError` narrowing catches it and the gap is closed — the paragraph
above.)_

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
through `paymentIntents.retrieve` and nothing else. `/v1/events`,
`/v1/events/{id}` (Step 5) and `GET /v1/refunds/{id}` (issue #45,
2026-09-05) are served, and so are `/v1/customers` (S4a) and — since
2026-09-07 — `/v1/invoices` and `/v1/invoice_items` (S4b), which this
package has wrapped since 2026-09-08. `POST /v1/refunds` and `/v1/balance`
are still the honest `404 unknown_route`. ~~**This package has no client method for any of
that beyond the payment-intent routes it already wraps.**~~ **— corrected
2026-09-05: it has `refunds.retrieve(id)`,** added in the same change that
served the route, so this package can now make the refund read a
reconciliation job needs when a `charge.refund.updated` was missed or
arrived out of order. It still has no `events.retrieve` — a dated gap in
[`../../docs/sdks/parity.md`](../../docs/sdks/parity.md) — and
`refunds.create` and `balance.retrieve` still reach the 404.

**`checkout.sessions` is newer than this paragraph and less proven than the
rest.** The four methods are implemented and tested against this package's
`node:http` stub, in the shape of Step 9's wire contract. The routes they
call — `/v1/checkout/sessions` and its three siblings — are built by lane 1
of the same step; until that lands and
`backends/tests/integration/tests/checkout_sessions.rs` is green, "the stub
answers the way this SDK expects" is the whole of the evidence, and a
mismatch would show up as a `VpayApiError` rather than as anything this
package could catch earlier.

**No test in _this package_ has ever talked to a vpay** — every server in
these tests is a `node:http` stub started by the test. What has talked to a
real one is `sdks/stripe-compat`, which drives the official `stripe` package
through this package's own `createStripeHttpClient` against a live compose
stack (CI's `e2e (compose)` job), and the Rust SDK's
`backends/tests/integration/tests/customers.rs` and siblings (the real
`EnvelopeLayer`). See
[`docs/status.md`](../../docs/status.md),
[`docs/flows/merchant-auth.md`](../../docs/flows/merchant-auth.md) and
[`docs/flows/stripe-sdk-compat.md`](../../docs/flows/stripe-sdk-compat.md).

What the tests in this package **do** prove. Everything that touches HTTP
runs against a real `node:http` server started by the test (never a mocked
`fetch`) and asserts the actual bytes sent on the wire; the form encoder,
the webhook verifier and the error-body bounding are pure unit tests, since
none of them touches the network:

- What a signature binds, driven straight at the sealing module with
  cratestack's server envelope on the other end (`src/sealer.test.ts`): an
  untouched message opens; the same bytes a second time are refused (a replay);
  a changed byte, another audience, another route, another path parameter,
  another method, another query (and the same pairs in another order are
  _accepted_: the query is canonical), another `Idempotency-Key` and a key the
  server does not hold are each refused; an operation no table row serves is
  refused before anything is signed; an answer opens only against its own
  request and status, and only once.
- Signing end to end through `VpayClient` and a real `node:http` server that
  verifies the way vpay does: a seed, a PEM, a `KeyObject` and a callback signer
  all sign (and a callback signature that does not verify is refused with the
  cause); an RSA key, a doubled or missing key, a malformed key (never quoted)
  and a missing `serverKeys` are construction errors; the pinned `next` key
  verifies an answer; a wrong pin is `VpayUnverifiedResponseError`; an
  unregistered key and a wrong audience are `VpayUnsignedResponseError`; a
  sealed request has no `Authorization` header and is the body of even a `GET`.
- Every resource method's exact HTTP method, path, headers, and
  bracket-encoded body — including that a caller-supplied `Idempotency-Key`
  is honoured and one is generated when omitted.
- That `create()` and `retrieve()` surface `client_secret` typed (`string`)
  when the server sends it, and that a `list()` item without one reads back
  `undefined` rather than a cast or a runtime crash.
- All four `checkout.sessions` operations: the exact path, method,
  `Idempotency-Key` and body of `create` (including that an unset `ui_mode`
  or URL is omitted rather than sent empty), the `GET` paths of `retrieve`
  and `list` with its `payment_intent` filter, `expire`'s empty-bodied
  `POST`, percent-encoding of a hostile id on both, and the `404`/`409`
  envelopes. The body strings are the ones `sdks/rust/tests/resources.rs`
  pins byte for byte, so the two SDKs cannot drift.
- That a `CheckoutSession`'s `client_secret` — and the fragment of a hosted
  session's `url`, which carries the same value — are redacted from
  `util.inspect` while `JSON.stringify` stays faithful.
- The form encoder's nested-object/array/boolean/percent-encoding rules, and
  that a non-integer `amount` throws `VpayInvalidParamsError` before any
  request is sent.
- That every kind of local refusal — an amount, an id, an out-of-band URL, a
  `Date` in `metadata` — is a `VpayInvalidParamsError` naming its parameter,
  caught by `instanceof VpayError` and not a `TypeError`, with nothing sent.
- That an empty or all-dots `id` throws `VpayInvalidParamsError` on all twenty-one
  id-taking methods before any request, with one
  fixed message that never quotes it; that an id with dots in it that is not
  only dots (`a..b`, `.a`, `%2E%2E`) is still sent as exactly one segment;
  and, as the premise, that WHATWG `URL` resolves `..`, `%2E%2E` and `.%2e`
  alike.
- Error mapping for a Stripe-shaped `400`, a sealed answer that is not the
  error envelope, a proxy's unsigned `502` HTML page, and a connection failure.
- Webhook verification: valid/invalid signatures, timestamp tolerance,
  multi-signature secret rotation, malformed headers, a one-byte body change,
  and that a `Buffer` body and its string form verify identically.
- That `util.inspect(client)` and `JSON.stringify(client)` never contain the
  signing seed or a PEM.
- All thirteen invoice operations (S4b, added 2026-09-08): the exact body of
  `invoices.create` and of a create that supplies nothing but the two required
  fields (`customer` and `currency` — the second was optional for one day, and
  a running vpay answers `400` without it); the `GET`
  whose response carries the lines; the update's three states (`due_date=` is
  _clear it_, an unmentioned field is absent, and a patch that mentions
  nothing sends nothing); the list's two cursors and two filters; the
  `DELETE` with its key and no body; that `finalize`, `void` and
  `markUncollectible` each `POST` an **empty** body to its own path while
  still sending a content type (the third method is camelCase and its route,
  `/v1/invoices/{id}/mark_uncollectible`, is not — the case asserts the path
  by name); that a refused transition arrives as a
  `VpayApiError` carrying the server's `409` whole; that `invoices.pay` sends
  both URLs and answers an invoice still `"open"` with a `payment_intent` and
  a `hosted_invoice_url`; the four `invoiceItems` methods, including that
  neither `currency` nor `amount` is ever in the body; and that a
  `unit_amount` outside the safe-integer range throws `VpayInvalidParamsError`
  before any request. Every body string is the one `sdks/rust/tests/resources.rs` pins,
  asserted as a literal on both sides.
- **Against a real `vpay-server`, not a stub** (added 2026-09-08):
  `src/invoices.live.test.ts` creates a customer, drafts an invoice, bills two
  lines, finalizes it, reads it back, checks the stored lines add up to the
  stored total, lists it, voids it and finds it keeps its number; then
  finalizes a second and has `pay` mint a payment intent and a hosted URL
  without charging anybody; and confirms `currency` is required. It is a
  **separate vitest project** (`vitest.live.config.ts`, `pnpm test:live`) so
  `pnpm test` stays runnable with no stack, and it **fails rather than
  skipping** when there is none — `just sdk-live` is what brings one up.
- That the four `invoice.*` types are in `KnownEventType` and narrow through
  `isInvoiceEvent`, that `isPaymentIntentEvent` and `isCheckoutSessionEvent`
  do **not** claim them, and that an event body's empty `lines` decodes
  through the same `Invoice` type a `/v1` response's populated `lines` does.

**There is no separate conformance check, and that is the point.** The old
`just sdk-conformance-node` fed an assertion this package minted to the real
OP verifier, because two implementations had to be shown to agree. The signing
here is the Rust SDK's own code (`sdks/rust/wasm`), and the module's server half
that these tests open requests with is cratestack's own server envelope, so
there is one implementation to check, and `sdks/rust/tests/signing.rs` and the
integration suites check it against the real `EnvelopeLayer`.

What this does **not** prove: that any of this works against a real vpay
deployment. ~~Nothing in this package has been run against `vpay-server`; the
`issuer`/`tokenEndpoint` defaults agree with `vpay_api::op::issuer_for` by
inspection, not by a test in this repository that exercises them together,
and the resource routes every method here calls do not exist server-side at
all~~ — see [`docs/status.md`](../../docs/status.md).

_(Struck 2026-09-23. All three clauses had gone stale. **Run against
`vpay-server`:** since 2026-09-10, `src/invoices.live.test.ts`, joined by
`src/refunds.live.test.ts` on 2026-09-16, drives a real `vpay-server` over a
socket, and CI's `e2e` job runs them (`pnpm --filter @vaam-apps/vpay-sdk
test:live`). **The defaults:** those suites' preflight
(`src/testing/live-preflight.ts`) builds its client from the live environment's `baseUrl` and keys, so
every live run exercises the default `audience` against the server's own
configured name. **The routes:** every method's route is mounted except
`balance.retrieve`, which answers `404` by design
([`docs/sdks/parity.md`](../../docs/sdks/parity.md)). What still stands is the
sentence this paragraph opens with. That stack is a compose stack with
WireMock rails, not a deployment.)_
