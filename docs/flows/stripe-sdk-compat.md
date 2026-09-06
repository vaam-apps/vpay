# Using the official Stripe SDKs against vpay

vpay's `/v1` object model, form encoding, error envelope and idempotency
semantics are Stripe's. Its **authentication is not**: there is no API key,
and every call carries a short-lived bearer token minted from an RFC 7523
`private_key_jwt` client assertion ([ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md),
[merchant-auth.md](merchant-auth.md)).

`stripe-node` accepts an arbitrary async `config.authenticator`, invoked once
per request attempt with the whole outbound request. That is the one seam the
handshake needs. This document is what does and does not carry over once it
is filled.

## The client

```js
import { readFileSync } from "node:fs";
import Stripe from "stripe";
import { createStripeAuthenticator } from "@vaam-apps/vpay-sdk/stripe";

const authenticator = createStripeAuthenticator({
  baseUrl: "https://api.vpay.example",
  clientId: "acme-cameroon",
  privateKey: readFileSync("./merchant-key.pem", "utf8"),
  kid: "acme-cameroon-2026-08", // only if you registered more than one JWK
});

const stripe = new Stripe("", {
  authenticator,
  host: "api.vpay.example",
  port: "443",
  protocol: "https",
  maxNetworkRetries: 2,
  timeout: 30_000,
  telemetry: false,
});
```

`new Stripe("", { authenticator })` is supported: stripe-node refuses only
when *both* a key and an authenticator are given, or neither.
`host`/`port`/`protocol` move every request off `api.stripe.com`. `basePath`
is fixed at `/v1/` and is not configurable — which is moot, because the
generated resources use absolute paths (`/v1/payment_intents`,
`/v1/payment_intents/{id}/confirm`) and those are exactly vpay's.

The authenticator writes **one** thing: `headers.Authorization`. It must not
rewrite the body — `Content-Length` is computed before it runs.

## What carries over unchanged

| | Why |
|---|---|
| The resource paths | `/v1/payment_intents`, `/{id}`, `/{id}/confirm`, `/{id}/cancel` are the four vpay serves, and stripe-node hardcodes exactly those strings |
| Form encoding, including nested and indexed keys | stripe-node percent-encodes and then decodes brackets back, so a space is `%20` and a literal `+` is `%2B` — which is what `vpay_api::form` requires. It serialises arrays **indexed** (`expand[0]=…`, `payment_method_types[0]=…`), and that spelling is proven end to end by `sdks/stripe-compat`'s "accepts and ignores `expand`, which stripe-node encodes as `expand[0]`" case. The bare `expand[]=…` spelling `vpay_api::form` also accepts is **not** exercised by this suite — stripe-node never emits it; its evidence is that decoder's own unit test |
| `Idempotency-Key` | stripe-node generates one for **every** v1 POST, unconditionally, "including when `maxNetworkRetries` is 0". vpay *requires* one on every `/v1` POST — stricter than Stripe — and that costs a stripe-node user nothing |
| The list envelope and auto-pagination | `autoPagingToArray` needs only `data[].id` and `has_more`; `ListObject` supplies both plus `url` |
| The error envelope | `{error: {type, code, message, param?}}`, with `type` from the same closed vocabulary |
| `webhooks.constructEvent` | vpay's signature construction is byte-identical to Stripe's — see "Webhooks" below for what is and is not built |

## Error mapping

stripe-node picks the error **class from the status code first**, and consults
`type` only inside 400/404. vpay derives status, `type` and `code` from one
classification ([ADR-0011](../adr/0011-error-modelling.md)), so the mapping
below is a property of the two designs meeting rather than of anything written
to make it line up.

| vpay answer | What you catch |
|---|---|
| `404` `resource_missing` | `StripeInvalidRequestError`, `err.code === "resource_missing"` |
| `400` `invalid_request` | `StripeInvalidRequestError`, with `err.param` naming the field |
| `400` `idempotency_error` | `StripeIdempotencyError` |
| `401` `authentication_error` | `StripeAuthenticationError` |
| `403` (missing scope) | `StripePermissionError` — **despite** carrying `type: invalid_request_error`, because stripe-node branches on the status |
| `409` (lifecycle conflict) | `StripeAPIError` — 409 falls through every branch of `generateV1Error` |
| `502` (rail transport) | `StripeAPIError`, **and stripe-node retries it** — see below |
| `429` | never emitted: nothing in `backends/crates` constructs `Category::RateLimited` |

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

The fix deliberately *not* taken was re-deriving the advisory from the stored
status at replay time: ADR-0011 makes one classification the source of status
*and* retry, and a second derivation running the other way round is exactly
the drift it exists to prevent. Storing the header's text rather than a
`BOOLEAN` follows from the same rule — a boolean would put a second
`bool → "true"/"false"` rendering in the replay path, where the column's job
is to record what was sent. The domain is bought back by a CHECK
(`response_retry_is_an_advisory`), proven firing by
`the_retry_advisory_round_trips_and_0025_refuses_anything_else` in
`vpay-db`'s repository suite.

