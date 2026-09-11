# Merchant auth — the `/v1` resource contract the SDKs implement

_Split out of [docs/flows/merchant-auth.md](../merchant-auth.md) on 2026-09-11 by exp57, which broke a 748-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## The `/v1` resource contract the SDKs implement

Stripe-shaped, per [`docs/api/README.md`](../../api/README.md): form-encoded
request bodies, Stripe's object model, Stripe's error envelope, Stripe's
idempotency semantics. Field names are Stripe's own wherever a Stripe field
exists, so a merchant's existing types keep working; nothing below is a vpay
invention beyond the rails' payment-method names.

### Encoding

Request bodies are `application/x-www-form-urlencoded`, bracket-nested the
way Stripe's official SDKs encode them:

| Shape         | Wire form                                                                                                                                                                                                                                       |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| scalar        | `amount=5000`                                                                                                                                                                                                                                   |
| nested object | `metadata[order_id]=1234`, `payment_method_data[mtn_momo][msisdn]=237670000000`                                                                                                                                                                 |
| array         | `payment_method_types[0]=mtn_momo&payment_method_types[1]=orange_money` (indexed, as `stripe-node`/`stripe-rust` send; the server must also accept the unindexed `payment_method_types[]=…` form the curl examples use, exactly as Stripe does) |
| boolean       | `true` / `false`                                                                                                                                                                                                                                |
| integer       | decimal, no separators; **amounts are integer minor units** ([money.md](../money.md)) — both SDKs refuse a non-integer amount before it reaches the wire                                                                                        |
| currency      | lowercase on the wire (`xaf`), matching Stripe                                                                                                                                                                                                  |

`GET` parameters use the same encoder into the query string. Responses are
JSON.

### Headers

| Header            | When         | Value                                                                                          |
| ----------------- | ------------ | ---------------------------------------------------------------------------------------------- |
| `Authorization`   | always       | `Bearer <access_token>`                                                                        |
| `Idempotency-Key` | every `POST` | caller-supplied, else a UUIDv4 generated per call — so a network retry can never double-create |
| `Content-Type`    | `POST`       | `application/x-www-form-urlencoded`                                                            |
| `Accept`          | always       | `application/json`                                                                             |
| `User-Agent`      | always       | `vpay-sdk-rust/<version>` / `vpay-sdk-node/<version>`                                          |

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

| What the caller did                              | What they get                                                                                                                                  |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| Replayed a key whose first request finished      | the **stored response body, byte for byte**, with its original status — `a_replayed_idempotency_key_returns_the_same_object_and_no_second_row` |
| Replayed a key with a different body             | `400` `idempotency_error`/`idempotency_key_in_use` — `a_reused_key_with_a_different_body_is_the_400_envelope`                                  |
| Sent a key whose first request is still running  | `400` `idempotency_error`/`idempotency_key_in_flight` — `a_key_whose_first_request_is_still_running_is_answered_with_its_own_code`             |
| Retried after a `5xx`                            | the key was **released**; the retry re-executes — `a_5xx_releases_its_idempotency_key_so_the_retry_re_executes`                                |
| Replayed after the deployment changed underneath | the original answer, unchanged — `a_replay_survives_the_rail_being_disabled`                                                                   |

Two orderings matter and are deliberate. The key is claimed **before** a
create body is validated, so a replay short-circuits before a rule that has
since changed can be re-evaluated; and a _validation_ failure **releases**
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
[ADR-0011](../../adr/0011-error-modelling.md) derives the status from the
error's `Category` and `Category::Idempotency` is `400`; splitting one
Stripe `type` across two statuses is an ADR-level change, left as a
maintainer decision. Branch on `code`, which is distinct either way.

**Not built:** nothing sweeps the table on a schedule.
`vpay_db::Idempotency::sweep_expired` exists and `vpay-server` calls it once
at boot as a stopgap; there is no worker job loop, so a long-lived
deployment grows `idempotency_keys` monotonically between restarts.

### Resources

