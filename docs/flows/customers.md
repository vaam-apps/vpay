# Customers

The merchant-owned record of a payer they expect to see again — `cus_…`, on
`/v1/customers`. It is the object a merchant stores against their own user
record and sends back on every later payment intent, and it is the only object
in vpay whose entire content is **another person's personal data**.

Everything below is about that second sentence. The shape of the object is
three paragraphs; the rules around it are the rest of the document.

---

## What it is

| Field      |                                            |                            |
| ---------- | ------------------------------------------ | -------------------------- |
| `id`       | `cus_…`                                    |                            |
| `object`   | `"customer"`                               |                            |
| `name`     | string or `null`                           |                            |
| `email`    | string or `null`                           |                            |
| `phone`    | string or `null`                           | canonicalised — see below  |
| `address`  | object of six components, or `null`        | see below                  |
| `metadata` | ≤ 50 keys, ≤ 40-char key, ≤ 500-char value |                            |
| `created`  | unix **seconds**                           |                            |
| `livemode` | boolean                                    |                            |
| `deleted`  | `true`, **or the key is absent**           | only on an erased customer |

Nine keys, and a tenth only when the payer has been erased. **At least one of
`name`, `email` and `phone` is always present**: a customer with none of them
names nobody, can never be matched to a payer, and is the shape an integration
creates by accident from a form with every field blank. An address does not
count — it does not name anybody, so a create carrying only an address is
refused exactly as one carrying nothing is.

`vpay_api::model`'s `the_customer_object_is_the_documented_nine_keys` is what
holds the count. It said _eight_ until 2026-09-10; `address` is the ninth.

### The address (2026-09-10, [issue #67](https://github.com/vaam-apps/vpay/issues/67))

`address` is **one nested object with six components**, every one nullable and
every one rendered — `line1`, `line2`, `city`, `state`, `postal_code`,
`country` — or `null` when the customer has no address at all. It is Stripe's
shape, and it is six `TEXT` columns behind the object rather than a `JSONB`
one: a JSONB column would be invisible to `cratestack migrate baseline` in
both directions and could not be declared on `model Customer` without
`Value::from_plain_json`'s number demotion (`../reference/vpay-db.md`).

Three rules, and none of them is obvious from the shape:

**`country` is ISO 3166-1 alpha-2, upper-cased on the way in.** `cm` and `CM`
are one country, and storing them as typed would leave vpay holding two
spellings — the same wire contract `phone`'s canonicalisation is, for the same
reason. `CMR`, `237` and `Cameroon` are a `400` naming `address`. The _shape_
is checked and the code is not resolved against any list: the list changes
(South Sudan in 2011, the Netherlands Antilles out in 2010), nothing in vpay
resolves a country code to anything, and a CHECK that had to be migrated
whenever the world did would refuse a merchant's perfectly real address until
somebody shipped a release.

**An update replaces the address whole; it never merges components.** A
request naming `address[line1]` and not `address[city]` clears the city. That
is a decision rather than an omission, and the argument is about failure
modes: a merchant correcting a payer's street who left `city` out meant "this
is the address", and a component-wise merge would keep the old city beside the
new street — an address that was never anybody's, assembled by vpay out of
two requests, discovered by whoever eventually posts something to it.
Replacement fails visibly on the next read.

**`address=` clears it**, exactly as `name=` clears a name; an absent key
leaves it alone. Both SDKs carry the three states (`Option<Option<…>>`,
`AddressParams | null | undefined`) and both prove them by asserting the
**body**.

### `last_used_at` is not on the wire

`customers.last_used_at` exists and is not a field. It is the retention
sweep's clock, vpay moves it whenever an intent or a session names the
customer, and a merchant who could read it would be building on a value whose
motion is vpay's business and whose meaning widens the day invoices exist.
`vpay_api::model`'s
`the_customer_object_is_the_documented_nine_keys` is the tripwire that keeps
it off — and `anonymized_at`, added by migration `0041`, is in the same list
of internals it refuses: adding it would put it in every `customer.*` webhook body, signed and
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
rail the phone number _is_ the payer, so an object that treated it as optional
extra data would be modelling a different market.

