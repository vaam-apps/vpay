# Customers

The merchant-owned record of a payer they expect to see again — `cus_…`, on
`/v1/customers`. It is the object a merchant stores against their own user
record and sends back on every later payment intent, and it is the only object
in vpay whose entire content is **another person's personal data**.

Everything below is about that second sentence. The shape of the object is
three paragraphs; the rules around it are the rest of the document.

---

## What it is

| Field | | |
|---|---|---|
| `id` | `cus_…` | |
| `object` | `"customer"` | |
| `name` | string or `null` | |
| `email` | string or `null` | |
| `phone` | string or `null` | canonicalised — see below |
| `metadata` | ≤ 50 keys, ≤ 40-char key, ≤ 500-char value | |
| `created` | unix **seconds** | |
| `livemode` | boolean | |

Eight keys and no more. **At least one of `name`, `email` and `phone` is
always present**: a customer with none of them names nobody, can never be
matched to a payer, and is the shape an integration creates by accident from a
form with every field blank.

There is no `address` on the object today, and no `line1`/`city`/`country`
columns behind it. That is a gap rather than a decision against one — see
"What is not built".

### `last_used_at` is not on the wire

`customers.last_used_at` exists and is not a field. It is the retention
sweep's clock, vpay moves it whenever an intent or a session names the
customer, and a merchant who could read it would be building on a value whose
motion is vpay's business and whose meaning widens the day invoices exist.
`vpay_api::model`'s
`the_customer_object_is_the_documented_eight_keys` is the tripwire that keeps
it off: adding it would put it in every `customer.*` webhook body, signed and
stored in `events` forever, before anybody wrote it down.

That tripwire **did not exist until 2026-09-07**, and this paragraph named it
anyway. Rendering `last_used_at` on the object was measured against the whole
repository on 2026-09-06 and nothing objected — `vpay-api`, `vpay-sdk`, the
container-backed cases in `backends/tests/integration/tests/customers.rs` and
the Node SDK were all green with the retention clock on the wire. The same
review found the count in this section was wrong: the table above lists eight
rows and the prose said seven.

---

## The three decisions the maintainer took (2026-09-05)

These are recorded decisions, not inferences, and each one is visible in a
specific place:

**1. Phone is the customer identity, and is stored by default.**
There is no opt-in, no configuration flag and no `store_phone` parameter. A
merchant who sends `phone` gets it stored and echoed back. On a mobile money
rail the phone number *is* the payer, so an object that treated it as optional
extra data would be modelling a different market.

**2. Phone-only customers are allowed.**
`at_least_one_identifier` (migration `0034`) requires *one* of the three, not
a name and not an email. A `cus_…` whose only content is a phone number is a
complete customer. `backends/tests/integration/tests/customers.rs`'s
`a_customer_with_only_a_phone_number_is_created_and_reads_back_canonical` is
the proof at the wire, and `postgres_smoke.rs`'s
`a_customer_with_no_name_email_or_phone_is_refused_by_the_database` proves
both halves at the database — the refusal *and* that a phone-only insert is
accepted, which is the direction a "require all three" constraint would break
silently.

**3. Retention is twelve months.**
`vpay_worker::handlers::CUSTOMER_IDLE_AFTER`, and the `sweep_idle_customers`
job below.

### The phone number is stored canonical

`237600000200` — twelve digits, no `+` — whatever the merchant typed.
`+237 6 00 00 02 00`, `237600000200` and `600000200` all store and read back
as the same string, through
`vpay_api::v1::account_holders::canonical_msisdn`, which is the *same*
function that canonicalises an account-holder lookup.

That is a wire contract rather than an implementation detail, and it is the
reason the function is shared rather than copied: it is the value a rail is
given, so a merchant comparing a customer's `phone` against a charge's payer
reference is comparing the same string, and widening the market rule (a second
country code, a second mobile prefix) moves both surfaces at once. Neither SDK
validates it locally, deliberately and identically — a client-side copy of a
market rule refuses offline a number a later server accepts.

---

## Privacy

### No cross-merchant identity

There is no unique index on `phone`, on `email`, or on anything else that
would let two merchants of one deployment discover they share a payer. Two
merchants who both take payments from `237600000200` have two unrelated
`cus_…` and no way to learn that. Cross-merchant identity is a product vpay
does not have, and the absence of the index is what stops it being acquired by
accident.

### `DELETE` is a hard delete

`DELETE /v1/customers/{id}` removes the row. There is no `deleted_at`, no
status column, no `@@soft_delete` on `model Customer`, and a subsequent `GET`
is byte-identical to a `GET` for an id that never existed.