**Served** marks what a running `vpay-server` actually answers, re-measured
against `vpay_api::V1_ROUTES` on **2026-09-06** (issue #45 rebased onto issue
#47; the table below carries both rows). Everything marked `⛔ 404` is
implemented by both SDKs and by no server route: an authenticated call gets
the honest `404`.

`GET /v1/refunds/{id}` is served and `POST /v1/refunds` is not, which is an
unusual pair and a deliberate one (issue #45). Creating a refund needs
`ProviderAdapter::refund`, which is `NotImplemented` on MTN (refunds are the
Disbursements product) and `Unsupported` on Orange; **reading** one is the
authoritative read `docs/flows/provider-port.md` requires of every money
movement, and without it a merchant holding a `re_…` has neither a call nor
an event — `charge.refunded` and `charge.refund.updated` are emitted by
nothing ([../status.md](../../status.md)) and webhook delivery is at-least-once
and unordered in any case ([webhooks.md](../webhooks.md)).

Two corrections this re-measurement produced, both of documents rather than
of code. The `/v1/events` row read `⛔ 404` and had been wrong since Step 5
served that resource on 2026-09-03; it and `GET /v1/events/{id}` are now
listed as what `V1_ROUTES` mounts. And **this table does not list the
Checkout Session routes at all** — `POST`/`GET /v1/checkout/sessions`,
`GET /v1/checkout/sessions/{id}` and
`POST /v1/checkout/sessions/{id}/expire` are served, and are documented with
their refusals in [hosted-checkout.md](../hosted-checkout.md); they are not
repeated here rather than being restated in a second place that can drift.

**Re-measured again on 2026-09-06** for S4a's five `/v1/customers` routes,
which _are_ listed below — unlike the Checkout Session ones, because these
five are the whole of that resource's surface and there is nothing left over
for a second document to own. The privacy rules around them, which no table
can carry, are [customers.md](../customers.md).

| Method   | Path                               | Request fields                                                                                       | Returns                             | Served                                                                                                                                                                                                                                                                                                                                                                                         |
| -------- | ---------------------------------- | ---------------------------------------------------------------------------------------------------- | ----------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST`   | `/v1/payment_intents`              | `amount`, `currency`, `payment_method_types[]`, `metadata[…]`, `description`, `customer`             | `payment_intent`                    | ✅ — `customer` was **accepted and dropped** until 2026-09-06 (S4a) and is now stored and rendered; see [customers.md](../customers.md)                                                                                                                                                                                                                                                        |
| `GET`    | `/v1/payment_intents/{id}`         |                                                                                                      | `payment_intent`                    | ✅                                                                                                                                                                                                                                                                                                                                                                                             |
| `POST`   | `/v1/payment_intents/{id}/confirm` | `payment_method_data[type]`, `payment_method_data[mtn_momo][msisdn]` (push), `return_url` (redirect) | `payment_intent`                    | 🟡 reaches a rail over HTTP: `processing` / `requires_action`, `409 charge_declined`, `502`. 🟡 because that rail has only ever been a WireMock stub                                                                                                                                                                                                                                           |
| `POST`   | `/v1/payment_intents/{id}/cancel`  |                                                                                                      | `payment_intent`                    | ✅                                                                                                                                                                                                                                                                                                                                                                                             |
| `GET`    | `/v1/payment_intents`              | `limit`, `starting_after`, `ending_before`                                                           | `list` of `payment_intent`          | ✅                                                                                                                                                                                                                                                                                                                                                                                             |
| `POST`   | `/v1/refunds`                      | `payment_intent`, `amount` (omit for full), `reason`, `metadata[…]`                                  | `refund`                            | ⛔ 404                                                                                                                                                                                                                                                                                                                                                                                         |
| `GET`    | `/v1/refunds/{id}`                 |                                                                                                      | `refund`                            | ✅                                                                                                                                                                                                                                                                                                                                                                                             |
| `GET`    | `/v1/events`                       | `limit`, `starting_after`, `ending_before`, `type`                                                   | `list` of `event`                   | ✅ since 2026-09-03 (Step 5), merchant-scoped and newest first (`events_are_listed_newest_first_scoped_to_the_merchant`). **`type` is accepted and ignored**, not refused — a filtered call gets an unfiltered page ([../status.md](../../status.md))                                                                                                                                          |
| `GET`    | `/v1/events/{id}`                  |                                                                                                      | `event`                             | ✅ since 2026-09-03 (Step 5). A foreign merchant's id is the same `404` a nonexistent one gets, byte for byte (`events_get_by_id_is_merchant_scoped`)                                                                                                                                                                                                                                          |
| `POST`   | `/v1/customers`                    | `name`, `email`, `phone`, `metadata[…]`                                                              | `customer`                          | ✅ **New 2026-09-06 (S4a).** At least one of `name`/`email`/`phone` is required and **`phone` alone is enough** — the maintainer's decision of 2026-09-05 ([customers.md](../customers.md))                                                                                                                                                                                                    |
| `GET`    | `/v1/customers/{id}`               |                                                                                                      | `customer`                          | ✅                                                                                                                                                                                                                                                                                                                                                                                             |
| `POST`   | `/v1/customers/{id}`               | `name`, `email`, `phone`, `metadata[…]`                                                              | `customer`                          | ✅ The update. A field sent **empty** is _cleared_, which is a different request from omitting it; clearing the last identifier is a `400`                                                                                                                                                                                                                                                     |
| `GET`    | `/v1/customers`                    | `limit`, `starting_after`, `ending_before`                                                           | `list` of `customer`                | ✅ No `email` filter, deliberately ([customers.md](../customers.md))                                                                                                                                                                                                                                                                                                                           |
| `DELETE` | `/v1/customers/{id}`               |                                                                                                      | `{"id", "object", "deleted": true}` | ✅ The erasure, and the only `DELETE` on this API — so the only route where the `Idempotency-Key` this table requires on every write is carried on a verb with no body. Since 2026-09-10 it never answers a `409`: a customer nothing references is hard-deleted, one an intent, a session or an invoice references is anonymised in place (migration `0041`, [customers.md](../customers.md)) |
| `GET`    | `/v1/balance`                      |                                                                                                      | `balance`                           | ⛔ 404                                                                                                                                                                                                                                                                                                                                                                                         |
| `GET`    | `/v1/account_holders`              | `msisdn`, `payment_method_type`                                                                      | `account_holder`                    | ✅ **New 2026-09-05 (issue #47).** Reaches the rail over HTTP; 🟡 in the sense every rail claim here is 🟡 — that rail has only ever been a WireMock stub. `payments:read` is enough. Nothing is persisted, and the response carries a name and nothing else ([account-holder-lookup.md](../account-holder-lookup.md))                                                                         |

### Objects

`payment_intent`

| Field                  | Type                    | Notes                                                                                                                                                                                                                       |
| ---------------------- | ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`                   | string                  | `pi_…`                                                                                                                                                                                                                      |
| `object`               | `"payment_intent"`      |                                                                                                                                                                                                                             |
| `amount`               | integer                 | minor units                                                                                                                                                                                                                 |
| `currency`             | string                  | lowercase                                                                                                                                                                                                                   |
| `status`               | enum                    | exactly `vpay_core::state::IntentStatus`'s five values: `requires_payment_method`, `requires_action`, `processing`, `succeeded`, `canceled` — there is no `failed` status ([payment-lifecycle.md](../payment-lifecycle.md)) |
| `payment_method_types` | string[]                | rail codes: `mtn_momo`, `orange_money`                                                                                                                                                                                      |
| `next_action`          | object or null          | redirect rails only: `{ "type": "redirect_to_url", "redirect_to_url": { "url": "…", "return_url": "…" } }`                                                                                                                  |
| `last_payment_error`   | object or null          | `{ "code": <failure taxonomy>, "message": "…" }` — `code` is one of [failures.md](../failures.md)'s closed vocabulary                                                                                                       |
| `metadata`             | object of string→string |                                                                                                                                                                                                                             |
| `description`          | string or null          |                                                                                                                                                                                                                             |
| `created`              | integer                 | Unix seconds                                                                                                                                                                                                                |
| `livemode`             | boolean                 |                                                                                                                                                                                                                             |

`refund`: `id` (`re_…`), `object: "refund"`, `amount`, `currency`,
`payment_intent`, `status` (`pending` \| `succeeded` \| `failed` \| `canceled`),
`reason` (string or null), `metadata`, `created`, `fee` (integer or null).

Ten fields since 2026-09-05 ([issue #46](https://github.com/vaam-apps/vpay/issues/46)).
`fee` is **what the movement cost us**, in integer minor units of the refund's
own `currency` — [money.md](../money.md)'s rule, so there is no second currency
field and no float. It is **not** deducted from `amount`: `amount` is the
payer's money and a buyer's refund never nets a fee.

Its nullability is the point of the field, not a detail of it:

| `fee`  | means                                          |
| ------ | ---------------------------------------------- |
| `null` | the rail reported no fee. **Not zero.**        |
| `0`    | the rail reported that the movement was free   |
| _n_    | the rail charged _n_ minor units of `currency` |

A merchant building a settlement statement shows a line for `0` and shows
nothing for `null`. Substituting one for the other — `fee ?? 0` in TypeScript,
`fee.unwrap_or(0)` in Rust — puts a number nobody measured in front of a
merchant, which is the reason the field was asked for: the integrator who
filed the issue had no way to say "unknown" and was sending a hardcoded `0`.

The key is always present on the wire, `null` and all, like every other
documented key on every object here. Both SDKs model it as optional anyway so
that a vpay older than the field still decodes; see
[../sdks/parity.md](../../sdks/parity.md) for how each spells it and which test
proves it.

**Every `refund` this deployment could produce carries `fee: null`**, and will
until a rail reports one. Orange's Web Payment product documents no refund API
and MTN refunds are the Disbursements product vpay has never been issued a
credential for — see [../status.md](../../status.md), which names what has to
exist before that changes. The same value appears on `charge.refunded` and
`charge.refund.updated`, because `data.object` is this object.

`event`: `id` (`evt_…`), `object: "event"`, `type` (one of the real Stripe
event types [webhooks.md](../webhooks.md) commits to), `created`, `livemode`,
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