**2. Phone-only customers are allowed.**
`at_least_one_identifier` (migration `0034`) requires _one_ of the three, not
a name and not an email. A `cus_…` whose only content is a phone number is a
complete customer. `backends/tests/integration/tests/customers.rs`'s
`a_customer_with_only_a_phone_number_is_created_and_reads_back_canonical` is
the proof at the wire, and `postgres_smoke.rs`'s
`a_customer_with_no_name_email_or_phone_is_refused_by_the_database` proves
both halves at the database — the refusal _and_ that a phone-only insert is
accepted, which is the direction a "require all three" constraint would break
silently.

**3. Retention is twelve months.**
`vpay_worker::handlers::CUSTOMER_IDLE_AFTER`, and the `sweep_idle_customers`
job below.

### The phone number is stored canonical

`237600000200` — twelve digits, no `+` — whatever the merchant typed.
`+237 6 00 00 02 00`, `237600000200` and `600000200` all store and read back
as the same string, through
`vpay_api::v1::account_holders::canonical_msisdn`, which is the _same_
function that canonicalises an account-holder lookup.

That is a wire contract rather than an implementation detail, and it is the
reason the function is shared rather than copied: it is the value a rail is
given, so a merchant comparing a customer's `phone` against a charge's payer
reference is comparing the same string, and widening the market rule (a second
country code, a second mobile prefix) moves both surfaces at once. Neither SDK
validates it locally, deliberately and identically — a client-side copy of a
market rule refuses offline a number a later server accepts.

### …but the phone number is **not** a key, and does not deduplicate

"Phone is the customer identity" (decision 1) is a statement about what a
`cus_…` _means_, not about uniqueness, and the difference is worth spelling
out because the natural reading is the wrong one. Two `POST /v1/customers`
with the same phone number, under the same merchant, create **two customers**
with two ids. vpay does not look for an existing row and does not answer with
one.

That is Stripe's behaviour and it is deliberate here for the reason the next
section gives: the index that would make a phone number unique is the index
that would make cross-merchant identity possible, and there is no way to have
the first without either the second or a per-merchant partial index nobody has
asked for. The merchant owns the mapping from their user to a `cus_…` and is
the only party that can — vpay has no view of which of two rows is the "real"
payer.

What canonicalisation buys, then, is narrower than deduplication and is still
worth having: `+237 6 00 00 02 00` and `600000200` sent as _one_ customer's
phone across a create and a later update do not leave two spellings in the
column, and a merchant comparing a customer's `phone` against a charge's payer
reference is comparing the same string. It does not stop a merchant creating
the same payer twice. The `Idempotency-Key` on `POST /v1/customers` is what
stops a _retry_ doing so
(`a_replayed_create_answers_the_stored_customer_and_a_reused_key_is_refused`);
two deliberate creates are two customers.

---

## Privacy

### No cross-merchant identity

There is no unique index on `phone`, on `email`, or on anything else that
would let two merchants of one deployment discover they share a payer. Two
merchants who both take payments from `237600000200` have two unrelated
`cus_…` and no way to learn that. Cross-merchant identity is a product vpay
does not have, and the absence of the index is what stops it being acquired by
accident.

### `DELETE` erases the payer, in one of two shapes

