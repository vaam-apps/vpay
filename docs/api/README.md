# API

vpay exposes three authenticated HTTP surfaces plus one public, stateless
callback path, and conflating any of them is a security bug: `/v1` (a
merchant's own backend), `/v1/browser` (a payer's browser, added Step 5c —
see below), and `/dash/v1` (a logged-in staff member). Each has its own
credential model and its own section below.

## `/v1` — the merchant API (Stripe-shaped object model, not Stripe-shaped auth)

Authenticated with OAuth2 `client_credentials` + `private_key_jwt` (RFC
7523) — **not** an API key. Each merchant is a statically registered OAuth2
client, configured directly in vpay's YAML config
([ADR-0003](../adr/0003-yaml-configuration.md)), holding its own private
key; vpay stores only the merchant's **public** JWK. There is no
`sk_live_`/`sk_test_` key, no database-stored secret, and no other way to
authenticate **this** surface — `/v1/browser` below is a deliberately
different surface, for a different caller, with its own credential model; it
is not a second way in through this one. See
[ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md), which supersedes
the scope-boundary paragraph of
[ADR-0009](../adr/0009-dashboard-oidc-provider.md) that had kept `/v1` on
Stripe-shaped API keys.

Everything else about `/v1` is still Stripe-shaped: form-encoded bodies,
Stripe's object model, Stripe's error envelope, Stripe's idempotency
semantics.

**Errors.** Every non-2xx response is the Stripe envelope
`{ "error": { "type", "code", "message", "param"? } }`, and the status,
`type` and `code` are *derived* from the failing error's classification
(`vpay_core::error::Classify` → `vpay_api::ApiError`), never chosen per
handler — so the same failure always gets the same answer. The full
category → status/type/code table is in
[../flows/errors.md](../flows/errors.md); the decision is
[ADR-0011](../adr/0011-error-modelling.md). `message` is the error's
public message and never contains hosts, tables, library text or
credentials; the full chain goes to the server log.

**Status: the OAuth2 endpoints are served, and so is `/v1/payment_intents`
— up to the rail, which does not exist.** As of 2026-09-03, `vpay-server`
serves three unauthenticated OP endpoints plus five authenticated
payment-intent routes. The OP endpoints are unauthenticated by necessity,
not omission: requiring a bearer token to obtain a bearer token is
circular, and a client that has never spoken to vpay has to be able to find
the token endpoint and the keys.

| Method | Path | Auth | What it does |
|---|---|---|---|
| POST | `/v1/oauth/token` | none | `client_credentials` + `private_key_jwt` only. RFC 6749 error JSON; `invalid_client` is 401, every other error 400. |
| GET | `/v1/oauth/.well-known/openid-configuration` | none | Hand-built, so it advertises only what this deployment serves — no `/authorize`, no `/userinfo`, no device or refresh grant, `private_key_jwt` as the only client-auth method. |
| GET | `/v1/oauth/jwks.json` | none | Every publishable signing key (the active one plus any retired-but-unexpired key inside the rotation window), `Cache-Control: public, max-age=300`. |

The issuer is `{deployment.public_base_url}/v1/oauth` and is a deployment
setting, not a per-client one. `GET /v1/oauth/token` — the right path with
the wrong method — returns axum's bare `405` with an empty body rather than
the Stripe envelope; the status is correct and a `method_not_allowed`
renderer for the whole surface is the fix, not a special case here.

**Every other `/v1` path requires a merchant access token.** An
unauthenticated request gets `401`
`authentication_error`/`missing_bearer_token`
(`every_registered_v1_path_answers_401_without_a_token` walks the router's
own route table, `vpay_api::v1::V1_ROUTES`, so a route cannot exist without
being covered); an authenticated request to a path with no route gets `404
unknown_route`. A token must also carry a scope: `payments:write` for any
non-read method, `payments:read` **or** `payments:write` for `GET`/`HEAD`,
else `403` `forbidden`
(`a_client_registered_for_no_scopes_is_forbidden_while_a_scoped_one_is_not`).

### Served today

`vpay_api::v1::V1_ROUTES` is the router's source, not a copy of it. Eleven
methods across nine paths since Step 9:

