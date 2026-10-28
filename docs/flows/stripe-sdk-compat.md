# Using the official Stripe SDKs against vpay

vpay's `/v1` object model, form encoding, error envelope and idempotency
semantics are Stripe's. Its **authentication is not**: there is no API key, and the SDKs send no
token. Every call is a signed message and every answer is a signed message
([signed-transport.md](signed-transport.md),
[ADR-0035](../adr/0035-v1-is-a-signed-message-not-a-token.md)).

`stripe-node` accepts an arbitrary `config.httpClient`: the object that turns
one outbound request into one response. That is the one seam a signed
transport needs, because **the signature is the request body** and the answer
is a signed body to be opened before stripe-node reads it. An `authenticator`
(the seam this page used when authentication was a bearer token) can only add a
header, so it can no longer carry vpay's authentication. This document is what
does and does not carry over once the `httpClient` is filled.

## The client

```js
import { readFileSync } from "node:fs";
import Stripe from "stripe";
import { createStripeHttpClient } from "@vaam-apps/vpay-sdk/stripe";

const httpClient = createStripeHttpClient({
  baseUrl: "https://api.vpay.example",
  clientId: "acme-cameroon",
  privateKey: readFileSync("./merchant.seed", "utf8"), // an Ed25519 seed
  serverKeys: {
    current: process.env.VPAY_SERVER_KEY_CURRENT, // vpay's response keys,
    next: process.env.VPAY_SERVER_KEY_NEXT, // pinned
  },
});

const stripe = new Stripe("unused", {
  httpClient,
  host: "api.vpay.example",
  port: "443",
  protocol: "https",
  maxNetworkRetries: 0,
  timeout: 30_000,
  telemetry: false,
});
```

The key string is **never sent**: stripe-node requires a non-empty one when an
`httpClient` is given, and the client ignores the `Authorization` header it
would have built. `host`/`port`/`protocol` move every request off
`api.stripe.com`, and they are not optional garnish: the client seals only for
the origin in `baseUrl` and refuses (signing nothing) a request addressed
anywhere else, so a mis-pointed `Stripe` object cannot put a message signed for
vpay into Stripe's logs. `basePath` is fixed at `/v1/` and is not configurable
— which is moot, because the generated resources use absolute paths
(`/v1/payment_intents`, `/v1/payment_intents/{id}/confirm`) and those are
exactly vpay's, and the operation table refuses a path or a verb `/v1` does not
serve before it is signed.

Transport is `node:http(s)`, not `fetch`, because a sealed `GET` carries its
signature as a body and `fetch` forbids one. Sealing is the Rust code compiled
to WebAssembly (`sdks/rust/wasm`), so the bytes signed are the Rust SDK's.

**A refusal reaches you as a Stripe error, once.** vpay's coarse unsigned
`401` is not the Stripe envelope, and an answer sealed under a key you did not
pin is not an answer at all. If the client rejected, stripe-node would report a
connection error and **retry it** (`maxNetworkRetries` defaults to 2 on Node,
and a rejection cannot be exempted), sending a refused, freshly signed message
three times. So a refusal is returned as a Stripe-shaped response with
`stripe-should-retry: false`: `StripeAuthenticationError` with `code:
"unsigned_response"` for the `401`, `StripeAPIError` with `code:
"unverified_response"` for an answer that does not verify under the pinned
keys (or `"response_too_large"` for a body over 8 MiB), the cause in `message`.
A gateway's own unsigned `5xx` stays retryable.

What still rejects, and is a `StripeConnectionError` retried up to
`maxNetworkRetries`, is a real transport failure. Each retry re-seals the call
(a new message). A `POST` keeps the `Idempotency-Key` stripe-node chose, so the
retry is the same operation. A `DELETE` has none from stripe-node and gets a
new one per attempt, so a repeat of a `DELETE` that had landed answers `404`:
set `maxNetworkRetries: 0` if that is not acceptable.

## What carries over unchanged