A soft delete would be a lie the schema tells: a row that says "this person
asked to be forgotten" is still the record of that person.
`vpay-db`'s `a_customer_delete_is_a_delete_and_not_a_soft_delete` pins the
rendered statement, because adding `@@soft_delete` is a one-line schema edit
that changes no Rust, compiles, passes `check-schema`, and would leave the API
answering `{deleted: true}` for data it kept.

### …and a customer with payment history **cannot** be deleted

`payment_intents.customer_id` and `checkout_sessions.customer_id` are foreign
keys with `NO ACTION`. A customer any intent or session references refuses
both `DELETE /v1/customers/{id}` (a `409` that says so) and the retention
sweep.

**This is a trade, and it is stated rather than implied: "delete this
customer" is therefore not a complete erasure of the payer.** The payment
record survives, with the payer's identifiers *on the intent's own history*
rather than on the customer. What a merchant can do instead is clear `name`,
`email` and `phone` with an update — which is why an update can clear a field
at all, and why clearing the *last* one is refused rather than silently
leaving a nameless row.

`ON DELETE SET NULL` was the alternative and is worse: it would let the delete
succeed and silently detach a payment from the payer it was taken from, which
is the record a dispute is settled with.

### What is logged, and by whom

| | redacts | why |
|---|---|---|
| `vpay_db::CustomerRow`'s `Debug` | `name`, `email`, `phone` (lengths only) | vpay's logs are not the merchant's. This struct reaches `tracing` fields, `anyhow` chains and every failing assertion. |
| `vpay_sdk::Customer`'s `Debug` | nothing (derived) | the merchant collected this data, already holds it, and is responsible for it. Redacting it would hide their own data from them and do nothing about the copy in their database. |
| `@vaam-apps/vpay-sdk`'s `Customer` | nothing | same. |

The asymmetry is deliberate and is the opposite of `CheckoutSession`'s, where
both SDKs *do* redact: that object carries a **credential**, and printing one
is a compromise the merchant who logged it cannot undo.

### The hosted page's local memory is not this

`frontends/apps/checkout` remembers a payer's number in the browser, on the
payer's own device, for the payer's own convenience. It shares nothing with
this object: no request creates a customer from it, no customer is read into
it, and the two have never been connected. They are named similarly and are
unrelated.

---

## The twelve-month retention sweep

`sweep_idle_customers` — a `jobs.kind` of its own (migration `0034`), seeded
at worker boot on the singleton dedupe key `sweep:customers`, rescheduling
itself hourly.

Each pass deletes every customer that is **both**:

* idle for more than `CUSTOMER_IDLE_AFTER` (365 days), and
* referenced by no payment intent and no checkout session.

"Idle" is measured by `customers.last_used_at`, and **"used" means**: created,
updated, or named by a payment intent or a checkout session. Every one of
those paths calls `vpay_db::Customers::touch_last_used`. A path that is
*missing* does not fail — it makes a live customer look idle, and the sweep
deletes it twelve months later with nothing in any log saying anything unusual
happened. That is why the confirm path stamps as well as the create path, and
why `a_confirm_stamps_the_customers_clock_and_keeps_it_out_of_the_sweep`
exercises it through the router rather than calling the method.

The stamp is **monotonic**: `touch_last_used` filters on `last_used_at < now`,
so a vpay process whose clock is behind cannot rewind a customer's retention
clock. Two processes do not share a clock and the horizon is twelve months, so
a rewind is not a rounding error — it is the difference between surviving a
pass and not.

The delete and its `customer.deleted` event are **one transaction**. A crash
between them would erase a merchant's customer with nobody ever told, and
there is no sweep over "customers deleted without an event" and no way to
build one, because the row that would prove it is gone.

### Why it is its own job kind

It runs on `sweep_expired`'s schedule and its healthy answer is zero too,
which is the argument that put checkout-session expiry *inside* that job. What
separates this one is what a failure means: `sweep_expired`'s three statements
are bounded deletes of vpay's own bookkeeping, and this one erases a
merchant's personal-data records and tells them about it. Sharing a job would
report a failing customer sweep as "the housekeeping sweep is unhealthy", with
an idempotency-key count it has nothing to do with in the same log line, and
would put a merchant-visible event behind the same lease as an internal
delete.

### `customer.deleted` is the only way a merchant learns

A hard delete is unobservable by polling: the object is gone, and a `GET`
afterwards is byte-identical to a `GET` for an id that never existed. So the
event's `data.object` is the customer **as it stood immediately before the
delete**, including the payer's `name`, `email` and `phone` — which is the
point rather than a leak. It is the same personal data the merchant gave vpay
and could have read a moment earlier, sent over a signed body to endpoints
they configured, and after the delete there is nothing else to read.

---

## The API

