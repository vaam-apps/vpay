# @vpay/sdk

The Node.js merchant SDK for vpay's `/v1` API. Implements the wire contract in
[`docs/flows/merchant-auth.md`](../../docs/flows/merchant-auth.md) exactly —
`private_key_jwt` client assertions, `client_credentials` token exchange and
caching, the form-encoded resource calls, and outbound-webhook verification.

**This package is `private: true` and is not published.** Publishing waits for
a server that actually serves `/v1` — see "Status" below.

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

## Configuration

| Option                     | Default               | Notes                                                                                                                                                                                                                       |
| -------------------------- | --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `baseUrl`                  | —                     | Required, e.g. `https://api.vpay.example`.                                                                                                                                                                                  |
| `clientId`                 | —                     | Required. Your registered OAuth2 `client_id`.                                                                                                                                                                               |
| `privateKey`               | —                     | Required. PEM text or a `crypto.KeyObject`. Never logged, never serialized.                                                                                                                                                 |
| `kid`                      | —                     | Required only if you registered more than one JWK.                                                                                                                                                                          |
| `issuer`                   | `${baseUrl}/v1/oauth` | This default is what the server does: `vpay_api::op::issuer_for` builds the issuer as `{public_base_url}/v1/oauth` from the deployment YAML, and `vpay_api::router` mounts the OP there. Override only for a deployment behind a path prefix. |
| `tokenEndpoint`            | `${issuer}/token`     | The server's token route, and also the assertion's `aud` claim.                                                                                                                                                             |
| `audience`                 | `vpay:v1`             | The OAuth2 `audience` request parameter. Load-bearing: without it the OP mints a token whose `aud` is the `client_id`, which the resource server rejects. Server-side the same string is `vpay_config::MERCHANT_AUDIENCE`; this package keeps its own copy, so the two must change together. |
| `scope`                    | —                     | Omitted from the token request unless set.                                                                                                                                                                                  |
| `assertionLifetimeSeconds` | `60`                  | Must be an integer in `1..=300`; anything else throws `VpayConfigError` at construction, not at request time. Keep the default — see the note below the table.                                                              |
| `timeoutMs`                | `30000`               | Applies to both the token exchange and every resource call.                                                                                                                                                                 |
| `fetch`                    | global `fetch`        | Injection point for tests or an outbound proxy.                                                                                                                                                                             |

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
is a *field*, not a subclass. Branch on `code`, which is what the server
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

**Half of the server side of this contract exists.** `vpay-server` mounts the
merchant OP — `POST /v1/oauth/token`, `GET /v1/oauth/jwks.json`,
`GET /v1/oauth/.well-known/openid-configuration` — and puts a bearer-token
boundary in front of every other `/v1` path. What it does **not** have is a
single `/v1` resource route: past the boundary, `payment_intents`, `refunds`,
`events` and `balance` all answer a Stripe-shaped `404 unknown_route`. So the
authentication half of this SDK has a real server to talk to and the resource
half does not, and no test in this package has ever talked to either — the
end-to-end proof that exists is the Rust SDK's
(`backends/tests/integration/tests/merchant_token_flow.rs`), not this one's.
See [`docs/status.md`](../../docs/status.md) and
[`docs/flows/merchant-auth.md`](../../docs/flows/merchant-auth.md).

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