|                                                  | Why                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The resource paths                               | `/v1/payment_intents`, `/{id}`, `/{id}/confirm`, `/{id}/cancel` are the four vpay serves, and stripe-node hardcodes exactly those strings                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Form encoding, including nested and indexed keys | stripe-node percent-encodes and then decodes brackets back, so a space is `%20` and a literal `+` is `%2B` — which is what `vpay_api::form` requires. It serialises arrays **indexed** (`expand[0]=…`, `payment_method_types[0]=…`), and that spelling is proven end to end by `sdks/stripe-compat`'s "accepts and ignores `expand`, which stripe-node encodes as `expand[0]`" case. The bare `expand[]=…` spelling `vpay_api::form` also accepts is **not** exercised by this suite — stripe-node never emits it; its evidence is that decoder's own unit test |
| `Idempotency-Key`                                | stripe-node generates one for **every** v1 POST, unconditionally, "including when `maxNetworkRetries` is 0". vpay _requires_ one on every `/v1` POST — stricter than Stripe — and that costs a stripe-node user nothing                                                                                                                                                                                                                                                                                                                                         |
| The list envelope and auto-pagination            | `autoPagingToArray` needs only `data[].id` and `has_more`; `ListObject` supplies both plus `url`                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| The error envelope                               | `{error: {type, code, message, param?}}`, with `type` from the same closed vocabulary                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `webhooks.constructEvent`                        | vpay's signature construction is byte-identical to Stripe's — see "Webhooks" below for what is and is not built                                                                                                                                                                                                                                                                                                                                                                                                                                                 |

## Error mapping

stripe-node picks the error **class from the status code first**, and consults
`type` only inside 400/404. vpay derives status, `type` and `code` from one
classification ([ADR-0011](../adr/0011-error-modelling.md)), so the mapping
below is a property of the two designs meeting rather than of anything written
to make it line up.

| vpay answer                | What you catch                                                                                                                |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `404` `resource_missing`   | `StripeInvalidRequestError`, `err.code === "resource_missing"`                                                                |
| `400` `invalid_request`    | `StripeInvalidRequestError`, with `err.param` naming the field                                                                |
| `400` `idempotency_error`  | `StripeIdempotencyError`                                                                                                      |
| the coarse unsigned `401`  | `StripeAuthenticationError`, `code: "unsigned_response"`, once — it is not the envelope, and vpay does not say why it refused |
| `403` (missing scope)      | `StripePermissionError` — **despite** carrying `type: invalid_request_error`, because stripe-node branches on the status      |
| `409` (lifecycle conflict) | `StripeAPIError` — 409 falls through every branch of `generateV1Error`                                                        |
| `502` (rail transport)     | `StripeAPIError`, **and stripe-node retries it** — see below                                                                  |
| `429`                      | never emitted: nothing in `backends/crates` constructs `Category::RateLimited`                                                |

`err.requestId` is populated from a `request-id` response header — stripe-node
never reads `x-request-id`. vpay emits **both names with one value**, so
`Category::Internal`'s "contact support with the request id" is a promise a
Stripe SDK user can act on.

### `stripe-should-retry`

vpay sets this header on every response its error renderer produces, derived
from `Classify::retry`. stripe-node consults it **above** its own status
rules, and vpay needs both directions:

- `409` gets `false`. stripe-node retries every 409 unconditionally; a
  lifecycle refusal ("this intent is already `processing`") is not something
  waiting fixes.
- `IdempotencyKeyInFlight`'s `400` gets `true`. stripe-node retries no 4xx; a
  key still in flight is the one refusal on this surface that clears itself.

**A replayed response carries the advisory the original carried.** Migration
`0025` adds `idempotency_keys.response_retry`, a `TEXT` column holding the
header's own two values; `PostRequest::finish` reads the value off the
response it is about to store — the rendered `HeaderMap`, not the status — and
`replay` writes those bytes back. A stored `2xx` has `NULL` there and its
replay emits no header, because a `2xx` never passed through the error
renderer in the first place.

