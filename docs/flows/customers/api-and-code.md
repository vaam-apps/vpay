# Customers — the API, tenancy, and where the code lives

_Split out of [docs/flows/customers.md](../customers.md) on 2026-09-11 by exp57, which broke a 888-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## The API

| Method   | Path                 |                                                                         |
| -------- | -------------------- | ----------------------------------------------------------------------- |
| `POST`   | `/v1/customers`      | `name`, `email`, `phone`, `address[…]`, `metadata[…]`                   |
| `GET`    | `/v1/customers/{id}` | answers an erased customer too, with `deleted: true`                    |
| `POST`   | `/v1/customers/{id}` | the update — Stripe has no `PUT`/`PATCH`; a `409` on an erased customer |
| `GET`    | `/v1/customers`      | `limit`, `starting_after`, `ending_before`                              |
| `DELETE` | `/v1/customers/{id}` | `{"id": …, "object": "customer", "deleted": true}` — always, or a `404` |

An erased customer **is** in `GET /v1/customers`. Excluding it would need a
filter, and a filter would make `has_more` and the cursors describe a
different set from the one the rows are in — while leaving a `cus_…` that
`GET /v1/customers/{id}` answers and the list denies. It is a row; it is
listed.

Every write carries an `Idempotency-Key`, `DELETE` included — and that verb is
where a replay is most confusing without one: the second call would otherwise
answer `404` for a deletion that succeeded, which a merchant retrying a
timed-out request cannot tell from "somebody else deleted it".

### Update has three states per field

`undefined`/absent leaves a field alone, a value sets it, and **the empty
string clears it** (`name=`). All three are Stripe's contract and a merchant
depends on all three: without the last one, an email a payer asked to have
removed cannot be removed without deleting the whole customer.

Both SDKs carry the distinction in their types — `Option<Option<String>>` in
Rust, `string | null | undefined` in TypeScript — and both prove it by
asserting the **body** rather than the type, because collapsing two of the
three is a one-word edit that compiles.

`address` has the same three states with one difference, and it is the one a
merchant can get wrong silently: an address object **replaces** the stored
address rather than merging into it. See "The address" above.

`metadata` has two states rather than three, and that is also the contract: it
is **merged key-wise**, and a key sent empty is removed. The merge bounds the
_result_ at fifty keys, not the request — otherwise a merchant could add one
key fifty times.

Clearing the **last** identifier is refused with a `400` naming all three
parameters. It has to be decided above the statement: the database's
`at_least_one_identifier` is a `23514`, and `vpay_db::classify_write` routes a
CHECK violation to `Category::Storage`, i.e. a `503` telling a merchant to
wait for a database that is perfectly healthy.

### Tenancy

Every query is merchant-scoped **in SQL**. Another merchant's `cus_…` and an
id that never existed are the same `404`, byte for byte, on retrieve, update
and delete; naming another merchant's `cus_…` as `customer` on an intent or a
session is the same `400` a nonexistent one gets, with the same sentence. A
distinct answer anywhere would make the field an oracle for which customers
exist under some other tenant.

### `customer` on a payment intent and a checkout session

`POST /v1/payment_intents` and `POST /v1/checkout/sessions` accept
`customer=cus_…`, store it, and render it. It was **accepted and dropped**
from Step 5b until migration `0034` gave it somewhere to point
([../api/README.md](../../api/README.md) said so at the time); a vpay predating
2026-09-06 answers `200` with the field absent rather than refusing, which is
why both SDKs model it as optional.

A session inherits its intent's customer when the request omits one. A session
whose `customer` _disagrees_ with its intent's is refused: a session and the
intent it drives naming two different payers is a contradiction, not a
preference between two answers, and the merchant is the only one who knows
which they meant.

The id is rendered, never the expanded object. `expand` is not implemented,
and rendering the customer unasked would put a payer's name, email and phone
number into every `payment_intent.*` webhook body.

All four of those session rules are proved by
`a_sessions_customer_is_inherited_supplied_or_a_refused_contradiction`, added
by the sabotage review on 2026-09-07 — S4a shipped them untested, and removing
the contradiction check left the whole thirty-one-case
`checkout_sessions.rs` suite green.

**Neither SDK can send or read a session's `customer`.**
`CreateCheckoutSessionParams` has no such field in either language and neither
`CheckoutSession` type carries the key the server returns, so this paragraph
describes the HTTP API and not what a merchant using `@vaam-apps/vpay-sdk` or
`vpay-sdk` can reach. The intent's `customer` _is_ in both. Dated ⛔/⛔ rows in
[../sdks/parity.md](../../sdks/parity.md), owned by the SDK maintainers.

---

## Where it lives

|            |                                                                                                 |
| ---------- | ----------------------------------------------------------------------------------------------- |
| Table      | `backends/migrations/0034_create-customers.sql`, `0041_customers-address-and-anonymisation.sql` |
| Model      | `schemas/vpay.cstack`, `model Customer`                                                         |
| Repository | `backends/crates/vpay-db/src/customers.rs`                                                      |
| API        | `backends/crates/vpay-api/src/v1/customers.rs`                                                  |
| Sweep      | `backends/crates/vpay-worker/src/handlers.rs`, `sweep_idle_customers`                           |
| SDKs       | `sdks/rust/src/resources.rs`, `sdks/nodejs/src/resources/customers.ts`                          |

`customers` is the first vpay table **born** with a `schemas/vpay.cstack`
model rather than acquiring one afterwards, and that is what lets every column
CrateStack may write carry no DB `DEFAULT` — the condition migration `0033`
had to create for `providers` after the fact — and it is what let migration
`0041` add seven columns for **zero** column-level drift lines. Two of the
seven repository methods run through the generated data layer
(`touch_last_used`, and the hard-delete half of `erase_in_tx`), and they are
the two where being wrong is irreversible. The other five are
hand-written statements because of one column, `metadata`;
[../reference/vpay-db.md](../../reference/vpay-db.md) carries that argument in
full.

---
