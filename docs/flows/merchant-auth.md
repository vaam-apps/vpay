# Merchant authentication and the SDK wire contract

## The invariant

> **No `/v1` request runs without a short-lived access token that vpay issued
> against an assertion only the merchant's own private key could have signed.**

`/v1` never accepts an API key ([ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md)).
A merchant's backend is an OAuth2 client registered in vpay's YAML with its
**public** JWK; it authenticates with `client_credentials` (RFC 6749 §4.4)
using `private_key_jwt` client authentication (RFC 7523). This document is
the wire contract the two merchant SDKs — [`sdks/rust`](../../sdks/rust) and
[`sdks/nodejs`](../../sdks/nodejs) — implement, and the contract the server
side (Phase 2/3 of the [roadmap](../roadmap.md)) must serve. Where the two
disagree, the SDK is wrong *or* the server is wrong; neither may quietly
adapt to the other.

Everything below was derived from what the pinned OP actually enforces —
`authkestra-op = "=0.7.1"`, `src/client_assertion.rs` and
`src/handlers/token.rs` — not from the RFCs alone. Cited inline.

## The handshake

```
merchant backend (SDK)                         vpay (as OP for /v1)
      |                                                     |
      | 1. mint assertion: RS256 JWT, iss=sub=client_id,    |
      |    aud=<token endpoint>, jti=uuid, exp=now+60s      |
      |                                                     |
      | 2. POST <token endpoint>  (form-encoded)            |
      |    grant_type=client_credentials                    |
      |    client_id=…  client_assertion_type=…             |
      |    client_assertion=<jwt>  audience=vpay:v1         |
      |---------------------------------------------------->|
      |    200 { access_token, token_type, expires_in }      |
      |<----------------------------------------------------|
      |                                                     |
      | 3. /v1/* calls, Authorization: Bearer <access_token>|
      |---------------------------------------------------->|
      |                                                     |
      | 4. on expiry (or a 401): back to step 1 — there is  |
      |    no refresh token, by design (ADR-0010)           |
```

### 1. The client assertion

| Part | Value | Why exactly this |
|---|---|---|
| Header `alg` | `RS256` | The registered JWK is RSA; the OP derives the permitted algorithm set from the *key*, never from the header (`assertion_algorithms`), and refuses `none`/`HS*` before loading any key |
| Header `typ` | `JWT` | Conventional; the OP does not read it |
| Header `kid` | Optional. If the merchant registered more than one key it is **required** and must match one registered `kid` exactly; with one registered key it may be omitted | `select_key`: no `kid` ⇒ the JWK Set must hold exactly one key, otherwise the assertion is refused rather than guessed |
| `iss` | `client_id` | RFC 7523 §3 points 1–2; the OP checks `iss == sub == client_id` of the registration it loaded |
| `sub` | `client_id` | same |
| `aud` | The token endpoint URL, as a single string | RFC 7523 §3; the OP accepts either its token endpoint URL or its issuer identifier (`handlers/token.rs`, `authenticate_client`). The SDKs send the token endpoint |
| `jti` | UUIDv4, fresh per assertion | Spent exactly once server-side (`ClientAssertionStore`, backed by `oauth_client_assertion_jtis` — see [status](../status.md)). Reusing one is indistinguishable from a replay and is refused |
| `exp` | `now + lifetime`, lifetime **1..=300 s**, default 60 | `MAX_CLIENT_ASSERTION_LIFETIME_SECS = 300` on both the minting (`authkestra-engine`) and verifying side; anything further out is refused. The SDKs reject a configured lifetime outside that range at construction rather than clamping silently |
| `iat` | `now` | Emitted by `authkestra-engine`'s own minter; harmless to the OP, useful in logs |
| `nbf` | Not emitted | Optional in RFC 7523 §3; the OP validates it only if present |