| Method | Path | |
|---|---|---|
| `POST` | `/v1/customers` | `name`, `email`, `phone`, `metadata[…]` |
| `GET` | `/v1/customers/{id}` | |
| `POST` | `/v1/customers/{id}` | the update — Stripe has no `PUT`/`PATCH` |
| `GET` | `/v1/customers` | `limit`, `starting_after`, `ending_before` |
| `DELETE` | `/v1/customers/{id}` | `{"id": …, "object": "customer", "deleted": true}` |

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

`metadata` has two states rather than three, and that is also the contract: it
is **merged key-wise**, and a key sent empty is removed. The merge bounds the
*result* at fifty keys, not the request — otherwise a merchant could add one
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
([../api/README.md](../api/README.md) said so at the time); a vpay predating
2026-09-06 answers `200` with the field absent rather than refusing, which is
why both SDKs model it as optional.

A session inherits its intent's customer when the request omits one. A session
whose `customer` *disagrees* with its intent's is refused: a session and the
intent it drives naming two different payers is a contradiction, not a
preference between two answers, and the merchant is the only one who knows
which they meant.

The id is rendered, never the expanded object. `expand` is not implemented,
and rendering the customer unasked would put a payer's name, email and phone
number into every `payment_intent.*` webhook body.

---

## Where it lives

| | |
|---|---|
| Table | `backends/migrations/0034_create-customers.sql` |
| Model | `schemas/vpay.cstack`, `model Customer` |
| Repository | `backends/crates/vpay-db/src/customers.rs` |
| API | `backends/crates/vpay-api/src/v1/customers.rs` |
| Sweep | `backends/crates/vpay-worker/src/handlers.rs`, `sweep_idle_customers` |
| SDKs | `sdks/rust/src/resources.rs`, `sdks/nodejs/src/resources/customers.ts` |

`customers` is the first vpay table **born** with a `schemas/vpay.cstack`
model rather than acquiring one afterwards, and that is what lets every column
CrateStack may write carry no DB `DEFAULT` — the condition migration `0033`
had to create for `providers` after the fact. Two of the seven repository
methods run through the generated data layer (`touch_last_used`, `delete`),
and they are the two where being wrong is irreversible. The other five are
hand-written statements because of one column, `metadata`;
[../reference/vpay-db.md](../reference/vpay-db.md) carries that argument in
full.

---

## Status

**Built and proven against a real Postgres and the shipping router, worker and
SDKs (2026-09-06, S4a).** `backends/tests/integration/tests/customers.rs` is
twelve cases; `postgres_smoke.rs` adds three at the schema; `vpay-db`'s own
module adds seven with no container.

**What is not built, and is a gap rather than a decision against it:**

- **`address`.** Stripe's customer has one and this does not. Six nullable
  text columns are perfectly expressible and were left out to keep S4a's first
  cut to the fields the maintainer's decisions are about; nothing here
  forecloses it. Neither SDK carries the field, so adding it later is additive
  in both.
- **`customer.created` and `customer.updated`.** Both are Stripe event types,
  neither is in `type_is_a_documented_event`, and **nothing writes them**.
  That is deliberate and is migration `0023`'s rule applied: the vocabulary
  moves in lockstep with the code that writes it, and `POST /v1/customers` is
  a single statement on the pool — emitting an event means putting the write
  and the event in one transaction, which is a change to the shape of two
  repository methods rather than a line in a list. A merchant keeping an
  external mirror of their customers therefore has to poll. Recorded as a
  dated ⛔/⛔ row in [../sdks/parity.md](../sdks/parity.md), owned by the vpay
  maintainers rather than the SDK ones.
- **Invoices.** The definition of "used" above names two referencing objects
  and would name three. Invoices do not exist
  ([../status.md](../status.md)), so `Customers::idle_since`'s and
  `delete_idle`'s shared `NOT EXISTS` guard is closed at two — and
  `the_sweep_guard_names_every_table_that_can_reference_a_customer` asserts
  the count, so the day an invoice table lands, the omission is a test failure
  rather than a customer swept out from under a live invoice.
- **No `email` filter on the list.** Stripe's takes one. A filter on a payer
  identifier turns the list into a lookup, and a lookup by email over a table
  holding one merchant's payers is one scoping mistake away from being a
  lookup over everybody's. The shape that would make it safe — the filter in
  the same `WHERE` as `merchant_id`, as `checkout_sessions`' `payment_intent`
  filter is — is available whenever it is wanted.
- **Neither SDK has run against a live vpay for this resource.** Every server
  in their customer cases is a stub; the integration suite drives the real one
  but not through either SDK. Dated ⛔/⛔ in
  [../sdks/parity.md](../sdks/parity.md), exactly as the Checkout Session and
  account-holder rows are.
- **No deployment has ever run the retention sweep.** It is proven against a
  real Postgres through the real worker loop with a horizon this suite
  controls; no vpay has been up for twelve months.