The fix deliberately _not_ taken was re-deriving the advisory from the stored
status at replay time: ADR-0011 makes one classification the source of status
_and_ retry, and a second derivation running the other way round is exactly
the drift it exists to prevent. Storing the header's text rather than a
`BOOLEAN` follows from the same rule — a boolean would put a second
`bool → "true"/"false"` rendering in the replay path, where the column's job
is to record what was sent. The domain is bought back by a CHECK
(`response_retry_is_an_advisory`), proven firing by
`the_retry_advisory_round_trips_and_0025_refuses_anything_else` in
`vpay-db`'s repository suite.

Two tests pin the behaviour, and both used to assert the opposite:
`a_replayed_response_carries_the_advisory_it_was_stored_with` (a `vpay-api`
unit test — it asserts the replayed value _equals_ what the same error renders
fresh, so a hard-coded `false` in `replay` fails it) and
`a_replayed_error_carries_the_same_retry_advisory_the_original_did` (an
integration test in `backends/tests/integration`, against a real Postgres and
a real replay reached through the real "one charge per intent" refusal).

### Two requests that never get an envelope from the server

`405` (a route asked with the wrong method) and `413` (a body past 64 KiB) are
not rendered by `ApiError::into_response`: the first is axum's own, with an
empty body, and the second is answered by `RequestBodyLimitLayer`, in front of
the renderer. Neither carries the Stripe envelope or `stripe-should-retry`.

Before the signed transport, stripe-node met that non-JSON body by discarding
everything it knew about the response and throwing `StripeAPIError: Invalid JSON
received from the Stripe API`, with `statusCode` and `headers` both `undefined`,
so a merchant could not tell a 405 from a 413 from a proxy's HTML 502. Under
the signed transport the two end differently, and both are **measured against a
live stack with stripe-node's default retries on**
(`sdks/stripe-compat/src/errors.compat.test.ts`):

- **A method the contract has no route for never reaches the server.** The
  sealing `httpClient` binds every request to an operation in the contract
  table before it signs anything, so a `GET` on a `POST`-only path has no
  operation to bind to and is refused on the client, unsent and unsigned. It
  resolves as a `404` `StripeInvalidRequestError`, `code: "unknown_operation"`,
  `stripe-should-retry: false`, once. It used to _reject_, which stripe-node
  reports as a `StripeConnectionError` and retries twice. There is no
  `requestId`: no exchange took place. A `405` from the server is unreachable
  through this client.
- **A body past the limit is a sealed `413` outside the Stripe envelope.** The
  signed transport seals the limit layer's answer as it finds it, in
  cratestack's `{ code, message, details }`, and stripe-node reads an error only
  from an `error` key. So unchanged, the call _resolved_: the merchant's
  `await paymentIntents.create(...)` returned `{ code: "INTERNAL_ERROR", ... }`
  as though it were an intent. The client now re-presents any verified `4xx`/`5xx`
  without an `error` object as an error: for a `413`, `StripeAPIError`
  (stripe-node picks the class from the status and has none for 413),
  `statusCode: 413`, `code: "request_too_large"`, `stripe-should-retry: false`,
  with the server's `request-id` in `requestId`, which survives and is what a
  merchant quotes to support. Any other such status becomes
  `code: "unexpected_error_response"` (retryable only for a `5xx`).

A Stripe-shaped `413` rendered by the server itself would still be better than
a client that translates one; none exists.

### A `502` from the rail is re-POSTed

`Category::Rail` renders as `502`, which stripe-node retries unconditionally
(and vpay's `Retry::AfterBackoff` for that category sets
`stripe-should-retry: true`, agreeing with it). So a `confirm` the rail did
not answer is sent again, under the same `Idempotency-Key`.

That is the correct thing to want and it is not what currently happens: the
retry meets vpay's "one charge per intent, forever" rule and comes back as a
`409`. **Not covered by any test** — the compose stack's WireMock rails cannot
be steered into a transport failure from outside, because the reference the
adapter sends is minted server-side. It is stated here because it is the
predictable consequence of the header, not because anything observed it.