The OP allows 60 s of clock leeway (`jsonwebtoken`'s default), so a merchant
clock a few seconds off still authenticates.

### 2. The token request

`POST` to the token endpoint, `Content-Type: application/x-www-form-urlencoded`:

| Field | Value |
|---|---|
| `grant_type` | `client_credentials` |
| `client_id` | the merchant's `client_id` (RFC 7521 §4.2 makes it optional alongside an assertion; the SDKs always send it so a log line names the caller even when the assertion fails to parse) |
| `client_assertion_type` | `urn:ietf:params:oauth:client-assertion-type:jwt-bearer` |
| `client_assertion` | the JWT from step 1 |
| `audience` | `vpay:v1` (see below) |
| `scope` | Only if the merchant configured one; otherwise omitted |

No `client_secret`, ever — the OP rejects a request presenting more than one
client-authentication method (`extract_credential`), and a merchant
registration has no secret to present.

**`audience=vpay:v1` is provisional and load-bearing.** Without a requested
audience the OP mints the access token with `aud = client_id`
(`handle_client_credentials`), and vpay's resource-server validator
(`vpay_api::resource_auth::Surface::Merchant.audience()`) requires
`aud = "vpay:v1"`, so a token minted without this field would be rejected by
every `/v1` route the moment one exists. Both SDKs therefore send it by
default and make it configurable. The string itself is marked provisional in
`resource_auth.rs`; whoever wires token issuance must register `vpay:v1` in
each merchant client's `allowed_audiences` (the OP checks that list) and keep
the two constants equal.

**Success response** (`authkestra_op::handlers::token::TokenResponse`):

```json
{ "access_token": "…", "token_type": "Bearer", "expires_in": 300, "scope": "…" }
```

No `refresh_token` on this grant, by the OP's own `client_credentials`
handler and by ADR-0010. `expires_in` is `OpConfig::access_token_ttl_secs`.

**Error response** (`TokenErrorResponse`, HTTP 400/401):

```json
{ "error": "invalid_client", "error_description": "…" }
```

The SDKs surface this as a distinct authentication-error type carrying both
fields; it is never retried automatically.

### 3. Using the token

Every `/v1` request carries `Authorization: Bearer <access_token>`. The
SDKs cache the token and reuse it until `expires_in` minus a safety margin
has elapsed (margin: 30 s, or half of `expires_in` for very short TTLs —
integer arithmetic only), then transparently repeat steps 1–2. Concurrent
callers share one in-flight token request rather than each minting their
own; two assertions minted in the same second would spend two `jti`s for no
reason.

### 4. Re-authentication

On a `401` from any `/v1` route the SDK discards its cached token, performs
steps 1–2 once more, and retries the request **once**. A second `401` is
returned to the caller. A `401` from the token endpoint itself is never
retried.

## Endpoint locations — not yet decided server-side

The token endpoint's path has not been fixed by any ADR or by code (the
router serves only `/healthz`). `authkestra-op` derives every OP endpoint from
one `issuer` string: token endpoint = `{issuer}/token`, JWKS =
`{issuer}/jwks.json`, discovery = `{issuer}/.well-known/openid-configuration`
(`OpConfig` in `config.rs`). The SDKs' **default** follows the existing
[`examples/merchant-curl`](../../examples/merchant-curl/README.md):

| | Default | Override |
|---|---|---|
| Issuer | `{base_url}/v1/oauth` | configurable |
| Token endpoint (and assertion `aud`) | `{issuer}/token` → `{base_url}/v1/oauth/token` | configurable |
| Resource base | `{base_url}/v1` | configurable |

This is a maintainer decision the SDKs deliberately do not make: a different
issuer is a one-line configuration change on the SDK side. It is listed under
open decisions in [`docs/status.md`](../status.md).

## The `/v1` resource contract the SDKs implement

Stripe-shaped, per [`docs/api/README.md`](../api/README.md): form-encoded
request bodies, Stripe's object model, Stripe's error envelope, Stripe's
idempotency semantics. Field names are Stripe's own wherever a Stripe field
exists, so a merchant's existing types keep working; nothing below is a vpay
invention beyond the rails' payment-method names.

### Encoding

Request bodies are `application/x-www-form-urlencoded`, bracket-nested the
way Stripe's official SDKs encode them:

