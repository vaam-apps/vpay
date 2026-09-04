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
| `aud` | The OP's own token endpoint URL or issuer, as a single string | RFC 7523 §3; the OP accepts either its token endpoint URL or its issuer identifier (`handlers/token.rs`, `authenticate_client`), **both derived from `deployment.public_base_url` and from nothing else**. The SDKs default it to the URL they POST to, which is right only when the merchant reaches vpay at the URL vpay publishes as its own — see below |
| `jti` | UUIDv4, fresh per assertion | Spent exactly once server-side (`ClientAssertionStore`, backed by `oauth_client_assertion_jtis` — see [status](../status.md)). Reusing one is indistinguishable from a replay and is refused |
| `exp` | `now + lifetime`, lifetime **1..=300 s**, default 60 | `MAX_CLIENT_ASSERTION_LIFETIME_SECS = 300` on both the minting (`authkestra-engine`) and verifying side; anything further out is refused. The SDKs reject a configured lifetime outside that range at construction rather than clamping silently |
| `iat` | `now` | Emitted by `authkestra-engine`'s own minter; harmless to the OP, useful in logs |
| `nbf` | Not emitted | Optional in RFC 7523 §3; the OP validates it only if present |

The OP allows 60 s of clock leeway (`jsonwebtoken`'s default), so a merchant
clock a few seconds off still authenticates.

**`aud` is the OP's own name for itself, not the URL you POST to.**
`authenticate_client` compares the claim against exactly two strings —
`{deployment.public_base_url}/v1/oauth/token` and the
`{deployment.public_base_url}/v1/oauth` issuer (`vpay_api::op::issuer_for`) —
and against nothing else. A merchant whose server reaches vpay by an internal
URL (a compose service name, a private DNS name, a mesh address) must say so:
`assertionAudience` in `@vpay/sdk`, `ClientBuilder::assertion_audience` in
`sdks/rust`. Both default to the token endpoint, which is right only when the
two coincide. Left wrong, every token request answers `invalid_client` /
`InvalidAudience` while the signature, the `client_id`, the `kid` and the
lifetime are all correct — the response says nothing about audiences, so this
is not a failure a merchant diagnoses from the wire.

That is a **third** string, not a redefinition of either of the two beside it,
and the three are worth keeping apart: the *token endpoint* is where the request
is POSTed and nobody compares it; the *assertion audience* is what the OP calls
itself; and the `audience` **request parameter** (`vpay:v1`, the next table) is
the resource server the minted token is for. It is
[ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md)-adjacent and changes
nothing that ADR decided — what changed on 2026-09-04 is that both SDKs stopped
conflating the first two.

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

**`audience=vpay:v1` is load-bearing, and no longer provisional.** Without a
requested audience the OP mints the access token with `aud = client_id`
(`handle_client_credentials`), and vpay's resource-server validator
(`vpay_api::resource_auth::Surface::Merchant.audience()`) requires
`aud = "vpay:v1"`, so a token minted without this field would be rejected by
every `/v1` route the moment one exists. Both SDKs therefore send it by
default and make it configurable.

The string is now defined once, as `vpay_config::MERCHANT_AUDIENCE`, and
`Surface::Merchant.audience()` returns that constant rather than a second
copy of the spelling — the "keep the two constants equal" instruction this
paragraph used to give is now structurally unnecessary. The third party to
the agreement, each merchant's `allowed_audiences`, is checked at boot:
`vpay_config::ConfigError::MerchantMissingV1Audience` refuses to start a
deployment whose merchant registration cannot target it, because neither
runtime symptom names the cause (`invalid_target` from the token endpoint if
the client requests the audience; a `200` carrying `aud = client_id` followed
by a bare `401` on every `/v1` call if it does not).

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

**Scopes, and what an omitted `scope` means.** The vocabulary is two
strings, defined once in `vpay_api::v1`: `payments:write`
(`SCOPE_PAYMENTS_WRITE`) and `payments:read` (`SCOPE_PAYMENTS_READ`). The
`/v1` boundary requires `payments:write` for any method that is not a read
and either scope for `GET`/`HEAD` — write implies read, and an unknown verb
requires write, which is fail-closed
(`only_a_write_scope_authorises_a_method_that_is_not_a_read`,
`the_scope_names_are_the_ones_registrations_are_written_against`, both in
`backends/crates/vpay-api/src/v1/mod.rs`). A token carrying neither is
`403 forbidden` — not `401`, because the credential is valid
(`a_token_without_the_required_scope_is_403_not_401`,
`a_client_registered_for_no_scopes_is_forbidden_while_a_scoped_one_is_not`).

**A token request that names no `scope` is granted the client's registered
`scopes:` from the YAML** — RFC 6749 §3.3's "locally defined default",
defined here as the registration itself. Both SDKs omit `scope`, so this is
the path every SDK call takes. Applied in `vpay_api::op::token::token_handler`
before the grant runs, and it only ever *fills in* an omitted value: a
request naming a narrower scope keeps it, and anything outside the
registration is still `invalid_scope`
(`the_default_scope_is_the_clients_own_registration_and_nothing_wider`).
An empty `scopes:` list is legal and means the client can mint a token and
be `403`ed by every `/v1` call it makes.

### 4. Re-authentication

On a `401` from any `/v1` route the SDK discards its cached token, performs
steps 1–2 once more, and retries the request **once**. A second `401` is
returned to the caller. A `401` from the token endpoint itself is never
retried.

## Endpoint locations — decided server-side on 2026-09-02

Until 2026-09-02 no ADR or code fixed the token endpoint's path; the
paragraph after the table records what fixed it. `authkestra-op` derives every OP endpoint from
one `issuer` string: token endpoint = `{issuer}/token`, JWKS =
`{issuer}/jwks.json`, discovery = `{issuer}/.well-known/openid-configuration`
(`OpConfig` in `config.rs`). The SDKs' **default** follows the existing
[`examples/merchant-curl`](../../examples/merchant-curl/README.md):

| | Default | Override |
|---|---|---|
| Issuer | `{base_url}/v1/oauth` | configurable |
| Token endpoint (and assertion `aud`) | `{issuer}/token` → `{base_url}/v1/oauth/token` | configurable |
| Resource base | `{base_url}/v1` | configurable |

**Decided by the server on 2026-09-02, and the SDK defaults were already
right.** `vpay_api::op::issuer_for` derives the issuer as
`{deployment.public_base_url}/v1/oauth` (trailing slash trimmed) and it is
the single derivation in the workspace — `MerchantOp::new` and
`vpay-server`'s `main` both call it, so the `iss` a token is stamped with,
the `iss` the validator pins and the `issuer` in the discovery document
cannot drift apart. `the_issuer_and_endpoints_are_what_the_sdk_derives_from_a_base_url`
pins the values; `the_jwks_and_discovery_documents_describe_this_process`
compares the served discovery document against what the SDK derived on its
own, over a booted server. It is a *deployment* setting, not a per-SDK one:
`/v1/oauth` is not configurable, because a deployment that moved it would
silently break every merchant who took the default.

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

### Idempotency

**`Idempotency-Key` is required on every `POST`**, which is stricter than
Stripe, where it is optional. Both SDKs already send one on every call (a
caller-supplied value, else a per-call UUIDv4), so the requirement costs a
correct client nothing and stops a hand-rolled client from double-creating.
A `POST` without the header is `400`
`invalid_request_error`/`invalid_request` naming `idempotency_key`
(`a_post_without_an_idempotency_key_is_the_documented_400`).

The key is 1–255 printable-ASCII bytes
(`a_key_at_the_bound_is_accepted_and_one_byte_over_is_not`,
`only_printable_ascii_is_a_key`), scoped to the merchant, and stored for
**24 hours** (`idempotency_keys.expires_at`, migration `0015`). A request is
identified by a SHA-256 over method, path and raw body, framed so the three
fields cannot be shifted across each other
(`the_method_path_and_body_cannot_be_shifted_across_each_other`,
`the_digest_is_thirty_two_bytes_of_sha256_over_the_framed_fields`), and the
stored digest is compared in constant time (`subtle::ConstantTimeEq`) so the
response cannot be used as a hash oracle.

| What the caller did | What they get |
|---|---|
| Replayed a key whose first request finished | the **stored response body, byte for byte**, with its original status — `a_replayed_idempotency_key_returns_the_same_object_and_no_second_row` |
| Replayed a key with a different body | `400` `idempotency_error`/`idempotency_key_in_use` — `a_reused_key_with_a_different_body_is_the_400_envelope` |
| Sent a key whose first request is still running | `400` `idempotency_error`/`idempotency_key_in_flight` — `a_key_whose_first_request_is_still_running_is_answered_with_its_own_code` |
| Retried after a `5xx` | the key was **released**; the retry re-executes — `a_5xx_releases_its_idempotency_key_so_the_retry_re_executes` |
| Replayed after the deployment changed underneath | the original answer, unchanged — `a_replay_survives_the_rail_being_disabled` |

Two orderings matter and are deliberate. The key is claimed **before** a
create body is validated, so a replay short-circuits before a rule that has
since changed can be re-evaluated; and a *validation* failure **releases**
the key rather than storing the `400`, so a merchant who fixes the
deployment and retries under the same key gets the intent rather than a
day-old refusal. The claim itself is one `INSERT … ON CONFLICT`, never
check-then-insert, whose `DO UPDATE` arm is guarded so it can only ever
reclaim a row whose 24 hours have passed
(`concurrent_claims_of_one_idempotency_key_yield_exactly_one_fresh`,
`an_expired_in_flight_key_is_reclaimable_and_a_live_one_is_not`,
`backends/crates/vpay-db/tests/repositories.rs`). Each claim carries a
database-minted `claim_id`, and `store` and `release` both match on it, so a
request whose expired claim was taken over by a later one can neither
overwrite the new response nor delete the new claim
(`a_reclaimed_key_is_not_writable_by_the_claim_it_replaced`) — an ABA whose
payload would have been a payment response handed to a merchant for a request
they did not make.

**vpay answers `400` where Stripe answers `409` for a key still in flight.**
[ADR-0011](../adr/0011-error-modelling.md) derives the status from the
error's `Category` and `Category::Idempotency` is `400`; splitting one
Stripe `type` across two statuses is an ADR-level change, left as a
maintainer decision. Branch on `code`, which is distinct either way.

**Not built:** nothing sweeps the table on a schedule.
`vpay_db::Idempotency::sweep_expired` exists and `vpay-server` calls it once
at boot as a stopgap; there is no worker job loop, so a long-lived
deployment grows `idempotency_keys` monotonically between restarts.

### Resources

**Served** marks what a running `vpay-server` actually answers as of
2026-09-03. Everything else in this table is implemented by both SDKs and by
neither server route: an authenticated call gets the honest `404`.

| Method | Path | Request fields | Returns | Served |
|---|---|---|---|---|
| `POST` | `/v1/payment_intents` | `amount`, `currency`, `payment_method_types[]`, `metadata[…]`, `description` | `payment_intent` | ✅ |
| `GET` | `/v1/payment_intents/{id}` | | `payment_intent` | ✅ |
| `POST` | `/v1/payment_intents/{id}/confirm` | `payment_method_data[type]`, `payment_method_data[mtn_momo][msisdn]` (push), `return_url` (redirect) | `payment_intent` | 🟡 reaches a rail over HTTP: `processing` / `requires_action`, `409 charge_declined`, `502`. 🟡 because that rail has only ever been a WireMock stub |
| `POST` | `/v1/payment_intents/{id}/cancel` | | `payment_intent` | ✅ |
| `GET` | `/v1/payment_intents` | `limit`, `starting_after`, `ending_before` | `list` of `payment_intent` | ✅ |
| `POST` | `/v1/refunds` | `payment_intent`, `amount` (omit for full), `reason`, `metadata[…]` | `refund` | ⛔ 404 |
| `GET` | `/v1/events` | `limit`, `starting_after`, `ending_before`, `type` | `list` of `event` | ⛔ 404 |
| `GET` | `/v1/balance` | | `balance` | ⛔ 404 |

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

## Known limitations (security review, 2026-09-02)

Two findings from the review of the first server-side implementation are
recorded here rather than fixed, because each needs a decision a maintainer
has not made:

- **The `jti` replay namespace is global, not per merchant.**
  `oauth_client_assertion_jtis.jti` is the primary key on its own, and
  `authkestra_op::client_assertion::ClientAssertionStore::record_jti` hands
  the store no `client_id` to scope by. RFC 7523 only requires uniqueness
  per issuer, so a merchant whose library used a counter or a timestamp as
  `jti` would collide with — and could deliberately pre-spend — another
  merchant's values. **Onboarding requirement until this changes: `jti`
  MUST be a UUID v4** (both vpay SDKs do this). Scoping the key to
  `(client_id, jti)` needs a new migration and either an upstream seam or a
  per-client store instance; that is the decision left open.
- **No rate limit in front of `/v1/oauth/token` or `/v1`.** A known
  `client_id` (they are public) costs one `disabled_clients` `SELECT` per
  token request before any signature check, and ADR-0009 leaves `/token`
  rate limiting to the ingress. Confirm the ingress does it before relying
  on that; nothing in this repository enforces it.

## Status

**Updated 2026-09-03 (Step 2): the journey now has a far end.**
`vpay-server` serves `POST /v1/oauth/token`,
`GET /v1/oauth/.well-known/openid-configuration` and
`GET /v1/oauth/jwks.json`, every other `/v1` path sits behind the
`AuthenticatedMerchant` boundary, **and five of that boundary's routes now
answer with a payment intent rather than a 404**: create, retrieve, list and
cancel return the object; confirm reaches the rail adapter. **Updated
2026-09-03 (Step 3): that adapter now makes a real HTTP call**, so confirm
answers `200` with the intent in `processing` or `requires_action`, or `409
charge_declined` / `502` — not the `501 not_implemented` this paragraph
recorded on the day it was written. The rail behind it has only ever been a
WireMock host, and nothing polls the resulting charge, so no intent has
reached `succeeded`. `/v1/refunds`, `/v1/events` and `/v1/balance` are
still the honest 404, and that is the intended answer rather than a
placeholder.

**Evidence for the Step 2 half, run on this machine on 2026-09-03 with a
working rootless Docker daemon:** `cargo nextest run -p vpay-db -p
vpay-tests-integration` — **74 passed, 0 failed, 0 skipped**, of which 16 are
the new `backends/tests/integration/tests/payment_intents.rs` suite (its test
names are cited throughout this document and in
[`docs/status.md`](../status.md)) and 7 are `merchant_token_flow`, one more
than the six listed below. This is the first time the Docker-backed form of
the merchant-token tests has been observed passing here rather than against a
hand-rolled scratch database.

**The wire object is pinned against the Rust SDK's own type**, not a copy:
`the_merchant_sdk_deserialises_what_this_renders` decodes what
`vpay_api::model` renders into `vpay_sdk::PaymentIntent`, and
`the_wire_object_is_the_sdk_fixture` / `every_documented_key_is_present_including_the_null_ones`
pin all twelve keys including the three that must be present-and-null.
**The Node SDK's model is not exercised against this renderer by anything** —
its parity rests on the SDK-to-SDK form-body tests described below.

**Read the evidence before the claim.** `backends/tests/integration/tests/merchant_token_flow.rs`
boots a real router against a real Postgres and covers seven things (the
seventh, `a_token_minted_with_no_audience_is_addressed_to_the_client_and_refused_on_v1`,
was added after this list was first written: a token this server signed with
no requested audience carries the `client_id` as `aud` and is refused on
`/v1` for that alone):

- `an_sdk_client_authenticates_and_reaches_the_honest_404` — the Rust SDK
  mints an assertion, exchanges it for a token, and reaches `/v1` past the
  authentication boundary. The 404 *is* the assertion: it is only reachable
  with a valid token.
- `a_v1_request_with_no_bearer_token_is_the_401_envelope` — the other side
  of the boundary, over a raw client the SDK cannot impersonate.
- `a_disabled_client_is_refused_with_invalid_client_and_401` — the kill
  switch, with no restart in between.
- `a_dashboard_audience_token_is_refused_on_v1` — a token this same server
  signed, with a correct `kid` and a valid signature, refused for its `aud`
  alone.
- `the_jwks_and_discovery_documents_describe_this_process` — `/jwks.json`
  lists exactly the `kid` this process signs with, and discovery's `issuer`
  and `token_endpoint` equal what the SDK derived from the base URL on its
  own.
- `the_same_client_assertion_cannot_be_spent_twice` — one assertion, sent
  twice by hand; the second is `invalid_client`/401 while still inside its
  own lifetime.

**Evidence, corrected 2026-09-03.** The previous version of this paragraph
said these tests had run once, by hand, against a scratch database, because
testcontainers could not start on the authoring machine. That is no longer
the state: on 2026-09-03 all seven ran under testcontainers on this machine
as part of `cargo nextest run -p vpay-db -p vpay-tests-integration`
(**74 passed, 0 failed, 0 skipped**), against a real `postgres:16-alpine`
container. **They have still never run in CI**, and no vpay outside a test
process has ever completed this handshake for a real merchant.
[`docs/status.md`](../status.md) records the state of that evidence and is
the page to check.

**Two constants in the flow above are defaults this code chose, not
decisions anyone recorded:** the access token's 900 s lifetime
(`vpay_api::op::ACCESS_TOKEN_TTL_SECS`) and the 24 h window a retired
signing key stays publishable (`vpay_api::op::keys::ROTATION_OVERLAP`).
[`docs/roadmap.md`](../roadmap.md) lists both as open questions and this
work does not close them.

**Not done, and not hidden by any of the above:**

- **No rail call, so no payment.** The SDKs implement eight resource
  endpoints; the server now routes five of them (create, retrieve, list,
  cancel, confirm) and none of the other three. **`confirm` stops at the
  adapter's `NotImplemented` and answers `501`** — no HTTP request has ever
  been made to a rail by this code, and no payment intent has ever left
  `requires_payment_method` except into `canceled`.
- **`next_action` is never populated and `return_url` is dropped.** A
  redirect confirm validates `return_url` and discards it; `charges` has no
  column for it, and the `next_action` it would feed can only come from a
  successful `submit`.
- **No `/v1/refunds`, `/v1/events` or `/v1/balance`.** Migrations `0017` and
  `0018` add the `refunds` and `events` schemas and nothing reads or writes
  either table.
- **No scheduled idempotency sweep.** See the Idempotency section above.
- **No rate limit on `/token`.** [ADR-0009](../adr/0009-dashboard-oidc-provider.md)
  leaves it to Kubernetes ingress. The endpoint is public and
  unauthenticated by necessity (the credential is the request body), and
  nothing in this repository verifies that ingress actually limits it.
- **No cleanup job for spent `jti`s.** `vpay-server` sweeps expired rows
  once at boot (`vpay_db::delete_expired_client_assertion_jtis`), which is a
  stopgap and labelled one; there is no timer, because the worker's job loop
  does not exist. A long-lived process grows that table monotonically.
- **No runtime key rotation.** One key per process; rotating means
  restarting with a new Secret. A rollback to a retired `kid` is refused
  rather than silently accepted.
- **The signing-key PEM is not zeroized.** Key bytes may linger in freed
  heap; `vpay_api::op::keys`'s module docs state this deliberately.
- **`sdks/nodejs` has still never spoken to a vpay.** All 126 of its tests
  run against its own `node:http` stub, and the integration suite above uses
  the Rust SDK. `just sdk-conformance-node` remains a manual recipe outside
  `just ci`.

What the SDKs themselves prove is unchanged by any of this:

- **Rust SDK** (`sdks/rust`, crate `vpay-sdk`): the assertion it mints is
  accepted by the real verifier, `authkestra_op::client_assertion::
  verify_client_assertion` at the pinned 0.7.1, against a `ClientRegistration`
  holding the corresponding public JWK — with and without a `kid` — and an
  assertion signed by a different key, or for a different audience, is
  refused by that same verifier (`tests/op_conformance.rs`). Since
  2026-09-02 the same check runs against the registration the *server*
  builds from YAML, in `vpay-api`'s own tests
  (`an_sdk_minted_assertion_verifies_against_the_registration_this_module_builds`,
  with `an_assertion_signed_by_a_key_this_merchant_did_not_register_is_refused`
  as the negative control). The token exchange, token caching, single-flight
  refresh, the 401 re-auth, every resource's exact form-encoded body and
  path, and the error envelope mapping are each exercised against a
  `wiremock` HTTP stub, asserting the bytes on the wire. The webhook
  verifier never touches the network; its cases are unit tests in
  `src/webhooks.rs`.
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
- **Both SDKs separate the assertion's `aud` from the URL the token request is
  POSTed to** (2026-09-04, Step 9 lane 5b). Proven by
  `sdks/rust/tests/op_conformance.rs`'s
  `the_real_verifier_refuses_a_client_that_reaches_vpay_internally_and_sets_no_audience`
  and `the_real_verifier_accepts_the_same_client_once_assertion_audience_is_set`,
  which run a real `Client`'s assertion through the real pinned
  `authkestra_op` verifier, and by `examples/shop/src/server/vpay.test.ts` on
  the Node side — verifier-*shaped* rather than the verifier itself, which is
  the standing Node gap two bullets up. [`docs/sdks/parity.md`](../sdks/parity.md)
  records the capability ✅/✅. **The defect this closed had never been caught by
  a test on either side**: it needed a merchant's server reaching vpay by a name
  vpay does not publish as its own, and until `examples/shop` ran inside the
  compose network, nothing in this repository did.

See [`docs/status.md`](../status.md) for the row-by-row account.