| Method | Path | Request params | Answer |
|---|---|---|---|
| POST | `/v1/payment_intents` | `amount` (integer minor units, `1 ..= 2^53-1`), `currency` (lowercase, must be one this deployment configures), `payment_method_types[]` (rail codes, each enabled here), `metadata[…]` (≤50 keys, ≤40-char keys, ≤500-char values), `description` (≤1000 chars) | `200` + `payment_intent` in `requires_payment_method` (`create_then_retrieve_round_trips_through_the_sdk`) |
| GET | `/v1/payment_intents/{id}` | | `200` + `payment_intent`, or `404 resource_missing` — **including for another merchant's id**, byte for byte (`merchant_b_cannot_read_merchant_as_intent`) |
| GET | `/v1/payment_intents` | `limit` (default 10, capped at 100), `starting_after`, `ending_before` (ids; not both) | `200` + `list` envelope (`list_pages_forward_and_backward_with_cursors`) |
| POST | `/v1/payment_intents/{id}/confirm` | `payment_method_data[type]`, `payment_method_data[<type>][msisdn]` (push), `return_url` (redirect) | `200` + `payment_intent` in **`processing`** (push) or **`requires_action`** with `next_action.redirect_to_url` (redirect); `409 charge_declined`; `409 checkout_session_expired` / `checkout_session_complete` when a checkout session drives this intent and is no longer `open` — **before any charge is opened** (Step 9); `502 provider_unavailable`; `400` (see below) |
| POST | `/v1/payment_intents/{id}/cancel` | | `200` + `payment_intent` in `canceled`, or `409 invalid_state` (`cancel_is_legal_only_from_requires_payment_method`, `a_confirmed_intent_cannot_be_canceled`) |
| GET | `/v1/events` | `limit` (default 10, capped at 100), `starting_after`, `ending_before` (`evt_…` ids; not both) | `200` + `list` envelope of `event` objects, newest first, scoped to your merchant (`events_are_listed_newest_first_scoped_to_the_merchant`) |
| GET | `/v1/events/{id}` | | `200` + `event`, or `404 resource_missing` — **including for another merchant's id**, byte for byte (same test) |
| POST | `/v1/checkout/sessions` | `payment_intent` (**required**, a `pi_…` of yours in `requires_payment_method` with no charge and no other open session), `ui_mode` (`hosted` \| `embedded`, default `hosted`), `success_url` + `cancel_url` (**required** for `hosted`, refused for `embedded`), `return_url` (**required** for `embedded`, refused for `hosted`) — all http(s), ≤ 2048 chars, `https` only under livemode, and each may carry the literal `{CHECKOUT_SESSION_ID}` | `201` + `checkout.session` **with `client_secret`**, and `url` when hosted; `400` naming the field; `500 checkout_not_configured` when this deployment serves no checkout page (Step 9) |
| GET | `/v1/checkout/sessions/{id}` | | `200` + `checkout.session` **with `client_secret`**, or the uniform `404` (Step 9) |
| GET | `/v1/checkout/sessions` | `limit`, `starting_after`, `ending_before`, `payment_intent` | `200` + `list` envelope, **no secrets** (Step 9) |
| POST | `/v1/checkout/sessions/{id}/expire` | | `200` + the session in `expired`, or `409` when its intent has a charge a rail may still act on (Step 9) |

**`GET /v1/events` renders an event through the same code the webhook
deliverer signs** (`vpay_api::model::EventObject`). That is deliberate: this
list is the documented fallback when a webhook is missed, and two renderers
would let it answer a different question from the one the webhook asked. The
`event` object is six keys, and the delivered webhook body is these same bytes:

```json
{
  "id": "evt_…",
  "object": "event",
  "type": "payment_intent.succeeded",
  "created": 1753401600,
  "livemode": false,
  "data": { "object": { "…": "the payment_intent, verbatim" } }
}
```

`created` is unix **seconds**, like every other `created` on this surface.
`type` is one of the eight in [../flows/webhooks.md](../flows/webhooks.md); only
`payment_intent.succeeded`, `payment_intent.payment_failed` and
`checkout.session.expired` are ever written today, and the CHECK
`type_is_a_documented_event` (migrations `0018` and `0029`) closes the
vocabulary at the database. `livemode` comes off the stored row, not from
configuration read at render time, so redeploying does not change what a
delivered event says about itself.