| Shape | Wire form |
|---|---|
| scalar | `amount=5000` |
| nested object | `metadata[order_id]=1234`, `payment_method_data[mtn_momo][msisdn]=237670000000` |
| array | `payment_method_types[0]=mtn_momo&payment_method_types[1]=orange_money` (indexed, as `stripe-node`/`stripe-rust` send; the server must also accept the unindexed `payment_method_types[]=…` form the curl examples use, exactly as Stripe does) |
| boolean | `true` / `false` |
| integer | decimal, no separators; **amounts are integer minor units** ([money.md](money.md)) — both SDKs refuse a non-integer amount before it reaches the wire |
| currency | lowercase on the wire (`xaf`), matching Stripe |

`GET` parameters use the same encoder into the query string. Responses are
JSON.

### Headers

| Header | When | Value |
|---|---|---|
| `Authorization` | always | `Bearer <access_token>` |
| `Idempotency-Key` | every `POST` | caller-supplied, else a UUIDv4 generated per call — so a network retry can never double-create |
| `Content-Type` | `POST` | `application/x-www-form-urlencoded` |
| `Accept` | always | `application/json` |
| `User-Agent` | always | `vpay-sdk-rust/<version>` / `vpay-sdk-node/<version>` |

### Resources

| Method | Path | Request fields | Returns |
|---|---|---|---|
| `POST` | `/v1/payment_intents` | `amount`, `currency`, `payment_method_types[]`, `metadata[…]`, `description` | `payment_intent` |
| `GET` | `/v1/payment_intents/{id}` | | `payment_intent` |
| `POST` | `/v1/payment_intents/{id}/confirm` | `payment_method_data[type]`, `payment_method_data[mtn_momo][msisdn]` (push), `return_url` (redirect) | `payment_intent` |
| `POST` | `/v1/payment_intents/{id}/cancel` | | `payment_intent` |
| `GET` | `/v1/payment_intents` | `limit`, `starting_after`, `ending_before` | `list` of `payment_intent` |
| `POST` | `/v1/refunds` | `payment_intent`, `amount` (omit for full), `reason`, `metadata[…]` | `refund` |
| `GET` | `/v1/events` | `limit`, `starting_after`, `ending_before`, `type` | `list` of `event` |
| `GET` | `/v1/balance` | | `balance` |

### Objects

`payment_intent`

| Field | Type | Notes |
|---|---|---|
| `id` | string | `pi_…` |
| `object` | `"payment_intent"` | |
| `amount` | integer | minor units |
| `currency` | string | lowercase |
| `status` | enum | exactly `vpay_core::state::IntentStatus`'s five values: `requires_payment_method`, `requires_action`, `processing`, `succeeded`, `canceled` — there is no `failed` status ([payment-lifecycle.md](payment-lifecycle.md)) |
| `payment_method_types` | string[] | rail codes: `mtn_momo`, `orange_money` |
| `next_action` | object or null | redirect rails only: `{ "type": "redirect_to_url", "redirect_to_url": { "url": "…", "return_url": "…" } }` |
| `last_payment_error` | object or null | `{ "code": <failure taxonomy>, "message": "…" }` — `code` is one of [failures.md](failures.md)'s closed vocabulary |
| `metadata` | object of string→string | |
| `description` | string or null | |
| `created` | integer | Unix seconds |
| `livemode` | boolean | |

`refund`: `id` (`re_…`), `object: "refund"`, `amount`, `currency`,
`payment_intent`, `status` (`pending` \| `succeeded` \| `failed` \| `canceled`),
`reason` (string or null), `metadata`, `created`.

`event`: `id` (`evt_…`), `object: "event"`, `type` (one of the real Stripe
event types [webhooks.md](webhooks.md) commits to), `created`, `livemode`,
`data: { "object": <the payment_intent or refund> }`. The SDKs keep
`data.object` as raw JSON with typed accessors, so an event carrying an
object the SDK does not model is still deliverable.

`balance`: `object: "balance"`, `available: [{ "amount", "currency" }]`,
`pending: [{ "amount", "currency" }]`.

`list`: `object: "list"`, `data: [...]`, `has_more: boolean`, `url`.

### Errors

Non-2xx responses carry `vpay_api::error_envelope`'s shape:

```json
{ "error": { "type": "invalid_request_error", "code": "…", "message": "…", "param": "…" } }
```