## Divergences a Stripe integration will hit

- **No API keys.** `apiKey`, `stripeAccount`, Connect: none of them mean
  anything. `Stripe-Version`, `Stripe-Account`, `Stripe-Context` and the
  `X-Stripe-Client-*` headers are accepted and ignored, and a `Stripe-Account`
  is deliberately _not_ a 400 — a documented "Connect is not a thing here" is
  a better diagnostic.
- **No dated API version.** vpay advertises none and echoes none, so
  `obj.lastResponse.apiVersion` is `undefined` and pinning `apiVersion` has no
  effect.
- **`payment_method_types` is required and non-empty**, and each entry must
  name a rail this deployment has enabled. A copied
  `automatic_payment_methods: { enabled: true }` snippet is silently dropped
  (no `deny_unknown_fields`) and the request is then refused for the missing
  required field, naming it.
- **`confirm: true` on create is refused**, with `param: "confirm"` and a
  message naming `POST /v1/payment_intents/{id}/confirm`. It used to be
  dropped silently, which left a merchant believing they had charged someone.
  `confirm=false` is accepted, because it asks for exactly what the endpoint
  does.
- **`payment_method_data.type` is a rail code**, e.g. `mtn_momo`, with the
  instrument under a key of the same name
  (`payment_method_data[mtn_momo][msisdn]`). TypeScript users need a cast:
  stripe-node's generated types know Stripe's methods, not vpay's rails.
- **The fields that decide where or when money moves are refused, not
  ignored**, with a `400` naming the field in `error.param`: `capture_method`
  with any value other than `automatic`, `application_fee_amount`,
  `transfer_data` and `on_behalf_of`. vpay has no authorise-now /
  capture-later split (confirming _is_ the charge) and no Connect, so ignoring
  any of them would settle a merchant's money at a time, or to an account,
  they did not ask for and could not see in the response. Both POST bodies
  carry the same refusal set — create and confirm.
- **Everything else Stripe sends and vpay does not implement is accepted and
  ignored**, and that is the default rather than the exception: neither
  `CreateParams` nor `ConfirmParams` has `deny_unknown_fields`, because a
  `400` per field a Stripe SDK adds of its own accord would make vpay
  unusable from the SDKs this work exists to support. `setup_future_usage`,
  `confirmation_method`, `receipt_email`, `statement_descriptor`, `customer`,
  `expand` and `metadata` all leave the payment exactly as requested
  (`metadata` and `customer` are stored; the rest are dropped). _(This said
  "`metadata` is stored; the rest are dropped" until 2026-09-23. `customer`
  has been stored and rendered on the intent since 2026-09-06 (S4a,
  `0fc5dcf`, migration `0034`), and a `cus_…` that is not this merchant's is a
  `400` naming `customer` — so it is no longer merely accepted either; see
  [customers.md](customers.md).)_
- **`expand` is ignored**, not refused. Nothing is expandable, so the response
  simply has no expanded field — an absence visible in the response itself,
  which is the line between this list's two halves.
- **`client_secret` is present on `create` and `retrieve` only, as of Step
  5c** (`PaymentIntentWithSecret`, decision D2,
  [browser-checkout.md](browser-checkout.md)) — this line used to say it was
  absent everywhere, which was true before that step and is no longer true
  for these two methods. It stays absent from `confirm`, `cancel`, `list`
  and every webhook body, which render the same 12-key object every other
  reader sees. `amount_received`, `capture_method` and `confirmation_method`
  remain genuinely absent, although stripe-node's types declare them
  present — a type-level lie with no runtime effect,
  `StripeResource._makeRequest` casts and never validates. `client_secret`
  now exists for a real reason: `@vaam-apps/vpay-stripe-js`'s payer-facing
  confirmation flow, not stripe-node's (which has no client-side
  confirmation step of its own and never reads this field).