Two tests pin the behaviour, and both used to assert the opposite:
`a_replayed_response_carries_the_advisory_it_was_stored_with` (a `vpay-api`
unit test — it asserts the replayed value *equals* what the same error renders
fresh, so a hard-coded `false` in `replay` fails it) and
`a_replayed_error_carries_the_same_retry_advisory_the_original_did` (an
integration test in `backends/tests/integration`, against a real Postgres and
a real replay reached through the real "one charge per intent" refusal).

### Two responses that carry neither the envelope nor the header

`405` (a route asked with the wrong method) is axum's own, with an empty body.
`413` (a body past 64 KiB) is tower-http's, `text/plain`. Neither passes
through `ApiError::into_response`, so neither carries the Stripe envelope nor
`stripe-should-retry`.

The consequence is worse than a missing header, and it is **measured, not
inferred** (`sdks/stripe-compat/src/errors.compat.test.ts`): stripe-node meets
a non-JSON body by discarding everything it knows about the response and
throwing

```
StripeAPIError: Invalid JSON received from the Stripe API
```

with `statusCode` and `headers` both `undefined`. So a merchant cannot tell a
405 from a 413 from a proxy's HTML 502 — the only thing that survives is
`err.requestId`. A `method_not_allowed` renderer for the whole surface, and an
envelope for the body limit, would fix both; neither exists.

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
  is deliberately *not* a 400 — a documented "Connect is not a thing here" is
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
  capture-later split (confirming *is* the charge) and no Connect, so ignoring
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
  (`metadata` is stored; the rest are dropped).
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
- **`search`, `POST /v1/refunds` and `/v1/balance`** are not routed and answer
  the honest `404 unknown_route`. **`GET /v1/refunds/{id}` *is* routed** since
  2026-09-05 (issue #45) and answers a Stripe-shaped `refund` — but **nothing
  here drives it through stripe-node's `refunds` resource**, so
  `stripe.refunds.retrieve()` working is untested rather than known, exactly
  as `stripe.events.list()` is below. `stripe.refunds.create()` remains a
  `404`, and correctly so: no rail can refund.
- **`/v1/events` and `/v1/events/{id}` *are* routed** (Step 5), and their
  bodies are Stripe's `event` shape — but **nothing here drives them through
  stripe-node's `events` resource**, so `stripe.events.list()` working is
  untested rather than known. The half that *is* observed is
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
journal (`GET /__admin/requests` — what a receiver *got*, not what vpay
believes it sent) and passes the recorded bytes and header straight to
`constructEvent`. The bytes are never re-serialised: the signature covers a
body, and parse-and-reprint is the commonest way a merchant breaks their own
verification. Both refusals are asserted too — one flipped byte of the
payload, and the right body with the wrong secret — because a verifier that
accepted everything would have accepted the delivery as well.

`Vpay-Signature` stays the authoritative name in vpay's own documentation.

## Rust

There is no Rust twin. `async-stripe` builds its client around a
headers/secret pair and has no per-request async hook equivalent to
stripe-node's `RequestAuthenticator`; reaching the same result means wrapping
its transport in custom middleware, which is a larger surface than this and
was scoped as a follow-up. Rust merchants use [`sdks/rust`](../../sdks/rust/).

## Status

**Built and proven against a real stack, 2026-09-03.**

The evidence is [`sdks/stripe-compat`](../../sdks/stripe-compat/): the real
`stripe@22.6.1` package driven through `createStripeAuthenticator` against a
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
body under the same key returning `StripeIdempotencyError`; and the 405/413
collapse described above.

**What is not proven, and must not be read as proven:**

- **The `stripe-should-retry: true` direction is not observed.** Provoking
  `IdempotencyKeyInFlight` needs two concurrent requests where one holds the
  key long enough for the other to collide, and the only slow operation is a
  confirm, whose rail-side delay is keyed by a reference the server mints. A
  deterministic stage would need a test double, which
  [ADR-0006](../adr/0006-no-mocks-in-main-processes.md) forbids in a shipping
  process. The *derivation* is unit-tested in `vpay-api`
  (`the_retry_advisory_follows_the_classification_not_the_status`); its effect
  on stripe-node is not.
- **The `502` re-POST described above is reasoning, not a measurement.**
- **The rail is a WireMock host.** MTN has never been called. No money has
  moved. The `succeeded` the suite polls to is a stub mapping answering
  `SUCCESSFUL`, driven through the real worker, the real settlement
  transaction and the real `/v1` renderer — but the approval itself is
  fiction.
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
`mode` or `amount_total` (Step 9's D10). What *is* portable, and is pinned as
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