**`data.object` is the same 12-key `payment_intent`
`GET /v1/payment_intents/{id}` returns** — `id`, `object`, `amount`, `currency`,
`status`, `payment_method_types`, `next_action`, `last_payment_error`,
`metadata`, `description`, `created`, `livemode` — rendered by
`vpay_api::model::PaymentIntentObject` at the moment the transition happened and
stored verbatim. It is a **snapshot, not a re-read**: an intent that changed
afterwards still shows what was true when the event was emitted. Neither the
delivered body nor this endpoint re-validates it against a payment-intent shape,
so an SDK version that predates a future object type can still receive the
event rather than failing to decode it.

**Except on `checkout.session.expired`, whose `data.object` is a
`checkout.session`** — the only type whose payload is not a `payment_intent` or
a refund. It is the 13-key object documented below, with `status` already
`expired`, `payment_status` whatever the money did, and **`url` always
`null`**: a hosted session's `url` carries its `client_secret` in the fragment,
and a webhook body is stored, delivered at-least-once and replayed. So `url:
null` here does **not** mean the session was embedded — read `ui_mode`.
`client_secret` is absent entirely. The event is emitted when vpay's hourly
sweep expires a session past its horizon, and **only** then: a session the
settlement finished already produced a `payment_intent.*` event for the same
thing, and `POST /v1/checkout/sessions/{id}/expire` emits nothing because you
asked for it.

### The `checkout.session` object (Step 9)

A session references a PaymentIntent you already created; it never creates one.
Amount, currency and the rails on offer stay on the intent.

```json
{
  "id": "cs_…", "object": "checkout.session", "livemode": false,
  "payment_intent": "pi_…", "ui_mode": "hosted",
  "status": "open", "payment_status": "unpaid",
  "success_url": "https://shop/ok?sid={CHECKOUT_SESSION_ID}",
  "cancel_url": "https://shop/cancel", "return_url": null,
  "url": "https://checkout.example/c/cs_…?key=pk_…#cs_…_secret_…",
  "expires_at": 1757000000, "created": 1756913600,
  "client_secret": "cs_…_secret_…"
}
```

`status` is `open`, `complete` (the intent reached `succeeded`) or `expired`
(24 hours from create, or the intent reached a terminal non-success state —
reported as `expired` with `payment_status: failed`). `payment_status` is
`unpaid` / `paid` / `failed`. There is no `line_items`, no `mode`, no
`amount_total` and no refunds; field names mirror Stripe's **only** where the
semantics match, and `sdks/stripe-compat` gets no row for any of it.

**`url` carries a live payer credential in its `#fragment`** — do not log it,
and do not move it into a query string. `client_secret` is rendered by
`create` and `retrieve` and never by `list`; both merchant SDKs redact it, and
the `url`'s fragment with it, from `Debug`/`util.inspect` while leaving
`JSON.stringify` faithful.

`{CHECKOUT_SESSION_ID}` in any of the three URLs is a **literal placeholder**
vpay substitutes when it forwards the payer. It is the only thing vpay ever
writes into a URL you supplied ([../flows/browser-checkout.md](../flows/browser-checkout.md)'s
D3 and D5); a URL without it gets a return with no correlation.

Creating a session requires `checkout.public_base_url` in the deployment's
configuration. Without it there is no page for a `url` to point at, and the
answer is `checkout_not_configured` — **status `500`, not `503`**, because
[ADR-0011](../adr/0011-error-modelling.md) derives the status from the error's
category and never from a call site. That is a known wart with an owner; see
[../flows/hosted-checkout.md](../flows/hosted-checkout.md).

**`?type=` is NOT implemented.** A `type` filter interacts with the cursor —
`has_more` and the `seq` window both have to be computed over the filtered set,
or paging silently skips rows — and half of that is worse than none. `?type=…`
is **ignored**, not refused, exactly as every other handler on this surface
ignores unknown query parameters, so a caller who sends it gets an unfiltered
page. See [../status.md](../status.md).

### What a webhook receiver sees

The delivered body is byte-for-byte the `event` object above. Four headers
accompany it (`vpay_worker::webhooks::handle_deliver`):