Rewritten 2026-09-10 ([issue #68](https://github.com/vaam-apps/vpay/issues/68),
[issue #96](https://github.com/vaam-apps/vpay/issues/96) item 2, migration
`0041`). What this section said before is at the end of it, because the change
is a correction and not an extension.

`DELETE /v1/customers/{id}` always succeeds or answers the uniform `404`.
There is no `409` any more. What it does depends on one fact about the
customer, and the database decides it inside the transaction:

**A customer nothing references is hard-deleted.** The row is gone. There is
no `deleted_at`, no status column and no `@@soft_delete` on `model Customer`,
and a subsequent `GET` is byte-identical to a `GET` for an id that never
existed. A soft delete would be a lie the schema tells: a row that says "this
person asked to be forgotten" is still the record of that person.
`vpay-db`'s `a_customer_delete_is_a_delete_and_not_a_soft_delete` pins the
rendered statement, because adding `@@soft_delete` is a one-line schema edit
that changes no Rust, compiles, passes `check-schema`, and would leave the API
answering `{deleted: true}` for data it kept.

**A customer a payment intent, a checkout session or an invoice references is
anonymised.** `payment_intents.customer_id`, `checkout_sessions.customer_id`
(migration `0034`) and `invoices.customer_id` (`0036`) are foreign keys with
`NO ACTION`, and they stay that way: vpay never detaches a payment from the
payer it was taken from, because that is the record a dispute is settled with.
`ON DELETE SET NULL` was the alternative and is worse. So the row stays and
the **payer** goes: every one of the nine identifier columns becomes the
literal `[redacted]`, `anonymized_at` is stamped, and the object then renders
`deleted: true`.

`metadata` is untouched. It is the merchant's own key/value data, not the
payer's, and destroying it would be vpay deleting a merchant's records to keep
a promise made to somebody else.

A `GET` afterwards answers `200`, not `404`, so a `cus_…` stored in a
merchant's own database goes on resolving. The customer cannot be updated
(`409`) or attached to a new payment (`409`): both would put a payer back on a
record there is deliberately nothing left in, and
`at_least_one_identifier` would not object, because `[redacted]` is not NULL.
A second `DELETE` is a no-op answering the same `{deleted: true}`, and emits
no second event.

**The asymmetry — a `404` in one case and a `200` with `deleted: true` in the
other — is Stripe's too**, and it is worth stating rather than smoothing over:
a merchant cannot predict which they will get without knowing whether the
customer ever paid.

### An anonymised row is not a soft delete, and the database is what says so

The difference is the whole reason `anonymized_at` is allowed to exist beside
the no-soft-delete rule. A soft delete keeps the record of the person and
hides it behind a predicate. An anonymised row holds **nothing** of theirs —
and that is not a promise two call sites remember, it is migration `0041`'s
`anonymized_customers_carry_the_marker`, which refuses any row whose
`anonymized_at` is set and whose nine identifier columns are not all
`[redacted]`.

All nine are written, including components the payer never filled in, because
_which fields a record carried is itself information about the person_.

`at_least_one_identifier` was **not** relaxed for this and does not need to
be: the marker is not NULL. It now also backstops the erasure in the one
direction that matters — an erasure that NULLed the three identifiers rather
than marking them is refused outright.

### The erasure covers every copy vpay kept, not just the row

This is the part [issue #68](https://github.com/vaam-apps/vpay/issues/68) was
written about and the part it got wrong. The issue says deletion should
"redact identifiers on retained intents and sessions". Read against the
schema, an intent has never carried a payer identifier: it carries an amount,
a status and a `cus_…`. The copies that actually survived a deletion were
somewhere else, and no code named them:

| Where                                         | What was in it                                                                                                            |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `customers.{name,email,phone}`                | the row could not be deleted at all                                                                                       |
| `events.data`                                 | **every** `customer.*` body ever written stores the whole rendered object, and nothing prunes `events`                    |
| `charges.payer_ref` / `payer_ref_masked`      | the payer's MSISDN as the rail was given it — reachable from a customer only _through_ an intent                          |
| `charges.failure_raw` / `refunds.failure_raw` | **the rail's own message, verbatim** — a mobile-money rail declining a collection names the subscriber it declined it for |
| `idempotency_keys.response_body`              | the exact JSON a `POST /v1/customers` answered, kept 24 hours to replay                                                   |

`vpay_db::customers::erase_in_tx` rewrites all five **in the transaction that
erases the customer**, because "vpay erased this payer" may not be true of one
table and false of four. `provider_requests` and `webhook_deliveries` need no
statement and that is a property of their schemas rather than an oversight:
the first stores a status code and an attempt number and no bodies (migration
`0016`), the second a `payload_sha256` and not the payload (`0022`).

**The two `failure_raw` columns were added to that list on 2026-09-11, by the
review, after they survived an erasure in a test.** They are not identifier
columns, which is why the enumeration that produced this table — an
enumeration of "the copies that survived, in full" — did not have them: they
hold `"{code}: {message}"` as MTN's `Reason` and Orange's `raw_reason`
assemble it out of a body vpay does not author, kept so an unmapped decline
survives for whoever fixes the mapping table
([failures.md](failures.md)). A rail that answers
`PAYER_NOT_FOUND: subscriber 2376… is not registered` has therefore put the
payer's number in vpay's database in a column no redaction named. The marker
replaces the whole string rather than the number inside it: a redaction that
had to recognise every spelling a rail might use fails silently on the first
one it has not seen. `failure_code` beside it survives, so _why_ the payment
failed is still answerable once the payer is gone. `refunds.reason` is left
alone — it is the **merchant's** free text about their own refund, the same
kind of thing `metadata` is.

`an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table` is the
proof, and its shape is the point: it scans **every** `text`, `varchar` and
`jsonb` column `information_schema` reports, before and after, for four
literals a fixture put there — including one the fixture writes into
`charges.failure_raw` and `refunds.failure_raw`, which is the assertion that
does not depend on anybody having thought of the column. A test that named
tables would have named the wrong ones, which is exactly what happened to the
issue.

### What the merchant is told, and what changed about it

`customer.deleted` now carries the **redacted** object: the ids, `created`,
`livemode`, the merchant's own `metadata`, `deleted: true`, and no identifier
of the payer's. The stored bodies of that customer's earlier
`customer.created` and `customer.updated` events are redacted in the same
transaction, the new one included.

**This reverses what this document said until 2026-09-10**, which was that the
body carried `name`, `email` and `phone` and that this was "the point rather
than a leak" because after a hard delete there was nothing else to read. Two
things about that argument survive and one does not. The merchant _did_
receive the payer's details, in `customer.created` and in every
`customer.updated`, over a signed body to endpoints they configured; that copy
is theirs, they are responsible for it, and vpay cannot reach it. What does
not survive is the conclusion that vpay may therefore keep its own copy for
ever in a table nothing prunes. Between the merchant's convenience in
identifying which payer was erased and the payer's erasure being real, the
erasure wins.

### The `409` that went away, and the advice that could never be followed

The old refusal said: _"This customer is referenced by a PaymentIntent or a
Checkout Session and cannot be deleted … Clear the customer's `name`, `email`
and `phone` instead."_ Clearing all three is what
`at_least_one_identifier` refuses. The advice was unfollowable, and the
paragraph this section replaces called the surviving identifiers "a trade, and
it is stated rather than implied". It was stated; it was also not a trade
anybody had chosen, and it exempted from the twelve-month retention promise
exactly the payers vpay had taken money from.

### What is logged, and by whom

|                                    | redacts                                                                | why                                                                                                                                                                              |
| ---------------------------------- | ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `vpay_db::CustomerRow`'s `Debug`   | `name`, `email`, `phone` (lengths only), `address` (a component count) | vpay's logs are not the merchant's. This struct reaches `tracing` fields, `anyhow` chains and every failing assertion.                                                           |
| `vpay_sdk::Customer`'s `Debug`     | nothing (derived)                                                      | the merchant collected this data, already holds it, and is responsible for it. Redacting it would hide their own data from them and do nothing about the copy in their database. |
| `@vaam-apps/vpay-sdk`'s `Customer` | nothing                                                                | same.                                                                                                                                                                            |

The asymmetry is deliberate and is the opposite of `CheckoutSession`'s, where
both SDKs _do_ redact: that object carries a **credential**, and printing one
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

Each pass **erases** every customer idle for more than
`CUSTOMER_IDLE_AFTER` (365 days) and not already erased — hard-deleting the
ones nothing references and anonymising the rest, exactly as
`DELETE /v1/customers/{id}` does. One `customer.deleted` per erasure, in the
same transaction.

**Until 2026-09-10 it skipped every customer a payment intent, a checkout
session or an invoice referenced**, because the `NO ACTION` foreign keys made
deleting one impossible and offering one would have minted an `evt_…` for a
deletion Postgres was about to refuse. That was a consequence of the schema
rather than a decision, and its effect was that the twelve-month promise did
not apply to any payer a merchant had ever billed or taken money from.
Migration `0041` separates the two things it conflated: nothing is detached,
and the payer is still erased.

The guard that replaced it is `anonymized_at IS NULL`. Without it an
anonymised customer stays idle for ever and the sweep offers it again on the
next pass, and the one after — an hourly `customer.deleted` about a payer
already erased, for the life of the deployment.

"Idle" is measured by `customers.last_used_at`, and **"used" means**: created,
updated, or named by a payment intent, a checkout session or an invoice. Every one of
those paths calls `vpay_db::Customers::touch_last_used`. A path that is
_missing_ does not fail — it makes a live customer look idle, and the sweep
deletes it twelve months later with nothing in any log saying anything unusual
happened. That is why the confirm path stamps as well as the create path, and
why `a_confirm_stamps_the_customers_clock_and_keeps_it_out_of_the_sweep`
exercises it through the router rather than calling the method.

The stamp is **monotonic**: `touch_last_used` filters on `last_used_at < now`,
so a vpay process whose clock is behind cannot rewind a customer's retention
clock. Two processes do not share a clock and the horizon is twelve months, so
a rewind is not a rounding error — it is the difference between surviving a
pass and not.

The erasure and its `customer.deleted` event are **one transaction**, and so
are the four redaction statements. A crash between them would erase a
merchant's customer with nobody ever told, and there is no sweep over
"customers erased without an event" and no way to build one for the branch
where the row is gone.

### Why it is its own job kind

It runs on `sweep_expired`'s schedule and its healthy answer is zero too,
which is the argument that put checkout-session expiry _inside_ that job. What
separates this one is what a failure means: `sweep_expired`'s three statements
are bounded deletes of vpay's own bookkeeping, and this one erases a
merchant's personal-data records and tells them about it. Sharing a job would
report a failing customer sweep as "the housekeeping sweep is unhealthy", with
an idempotency-key count it has nothing to do with in the same log line, and
would put a merchant-visible event behind the same lease as an internal
delete.

### `customer.deleted` is the only way a merchant learns

A hard delete is unobservable by polling: the object is gone, and a `GET`
afterwards is byte-identical to a `GET` for an id that never existed. An
anonymisation is _observable_ — the object answers `deleted: true` — but only
to a merchant who thinks to re-read it, which nobody does on a schedule for
twelve months.

The event's `data.object` is the **redacted** customer: the ids, `created`,
`livemode`, the merchant's own `metadata` and `deleted: true`. It names which
`cus_…` was erased and nothing about the payer. See "What the merchant is
told, and what changed about it" above for why that is the reverse of what
this paragraph said until 2026-09-10 and what the reversal costs.

---

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
([../api/README.md](../api/README.md) said so at the time); a vpay predating
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
[../sdks/parity.md](../sdks/parity.md), owned by the SDK maintainers.

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
[../reference/vpay-db.md](../reference/vpay-db.md) carries that argument in
full.

---

## Events (2026-09-10, [issue #66](https://github.com/vaam-apps/vpay/issues/66))

Three of this resource's four writes emit, and each writes its `events` row in
the transaction of the write it describes.

| Write                                             | Event              | Where                                                                                                                                                                                                                     |
| ------------------------------------------------- | ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /v1/customers`                              | `customer.created` | `vpay_api::v1::customers::create_with_event`                                                                                                                                                                              |
| `POST /v1/customers/{id}`, when something changes | `customer.updated` | `vpay_api::v1::customers::update_once`                                                                                                                                                                                    |
| `POST /v1/customers/{id}` with no body            | — nothing          | Stripe's no-op; nothing is written, so there is nothing to report                                                                                                                                                         |
| `DELETE /v1/customers/{id}`                       | `customer.deleted` | `vpay_api::v1::customers::delete_once` → `vpay_db::customers::erase_in_tx` (**2026-09-10**, [#96](https://github.com/vaam-apps/vpay/issues/96) item 2 — until then this route emitted **nothing** and only the sweep did) |
| The retention sweep                               | `customer.deleted` | `vpay_db::customers::erase_idle` (2026-09-06)                                                                                                                                                                             |
| A second `DELETE` on an erased customer           | — nothing          | the erasure already happened; a second event would say it happened twice                                                                                                                                                  |

`customer.created` and `customer.updated` were **not** in
`type_is_a_documented_event` until migration `0039`, and this document's
"What is not built" section said so for four days. The reason it gave was
migration `0023`'s rule — the vocabulary moves in lockstep with the code that
writes it, and both routes were single statements on the pool. Making them
transactional is what let the labels come in.

### The update takes the row's lock, and that is not defensive

`metadata` is merged key-wise (Stripe's contract), so the value written is a
function of the value stored: the handler reads, merges in Rust, and writes.
Until 2026-09-10 that read ran on the pool, and
`vpay_api::v1::customers::update`'s own doc comment said in as many words that
two concurrent updates each adding one key could lose one of them, and that it
was not closed. That was tolerable while nothing depended on the result being
definite.

`customer.updated` is exactly such a dependency. A merchant acting on an event
that described the losing merge would be acting on a state the database does
not hold — and unlike a stale read, they would have no reason to doubt it.

So the read is `SELECT … FOR UPDATE` inside the transaction that writes and
emits. A second request blocks on the lock, re-reads the **committed** merge,
and merges onto that; the two events then describe the two states in the order
they happened. `two_concurrent_metadata_merges_keep_both_keys_and_the_event_carries_the_committed_state`
drives two real requests through the shipping router, and
`a_locked_customer_read_waits_for_the_writer_and_then_sees_its_value` forces
the interleaving deterministically at the repository seam. Removing
`FOR UPDATE` was measured on 2026-09-10: both fail.

The three scalar fields were never at risk and still are not — each is written
from the request alone, and the statement assigns only the columns the request
mentioned.

### What does not emit, and why each one would be noise

- **A bodiless `POST /v1/customers/{id}`.** Stripe answers the object
  unchanged and writes nothing; so does vpay. An event about a change that did
  not happen is a webhook a merchant has to work out how to ignore.
- **`touch_last_used`, the retention stamp.** It moves `last_used_at`, which
  is on **no** wire object at all — so an event for it would carry a body
  byte-identical to the previous one, once per payment, for ever.
- **A `404`.** The transaction is abandoned before any write.

## Status

**Built and proven against a real Postgres and the shipping router, worker and
SDKs (2026-09-06, S4a; extended 2026-09-10 twice — issue #66's events, then
issues #67/#68/#96's address and erasure).**
`backends/tests/integration/tests/customers.rs` is **twenty-one** cases;
`postgres_smoke.rs` adds five at the schema; `vpay-db`'s own module adds
nine with no container plus two container-backed ones for the transaction
seam.

**The retention promise is complete as of 2026-09-10.** Before migration
`0041` a customer with payment history could not be deleted, so the payer's
`name`, `email` and `phone` survived every "deletion" — and so did every copy
in `events.data`, `charges.payer_ref` and `idempotency_keys.response_body`,
which no code named. `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`
is the evidence: it scans every `text`, `varchar` and `jsonb` column
`information_schema` reports, finds three fixture literals before the
`DELETE` and none after.

**What is not built, and is a gap rather than a decision against it:**

- **Nothing erases a payer from a merchant's own copy**, and nothing can. The
  merchant received `name`, `email`, `phone` and `address` in
  `customer.created` and in every `customer.updated`, over signed bodies to
  endpoints they configured, before the erasure. vpay redacts _its_ stored
  copy of those bodies and cannot reach theirs. A deployment answering a
  payer's erasure request has to tell the merchant separately; there is no
  `customer.redacted` fan-out and no design for one.
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
- **An erased customer is still listed by `GET /v1/customers`**, which is a
  choice rather than a gap but is worth reading as one: a merchant paging
  their customers sees `[redacted]` rows. The alternative — filtering them
  out — makes the list deny a `cus_…` that `GET /v1/customers/{id}` answers,
  and makes `has_more` describe a different set from the rows. If a merchant
  wants them out, the shape is a `deleted` filter in the same `WHERE` as
  `merchant_id`, and nobody has asked.

### Correction, 2026-09-07 (S4b)

This section listed **Invoices** as a gap until 2026-09-07: "the definition of
'used' above names two referencing objects and would name three. Invoices do
not exist". [Migration `0036`](invoices.md) is when that stopped being true.
`invoices.customer_id` is a **required** `NO ACTION` foreign key, so a customer
with any invoice cannot be row-deleted, and
`vpay_db::customers::UNREFERENCED` grew its third `NOT EXISTS` in the same
commit — with `the_sweep_guard_names_every_table_that_can_reference_a_customer`
moved from two to three, which is the assertion that made the omission
impossible to ship.

**Second correction, 2026-09-10.** That paragraph said an invoiced customer
"cannot be deleted at all", and until migration `0041` it could not: the sweep
skipped it and `DELETE` answered `409`. It is now **anonymised** instead. The
third `NOT EXISTS` is still load-bearing and what it guards is worse than
before — without it the erasure takes the hard-delete branch, the foreign key
raises `23503`, and the whole transaction rolls back, leaving the payer
un-erased with the merchant told nothing.
`an_invoiced_customer_is_anonymised_rather_than_deleted` (renamed from
`an_invoiced_customer_is_never_offered_to_the_sweep`) is the test.

Creating an invoice also **stamps** the customer's retention clock, exactly as
creating an intent or a session does, so a merchant who bills a payer monthly
never has that payer swept.

Without the third clause nothing would have broken loudly: the foreign key
would still refuse the delete, and the sweep would simply _offer_ a customer it
can never remove — minting an `evt_…` and building a `customer.deleted` object
for a payer whose record is not going anywhere, once an hour, forever.
`an_invoiced_customer_is_never_offered_to_the_sweep` is the test, and deleting
the clause is what makes it fail.