`param` is optional. The SDKs map this to one typed error carrying the HTTP
status and all four fields; a body that is not that shape (a proxy's HTML
502, say) becomes a distinct "unexpected response" error carrying the status
and a bounded prefix of the body. Transport failures (DNS, TLS, timeout) are
a third, distinct error. Nothing is retried except the single re-auth in
step 4.

## Webhook verification

Both SDKs ship the verifier for [webhooks.md](webhooks.md)'s scheme — the
same one `examples/webhook-receiver` hand-rolls:

- Header `Vpay-Signature: t=<unix seconds>,v1=<hex>`; more than one `v1=`
  may be present during a secret rotation and any one matching is enough.
- Signed payload is the literal bytes `"<t>.<raw body>"`. **The raw request
  body must be used**; a parsed-and-reserialised body breaks the HMAC.
- HMAC-SHA256 with the endpoint secret, hex-encoded, compared in constant
  time.
- Reject if `|now − t|` exceeds the tolerance (default 300 s), if the header
  is malformed, or if no `v1` matches. Then, and only then, parse the body as
  an `event`.

Delivery is at-least-once; the verifier does not dedupe by `event.id` —
that is the merchant's job, and the docs say so where the verifier is used.

## What can go wrong

| Failure | Where it surfaces | What the SDK does |
|---|---|---|
| Wrong private key, unregistered `kid`, `client_id` typo | `401`/`400` from the token endpoint with `invalid_client` | Returns an authentication error; no retry |
| Merchant disabled via `disabled_clients` ([status](../status.md)) | Token endpoint refuses, or a `/v1` route returns `401` | One re-auth attempt, then the error |
| Assertion `exp` too far out (> 300 s) | Refused by the OP | Cannot happen: the SDK refuses to be configured that way |
| Clock skew beyond 60 s | `invalid_client` | Returned; the message names the check that failed only as far as the OP does (it is deliberately not an oracle) |
| Token endpoint path differs from the SDK default | `404` with the Stripe-shaped `unknown_route` envelope | Returned as an unexpected-response error; the fix is the `issuer`/`token_endpoint` setting |
| Merchant's PR merged but a pod not yet restarted | `invalid_client` from one replica, success from another (ADR-0010's rolling-deploy window) | Returned; the merchant's own retry policy decides |

## Status

**The SDKs exist and are tested against a stub of this contract; the server
side of this contract does not exist.** No `/v1` route, no token endpoint,
no OP for merchants is mounted — `vpay-server` serves `/healthz` and a 404.
Neither SDK has ever completed step 2 against a real vpay, because there is
no real vpay to complete it against. What *is* proven:

- **Rust SDK** (`sdks/rust`, crate `vpay-sdk`): the assertion it mints is
  accepted by the real verifier, `authkestra_op::client_assertion::
  verify_client_assertion` at the pinned 0.7.1, against a `ClientRegistration`
  holding the corresponding public JWK — with and without a `kid` — and an
  assertion signed by a different key, or for a different audience, is
  refused by that same verifier (`tests/op_conformance.rs`). The token
  exchange, token caching, single-flight refresh, the 401 re-auth, every
  resource's exact form-encoded body and path, and the error envelope
  mapping are each exercised against a `wiremock` HTTP stub, asserting the
  bytes on the wire. The webhook verifier never touches the network; its
  cases are unit tests in `src/webhooks.rs`.
- **Node SDK** (`sdks/nodejs`, package `@vpay/sdk`): the same set, against a
  real `node:http` server started by the test, with the assertion's
  signature verified by `node:crypto` against the public key and its claims
  asserted one by one against the table above. Node cannot link the Rust
  verifier, so `just sdk-conformance-node` bridges the gap: it mints an
  assertion with the built Node SDK and pipes it into
  `sdks/rust/examples/verify_assertion.rs`, which runs the real
  `verify_client_assertion`. It is a recipe, not a CI gate; `docs/status.md`
  records when it was last run and what it printed.
- **Form-body parity** between the two SDKs is pinned by tests in
  `sdks/rust/src/form.rs` that carry the exact string the Node encoder
  emitted for the same parameters.

See [`docs/status.md`](../status.md) for the row-by-row account and for the
open decisions this contract leaves to a maintainer.