| Header | Value |
|---|---|
| `Content-Type` | `application/json` |
| `Vpay-Signature` | `t=<unix seconds>,v1=<hex>` — one `v1=` per configured secret, so **two** during a rotation |
| `Stripe-Signature` | the **same string**, byte for byte, so `stripe.webhooks.constructEvent` verifies it with the vpay secret |
| `Vpay-Event-Id` | the `evt_…` this delivery carries |

There is no custom user agent: the request goes out on
`vpay_provider::http::client_with_timeouts`, which sets none, so a receiver sees
reqwest's default. Do not filter on it.

`Vpay-Event-Id` is a **convenience for an access log, not evidence** — only the
body is signed, so a receiver must read `event.id` out of the verified body and
dedupe on that. **Delivery is at-least-once and unordered:** concurrent worker
tasks and the retry ladder can deliver two of your events out of the order they
happened, so do not decide state from arrival order — reason from
`event.created` and the object's own `status`. A failing delivery is retried
**8 times over about 31 hours** and then abandoned, and every non-2xx walks that
whole ladder, `4xx` included: answering `410 Gone` does not stop it. Verify the raw bytes as received: a framework that parses the
JSON and re-serialises it before verifying breaks every delivery. Both SDKs ship
the verifier (`vpay_sdk::webhooks::verify`, `@vaam-apps/vpay-sdk`'s `verifyWebhook`), both
reject a `t` more than 5 minutes from the receiver's clock, and both try every
`v1=`. The request deadline is **10 seconds end to end** (5 to connect), so
acknowledge with any `2xx` first and do the work afterwards; a `3xx` is a failed
attempt, because the client refuses redirects. Diagnosis and replay are
[../runbooks/webhook-delivery-failures.md](../runbooks/webhook-delivery-failures.md).

**A webhook that is never delivered leaves nothing on this surface to say
so.** A delivery that exhausts its retry ladder is recorded in
`webhook_deliveries` and logged with `alert = true`; the merchant is not told.
`GET /v1/events` is how they find out, by polling.

**`confirm` reaches a rail, and has four outcomes.** It performs every write
the lifecycle requires — a `submitting` charge row committed with its
`provider_reference_id` (and, on a redirect rail, the merchant's
`return_url`), then a `provider_requests` row with no status — *before*
calling the adapter. Which outcome you get is decided by the rail's answer,
never by anything the handler knows about rails:

| Outcome | Answer | What moved |
|---|---|---|
| Push rail accepted | `200`, intent **`processing`**, `next_action: null` | charge `submitted`; the payer's handset is prompting (`a_push_confirm_the_rail_accepts_moves_the_intent_to_processing`) |
| Redirect rail accepted | `200`, intent **`requires_action`**, `next_action.redirect_to_url` | charge `submitted` with the rail's token **and** URL, committed before this response was built (`redirect_confirm_commits_the_rails_material_before_it_answers`) |
| Rail declined | `409` `charge_declined` | charge **`failed`** with its `failure_code`; the intent keeps `requires_payment_method` and carries `last_payment_error`. A retry is a **new** PaymentIntent (`a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read`, `credentials_the_rail_refuses_are_a_page_and_a_terminal_charge`) |
| Rail unreachable / unreadable | `502` `provider_unavailable` | **nothing.** The charge stays `submitting` because we do not know what the rail did (`an_unreachable_rail_leaves_the_charge_where_recovery_expects_it`) |

**After a `502`, retry the same call under the same `Idempotency-Key`.** If
the first attempt did reach the rail, that retry gets a `409` whose message
is *"A charge for this PaymentIntent is being resolved with the rail; poll
`GET /v1/payment_intents/{id}` — do not create a new PaymentIntent."* Do
what it says: on a push rail, opening a second intent prompts the payer's
handset a second time for the same money. Only once the charge is
**terminal** does the `409` say "create a new payment intent to try again".

**Two `400`s happen before any charge exists**, so the intent is still
confirmable on another rail afterwards:

- the intent's `currency` is not the one the chosen rail settles in — `400`
  naming `payment_method_data[type]`
  (`a_rail_that_settles_in_another_currency_is_refused_before_any_charge`);
- `return_url` is missing on a redirect rail, is not `http`/`https`, or is
  over **2048** characters — `400` naming `return_url`
  (`a_return_url_that_is_not_a_bounded_web_url_is_refused_before_any_charge`).

A second confirm is `409` because the first one's charge row is still there
(`a_second_confirm_cannot_produce_a_second_charge`), and a cancel after a
confirm is `409` too, because the rail may hold a live payment. See
[../flows/crash-safety.md](../flows/crash-safety.md).

`next_action` is `null` on every intent except one in `requires_action`,
where it is a `redirect_to_url` carrying the rail's `url` and your
`return_url` — and the **same** `next_action` comes back from
`GET /v1/payment_intents/{id}`, so losing the confirm's response does not
strand the payment. There is no other `next_action` type.

**No rail this deployment has ever called was a real one.** Every outcome
above has been observed against a WireMock host; MTN and Orange have never
been contacted. *(The second half of this paragraph said "nothing polls a
`submitted` charge yet … `succeeded` has never happened" until 2026-09-03; Step
4's worker retired it. A `processing` intent is now driven to `succeeded` by
`vpay_worker`, and Step 5 delivers the webhook that follows — against WireMock
hosts, on a developer machine and in CI, and nowhere else.)* See
[../status.md](../status.md).

### Bodies, and the `Idempotency-Key` header

Request bodies are `application/x-www-form-urlencoded`, bracket-nested
Stripe-style, decoded by `vpay_api::form` (both `k[0]=v` and `k[]=v` array
spellings; `both_array_spellings_produce_the_same_array`). A JSON body is
refused with a 400 telling the caller to send a form
(`a_json_body_is_told_to_send_a_form`). Bodies over **64 KiB** are refused by
a layer on the whole `/v1` nest (`a_body_over_the_limit_is_refused_by_the_layer`).

**`Idempotency-Key` is required on every `POST`** — not optional as it is at
Stripe. A request without one is `400`
`invalid_request_error`/`invalid_request` naming `idempotency_key`
(`a_post_without_an_idempotency_key_is_the_documented_400`). Keys are 1–255
printable-ASCII bytes and live for 24 hours.

| Situation | Answer | Test |
|---|---|---|
| Same key, same body, first call finished | the **stored response, byte for byte** | `a_replayed_idempotency_key_returns_the_same_object_and_no_second_row` |
| Same key, different body | `400` `idempotency_error`/`idempotency_key_in_use` | `a_reused_key_with_a_different_body_is_the_400_envelope` |
| Same key, first call still running | `400` `idempotency_error`/`idempotency_key_in_flight` | `a_key_whose_first_request_is_still_running_is_answered_with_its_own_code` |
| First call answered `5xx` | key released; the retry re-executes | `a_5xx_releases_its_idempotency_key_so_the_retry_re_executes` |
| Deployment changed under a replay | the replay still answers what the original did | `a_replay_survives_the_rail_being_disabled` |

**Stripe answers `409` for a key still in flight; vpay answers `400`.**
[ADR-0011](../adr/0011-error-modelling.md) derives the status from the error's
`Category`, and `Category::Idempotency` is `400`/`idempotency_error` for both
idempotency codes. Splitting one `type` across two statuses is an ADR-level
change and is left as a maintainer decision; the `code` is what an SDK should
branch on. Recorded in `ApiError::IdempotencyKeyInFlight`'s own doc comment.

### Error codes a `/v1` caller can actually receive

`invalid_request` (400), `idempotency_key_in_use` (400),
`idempotency_key_in_flight` (400), `invalid_token` /`missing_bearer_token`
/`malformed_authorization_header` (401), `forbidden` (403),
`resource_missing` (404), `unknown_route` (404), `invalid_state` (409),
`resource_conflict` (409, a database uniqueness refusal),
`charge_declined` (409), `checkout_session_expired` (409),
`checkout_session_complete` (409), `provider_unavailable` (502),
`service_unavailable` (503), `checkout_not_configured` (500),
`internal_error` (500). *(`not_implemented`
(501) was on this list until 2026-09-03; the one remaining
`NotImplemented` token is `mtn_momo::refund`, reachable only through
`POST /v1/refunds`, which is not routed — so no `/v1` caller can provoke a
`501` today.)*
Every one is derived from a `Category`; see
[../flows/errors.md](../flows/errors.md). `Category::Conflict` now carries
three codes rather than its default alone — `invalid_state`,
`checkout_session_expired` and `checkout_session_complete` — for the reason
`Category::Idempotency` carries two: the status comes from the category and
the `code` is what an SDK branches on, and a merchant must be able to tell
"this intent is already processing" from "your payer's checkout is over".

### Using the official Stripe SDKs

A merchant with an existing Stripe integration can point `stripe-node` at
vpay: build it with an empty key and a `config.authenticator` that performs
the `private_key_jwt` handshake.

```js
import Stripe from "stripe";
import { createStripeAuthenticator } from "@vaam-apps/vpay-sdk/stripe";

const stripe = new Stripe("", {
  authenticator: createStripeAuthenticator({
    baseUrl: "https://api.vpay.example",
    clientId: "acme-cameroon",
    privateKey: readFileSync("./merchant-key.pem", "utf8"),
  }),
  host: "api.vpay.example",
  port: "443",
  protocol: "https",
});
```

Two response headers exist for those clients specifically. Every response
carries the request id under **`request-id`** as well as `x-request-id`, with
one value, because `request-id` is the only spelling stripe-node reads when
it populates `err.requestId`. Every response the error renderer produces also
carries **`stripe-should-retry`**, derived from the error's
[`Classify::retry`](../adr/0011-error-modelling.md) — `false` on a `409`,
which stripe-node would otherwise retry unconditionally, and `true` on an
in-flight idempotency key, which it would otherwise never retry. `405` and
`413` are produced above that renderer and carry neither the envelope nor the
header.

`Stripe-Version`, `Stripe-Account`, `Stripe-Context` and the
`X-Stripe-Client-*` headers are accepted and ignored; vpay advertises no
dated API version and echoes none.

**A request field is refused only when ignoring it would change where or when
money moves.** `confirm: true` on create is refused with `param: "confirm"`
rather than dropped, and so are `capture_method` with any value other than
`automatic`, `application_fee_amount`, `transfer_data` and `on_behalf_of` —
each a `400` naming the field, on both POST bodies. vpay has no authorise-now
/ capture-later split and no Connect, so silently dropping one of those would
settle a merchant's money at a time, or to an account, they neither asked for
nor can see in the response. Everything else Stripe sends and vpay does not
implement — `setup_future_usage`, `confirmation_method`, `receipt_email`,
`statement_descriptor`, `customer`, `expand` — is accepted and ignored,
because none of it changes the payment that results. `metadata` is accepted
on both bodies too, but it is not on that list: `metadata` is stored; the
rest are dropped — it is persisted on the intent and comes back on every
read.

**A replayed response carries the advisory the original carried.** Migration
`0025` stores the header's own value beside the status and body
(`idempotency_keys.response_retry`), written from the rendered response's
`HeaderMap` and re-emitted unchanged — never re-derived from the stored
status, which is what ADR-0011 forbids and what would make the replay
disagree with the response it replays. A stored `2xx` carries `NULL` there and
its replay emits no header, because a `2xx` never reaches the error renderer.

[../flows/stripe-sdk-compat.md](../flows/stripe-sdk-compat.md) is the full
list of what carries over and what does not, and what has actually been
proven — `sdks/stripe-compat` drives the real `stripe` package against a real
stack, and CI runs it.

### Not served — the honest 404 stands

| Method | Path | Why |
|---|---|---|
| POST | `/v1/refunds` | no refunds repository, no handler; migration `0017` is the schema only |
| GET | `/v1/balance` | no ledger read path |

Both SDKs can call both. Each returns the `404` envelope to an
authenticated caller, because a `200` would mean someone invented a resource.

`GET /v1/events` was on this list until 2026-09-03 and is now served — see
"Served today" above.

See [../status.md](../status.md) for the tests behind each claim above and
for the state of the evidence.

**Client-side, two SDKs implement this surface** — [`sdks/rust`](../../sdks/rust)
(`vpay-sdk`) and [`sdks/nodejs`](../../sdks/nodejs) (`@vaam-apps/vpay-sdk`). They do
the `private_key_jwt` handshake, token caching, the form-encoded resource
calls in the table below and webhook verification. The exact wire contract
they implement, and the server must serve, is
[../flows/merchant-auth.md](../flows/merchant-auth.md). They are tested
against HTTP stubs of that contract and against the real Authkestra
assertion verifier. Since 2026-09-02 the **Rust** SDK completes the whole
handshake against a real `vpay_api::router` in
`backends/tests/integration/tests/merchant_token_flow.rs`, and since
2026-09-03 it also drives the five served payment-intent methods above
against a real router and a real Postgres in
`backends/tests/integration/tests/payment_intents.rs`, and its
`client.events().list()` against the same in
`backends/tests/integration/tests/webhooks.rs`. Two of its eight resource
methods still have no route to call (`refunds().create()`,
`balance().retrieve()`).

**The Node SDK has now spoken to a vpay, in exactly one respect.** Its
`verifyWebhook` verifies a `Vpay-Signature` this server emitted, in a
subprocess, in `backends/tests/integration/tests/webhooks.rs`
(`the_delivered_signature_verifies_with_the_shipping_node_sdk`) — the header
comes off a real WireMock receiver's request journal, and the same test
asserts the wrong secret is refused. That test **fails** rather than skips
when `node` is missing; CI's `rust` job sets `VPAY_REQUIRE_NODE=1`. Nothing
else in the Node SDK — the handshake, the resource calls — has ever reached a
vpay.

Everything not listed under "Served today" returns a Stripe-shaped 404 naming
vpay honestly, rather than pretending the route exists.

## `/v1/browser` — the payer's browser (Step 5c)

**Five** routes since Step 9, unauthenticated by a bearer token of any kind
and **not** part of `/v1`'s route table (`vpay_api::V1_ROUTES`) or its 401
boundary — its own table is `BROWSER_ROUTES`, its own `.nest`, its own
`.fallback(not_found)`. The property the route table's test pins is not the
count but that **exactly one entry answers a non-`GET` method**: the confirm
that has been there since Step 5c. Step 9 added three reads and no second way
to move money. This is the surface `@vaam-apps/vpay-stripe-js`
(`sdks/stripe-js/`) speaks, because `@stripe/stripe-js` cannot be pointed at
vpay (verified directly — see that package's own README).

| Method | Path | Auth | What it does |
|---|---|---|---|
| GET | `/v1/browser/payment_intents/{id}` | `key` + `client_secret` (query) | The polling endpoint — `waitForPaymentIntent` calls it. |
| POST | `/v1/browser/payment_intents/{id}/confirm` | `key` + `client_secret` (form) | Reaches the same `confirm_once` `/v1` calls, scoped to the one intent the credential names. **Since Step 9 a confirm on an intent an open Checkout Session drives needs no `return_url`** — vpay substitutes the session's own return page, and a `return_url` sent anyway is ignored. With no session the rule is unchanged: a redirect rail requires one. |
| GET | `/v1/browser/checkout/sessions/{id}` | `key` + the **session's** `client_secret` (query) | The `checkout.session` with `payment_intent` **expanded and carrying the intent's own `client_secret`** — what vpay's page reads before it can paint, and only while the session is `open`. Step 9. |
| GET | `/v1/browser/checkout/sessions/{id}/return` | `key` + `t` (the session's `return_token`, query) | The same, with `payment_intent` expanded **without** the intent's secret. Where a redirect rail sends the payer back; it must be a query parameter, because a fragment does not survive a rail's redirect. Step 9. |
| GET | `/v1/browser/checkout/origins` | `key` (query) | `{"origins": [...]}` for the key's tenant, **with no secret at all** — an origin is the merchant's own public website. The checkout app calls it server-side to build `frame-ancestors`. Step 9. |

**The credential model is not `/v1`'s.** A **publishable key** (`pk_test_…`/
`pk_live_…`, `merchant_clients[].publishable_keys` — explicit YAML, never
derived from `merchant_id`) names a tenant and authorises nothing; the
intent's own **`client_secret`** (`pi_…_secret_…`, 160 bits from the OS
CSPRNG, minted once at `create`) is what authorises the one request it is
presented with. Neither is a bearer token and neither can be exchanged for
one. Every credential failure — unknown key, wrong secret, another
merchant's key, unknown intent — answers the byte-identical `404`; a
distinct answer for any one of them would let a caller enumerate merchants
or separate "wrong secret" from "no such intent", which is half of a
guessing attack. `client_secret` itself is rendered only by `create`,
`retrieve`, and these two routes (`PaymentIntentWithSecret`) — never by
`list` or by a webhook body, which keep the same 12-key object every other
`/v1` reader sees. CORS (`allow_origin: Any`, no credentials) is mounted on
this nest and on no other — the merchant `/v1` nest carries none, so a
stolen bearer token cannot even be *attempted* cross-origin. Full detail,
the decisions (D1–D5), the redirect gap, and a Node/Rust SDK typing gap this
step found and did not fix: [../flows/browser-checkout.md](../flows/browser-checkout.md).

**The checkout session reads have their own uniform 404** — unknown key,
session not found, wrong tenant, wrong credential, missing credential, and
**past the session's 24-hour `expires_at` whatever its `status`** — all one
byte-identical `ApiError::NotFound { resource: "checkout session" }`. Neither
read renders the session's `url`: it carries the stronger credential in its
fragment, and echoing it to the holder of the weaker one would be an
escalation. Both carry `merchant: { name }` when the deployment configured a
`display_name` for that merchant, and **omit the member entirely** when it did
not.

**Status: served**, as of Step 5c and extended by Step 9 — see
`docs/status.md` for the machine-checked test counts. **Rate limiting is not in
this process, deliberately (D5)** — an ingress in front of it is an operational
requirement this repository does not enforce, and Step 9 added a second
unauthenticated surface (the checkout page itself) under the same unmet
requirement.

**The page these routes exist for is `frontends/apps/checkout`**, served on
`checkout.public_base_url` and not by `vpay-server`:
[../flows/hosted-checkout.md](../flows/hosted-checkout.md) is its design and
[../runbooks/checkout.md](../runbooks/checkout.md) is how a merchant integrates
it.

## `/dash/v1` — the dashboard API

Authenticated with **OIDC sessions** (authorization code + PKCE), never a
merchant credential of any kind. Called server-side from Next.js only.

Keeping these separate from `/v1` is deliberate, even though both surfaces
are OAuth2-shaped now: a merchant's `client_credentials` token authenticates
a *backend service* with standing payment authority over its own account,
while a dashboard session authenticates a *human*, scoped to one read-only
capability, for as long as they stay logged in. Conflating the two would let
a browser-held credential reach payment-authority endpoints, or a merchant
integration reach staff-only state. See
[ADR-0008](../adr/0008-dashboard-scope.md) and
[ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md).

**Status: not implemented.** No `/dash/v1` route, no `/login`, no
`/authorize`, no session store — `authkestra-engine` is pinned without its
`sql-postgres` feature, so none is compiled in. The signing keys and the
JWKS endpoint that landed on 2026-09-02 serve `/v1` and are not a step
toward this surface being reachable; there is also an unresolved audience
problem in the authorization-code grant that must be settled first. Tracked
as Phase 2b in [../roadmap.md](../roadmap.md); see
[../flows/dashboard-auth.md](../flows/dashboard-auth.md).

## `/provider/{code}/callback` — rail callbacks

Public and unauthenticated by necessity. A callback **never changes state** — it
only enqueues a status query. See [../flows/reconciler.md](../flows/reconciler.md).

~~**Status: not implemented.**~~ **Status: implemented 2026-09-04 (Step 8, lane
C), and never called by a rail.** `POST` only: it resolves the adapter by
`code`, `parse_callback`s the body, finds the charge by its rail reference and
pulls that charge's poll job forward. ~~That is one `UPDATE jobs SET run_at =
now()` and no other write.~~ **Corrected 2026-09-04: two statements, in one
transaction** — `enqueue_in_tx` (`ON CONFLICT DO NOTHING`, a no-op in the
ordinary case, but it inserts a fresh poll job when the charge's job was deleted
or already finished) and `pull_forward_in_tx`, an `UPDATE jobs SET run_at =
now()`. **It writes no charge or intent state.** Unknown rail code → `404` (byte-identical to the router's
fallback); unparseable body → `400`; unknown reference → `202` (a rail must not
be told to retry forever, and a different answer would be an oracle); accepted →
`202`. Body bounded at 16 KiB. **A `GET` on this path is axum's bare `405`, not
the Stripe error envelope** — the same precedent `GET /v1/oauth/token` already
sets. Neither rail signs a callback, so it is a hint on both, and every body the
route has parsed was transcribed from `docs/flows/adapter-*.md` by this
repository's own tests.