- **`next_action` is only ever `redirect_to_url`**, and only on a redirect
  rail. A push rail leaves it `null`.
- **Currencies are XAF and EUR**, integer minor units, and a confirm whose
  intent currency is not the rail's settlement currency is a 400.
- **There is no `failed` status.** A refused charge returns the intent to
  `requires_payment_method` with `last_payment_error` set.
- **`search` and `/v1/balance`** are not routed and answer the honest
  `404 unknown_route`. `POST /v1/refunds` was in that list until
  **2026-09-16**, when RFC-0003 § 2 mounted it along with
  `POST /v1/refunds/{id}` (metadata only, as Stripe), `GET /v1/refunds` and
  `POST /v1/refunds/{id}/cancel`; `GET /v1/refunds/{id}` has been routed
  since 2026-09-05 (issue #45). All five answer Stripe-shaped bodies — but
  **nothing here drives any of them through stripe-node's `refunds`
  resource**, so `stripe.refunds.retrieve()` and `stripe.refunds.create()`
  working is untested rather than known, exactly as `stripe.events.list()` is
  below.

  **Two divergences a Stripe-shaped client will meet on the create**, both
  deliberate and neither of them Stripe's:

  - `destination[<rail code>][msisdn]` is **required** on `mtn_momo` and
    `orange_money`, because a refund there is an outbound transfer and needs
    a payee (RFC-0003 § 1); `notchpay` refunds to the instrument that paid
    and refuses one. Since 2026-10-24 the key may instead be the
    payment-method type the charge records (`destination[mtn_wallet][msisdn]`). Stripe has no such parameter, so
    `stripe.refunds.create({ payment_intent })` is a `400` naming
    `destination`. **Both merchant SDKs carry the field** — `destination` on
    `CreateRefundParams` in `sdks/rust/src/resources.rs` and on the
    `CreateRefundParams` in `sdks/nodejs/src/types.ts`, each sending
    `destination[<rail>][msisdn]` — so a vpay SDK is not where this
    divergence bites; a Stripe-shaped client is.
    ~~Neither merchant SDK carries the field either — a dated gap in
    [../sdks/parity.md](../sdks/parity.md).~~ **— corrected 2026-09-16.** That
    sentence was written by the seam pass when it was true, and the same day's
    SDK work made it false; `parity.md`'s `destination` row has been ✅/✅
    since, and its § "Not that the server offers every capability" retracts
    the gap in its own words. Two vpay documents contradicting each other on
    the same field is the failure this line is kept to record.
  - `POST /v1/refunds/{id}` takes `metadata` and nothing else, which **is**
    Stripe's contract. Where vpay diverges is the refusal: Stripe answers an
    unaccepted parameter with `parameter_unknown`, and vpay answers a `400`
    naming the parameter it will not take (`amount`, `reason`,
    `payment_intent` or `destination`) with a sentence saying to cancel and
    re-create instead.

  And the thing no compatibility note can carry: **a `201` does not mean
  money came back.** The refund is `pending`, no rail in this repository has
  ever returned money, and nothing settles a `pending` refund (RFC-0003 open
  question 8).

- **`/v1/events` and `/v1/events/{id}` _are_ routed** (Step 5), and their
  bodies are Stripe's `event` shape — but **nothing here drives them through
  stripe-node's `events` resource**, so `stripe.events.list()` working is
  untested rather than known. The half that _is_ observed is
  `webhooks.constructEvent` over a delivered body, below.

## Webhooks

`stripe.webhooks.constructEvent(rawBody, header, secret)` takes the header
**value**, not the request, and verifies `t=<unix>,v1=<lowercase hex
HMAC-SHA256 of "<t>.<raw body>">` with a 300 s default tolerance. That is
byte-identical to vpay's `Vpay-Signature` construction, so the verifier works
unchanged:

```js
const event = stripe.webhooks.constructEvent(
  rawBody,
  req.headers["vpay-signature"],
  process.env.VPAY_WEBHOOK_SECRET,
);
```

**A delivery has been verified with this exact call.** vpay's deliverer sends
`Vpay-Signature` and `Stripe-Signature` carrying the same string byte for
byte, so `req.headers["stripe-signature"]` works unedited and the snippet
above is the same call with the other name.

`sdks/stripe-compat/src/webhooks.compat.test.ts` is the evidence, and it is an
observation rather than an argument from the scheme being identical: it makes
a payment through the official `stripe` package, waits for the worker to
settle it, pulls the delivery out of the WireMock receiver's own request
journal (`GET /__admin/requests` — what a receiver _got_, not what vpay
believes it sent) and passes the recorded bytes and header straight to
`constructEvent`. The bytes are never re-serialised: the signature covers a
body, and parse-and-reprint is the commonest way a merchant breaks their own
verification. Both refusals are asserted too — one flipped byte of the
payload, and the right body with the wrong secret — because a verifier that
accepted everything would have accepted the delivery as well.

`Vpay-Signature` stays the authoritative name in vpay's own documentation.

## Rust

There is no Rust twin. `async-stripe` builds its client around a
headers/secret pair and has no seam through which one request becomes a sealed
one; reaching the same result means wrapping its transport in custom
middleware, which is a larger surface than this and was scoped as a follow-up
(a dated gap in [../sdks/parity.md](../sdks/parity.md)). Rust merchants use
[`sdks/rust`](../../sdks/rust/).

## Status

**Moved onto a sealing `httpClient`, 2026-10-26 (VP-3 + VP-5 + VP-7).** The
`authenticator` this page described is deleted (`createStripeAuthenticator`, the
token cache and the RS256 assertion went with it), `createStripeHttpClient`
replaces it, and `sdks/stripe-compat` drives the real `stripe` package through
it. What was run for this change: the client's own suite
(`sdks/nodejs/src/stripe-http-client.test.ts`, 19 cases, against a verifier
built from cratestack's own server envelope, not from the SDK's sealing code)
and `stripe-node`'s types against it. What was **not** run: the 25 cases below
against a live compose stack, which needs the full `e2e` job; the paragraph that
follows is the record of the 2026-09-03 run that proved them with the
authenticator, and the case that exercised a rejected bearer is now the one that
exercises a refused signature (`errors.compat.test.ts`).

**Built and proven against a real stack, 2026-09-03.**

The evidence is [`sdks/stripe-compat`](../../sdks/stripe-compat/): the real
`stripe@22.6.1` package driven through the authenticator then, and a sealing `httpClient` now, against a
real `vpay-server` + Postgres + WireMock rails + worker + WireMock webhook
receiver, out of process over TCP. **25 cases, 0 skipped**, measured passing
on the authoring machine on 2026-09-03 via `just demo_port=18080
stripe-compat`. The suite cannot skip: its `globalSetup` fails the run when no
stack answers `/healthz` or when the merchant handshake does not complete. CI
runs it in the `e2e (compose)` job.

What those 25 cases cover: create, retrieve, cursor paging (including
stripe-node's own `autoPagingToArray`), cancel, confirm to `processing` and
then **poll to `succeeded`**; a **delivered webhook verified with
`stripe.webhooks.constructEvent`**, and refused for a tampered body and for a
wrong secret; the `request-id`/`x-request-id` mirror carrying one value;
`Stripe-Version` and `Stripe-Account` accepted and ignored; no `apiVersion`
echoed; `expand` accepted and ignored through stripe-node's own indexed array
encoding; an unknown id, a rejected bearer, `confirm: true`, a missing
`payment_method_types` and a lifecycle 409 all mapping to the classes above
with `err.requestId` populated; `capture_method: "manual"` and `transfer_data`
refused on create and `application_fee_amount` refused on confirm, each
naming its own field in `err.param`, with `capture_method: "automatic"`
accepted and the refused confirm leaving the intent still at
`requires_payment_method`; `stripe-should-retry: false` read back off the 409
through the SDK's own error object, inside a time bound no retried request
could meet; `Idempotency-Key` replay returning the same object and a changed
body under the same key returning `StripeIdempotencyError`; and the refused
unknown operation and the over-limit body described above, each answered once.

**What is not proven, and must not be read as proven:**

- **The `stripe-should-retry: true` direction is not observed.** Provoking
  `IdempotencyKeyInFlight` needs two concurrent requests where one holds the
  key long enough for the other to collide, and the only slow operation is a
  confirm, whose rail-side delay is keyed by a reference the server mints. A
  deterministic stage would need a test double, which
  [ADR-0006](../adr/0006-no-mocks-in-main-processes.md) forbids in a shipping
  process. The _derivation_ is unit-tested in `vpay-api`
  (`the_retry_advisory_follows_the_classification_not_the_status`); its effect
  on stripe-node is not.
- **The `502` re-POST described above is reasoning, not a measurement.**
- **The rail is a WireMock host.** This compatibility suite has never called
  MTN. No money has moved. The `succeeded` the suite polls to is a stub mapping answering
  `SUCCESSFUL`, driven through the real worker, the real settlement
  transaction and the real `/v1` renderer — but the approval itself is
  fiction. **Scope clarified 2026-10-20:** Valantine Suh (@Valsuh45) separately confirms
  over seven successful MTN sandbox payments in the testing recorded on
  2026-09-15 ([verification](../status/verification/2026-09-15.md)).
  That aggregate is not evidence that this compatibility suite ran against MTN.
- **The receiver is a WireMock host too.** No merchant endpoint has ever been
  POSTed to. ~~and there is no SSRF protection on the destination~~ —
  **corrected 2026-09-04 (Step 8): every delivery now goes through
  `vpay_worker::ssrf`**, and the compose stack's private receiver is permitted
  by the sandbox profile's `webhooks.allow_private_targets` rather than by the
  guard being absent ([webhooks.md](webhooks.md)).
- **Nothing pins vpay against a future `stripe` release.** The suite runs
  against `^22.6.1`, tested at exactly `22.6.1`; a dependency bump is what
  will surface a break.

**2026-09-04 (Step 9).** `@vaam-apps/vpay-stripe-js`'s README no longer lists
"Checkout (hosted or embedded)" under "Not compatible, ever". The retraction
is narrower than the removal looks and is worded that way in the README:
vpay now serves its **own** checkout page, hosted and embedded, and
`initEmbeddedCheckout`/`retrieveCheckoutSession` speak to it. It is not
`@stripe/stripe-js`'s Checkout — Stripe's own method is
`createEmbeddedCheckoutPage` in the pinned 9.15.0, its options are not ours
in either direction, and vpay's `checkout.session` has no `line_items`,
`mode` or `amount_total` (Step 9's D10). What _is_ portable, and is pinned as
a compile-time assertion in both directions in `src/compat.test.ts`, is the
handle: `{ mount(string | HTMLElement), unmount(), destroy() }` is
assignable to and from Stripe's `StripeEmbeddedCheckout`. The mounting
plumbing moves; the session model does not. `sdks/stripe-compat` gains no
row for any of this — D10 says the Checkout Session is evidence-free of a
Stripe promise, and the compat suite proves claims rather than making them.

**One thing about the compat suite itself moved with Step 9, and it is a
constant rather than a claim:** `sdks/stripe-compat`'s `CURRENCY` is now
`xaf`. CI's `e2e` job brings the stack up with `-f compose.demo.yml`, whose
generated overlay settles **both** rails in XAF so the demo shop's MTN button
is payable; left on `eur`, every confirm in this suite would have been refused
with `rail 'mtn_momo' settles in XAF; this PaymentIntent is EUR`.
`config/application.yml` is unchanged and still puts `mtn_momo` on EUR, because
MTN's real sandbox rejects XAF ([money.md](money.md)).
