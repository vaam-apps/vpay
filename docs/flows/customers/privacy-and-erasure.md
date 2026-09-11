# Customers — privacy, and what `DELETE` actually erases

_Split out of [docs/flows/customers.md](../customers.md) on 2026-09-11 by exp57, which broke a 888-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

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
the **payer** goes: all eleven identifier columns are written, `anonymized_at`
is stamped, and the object then renders `deleted: true`.

Eleven, and they are not written the same way, because they cannot be. The
nine **text** columns become the literal `[redacted]`. The two **coordinate**
columns become `NULL` — they are `BIGINT`, and there is no integer that is not
a possible place, so a marker value would be a coordinate, somewhere real, on
a row claiming the payer is gone. For those two the erasure is the absence,
and `anonymized_at` is what still says a payer was there. A payer's
coordinates are the most sensitive field on this object: a name is how
somebody is addressed and a point is where they sleep.

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
`anonymized_at` is set and whose eleven identifier columns are not all in
their erased state: the marker in the nine text ones, `NULL` in the two
coordinate ones.

All eleven are written, including components the payer never filled in,
because _which fields a record carried is itself information about the
person_. That argument is why the nine carry a value rather than a NULL, and
it is not lost on the two that cannot: `anonymized_at` is non-NULL on exactly
the rows the constraint applies to, so "was there a payer here?" stays
answerable without the coordinate being what answers it.

The constraint is spelled `IS NOT DISTINCT FROM '[redacted]'`, not `=`, and
that is not a stylistic choice. **A CHECK is violated only when its expression
evaluates to FALSE, and `name = '[redacted]'` over a NULL `name` is NULL,
which passes.** So the `=` spelling accepted an `anonymized_at` row with a
NULL identifier column — the first state a missed assignment produces, since
the columns an erasure most easily misses are the ones nobody filled in. The
review caught it on 2026-09-11 by holding each of the nine back as `NULL` as
well as as a value; both halves are in
`an_anonymised_customer_carries_the_marker_in_every_identifier_column`.

The two coordinate columns are held back the other way round in the same case
— as an in-range **value**, since `NULL` is their legal erased state — and
each is held back _alone_. That is only attributable because
`address_coordinates_are_both_or_neither` carries the same
`anonymized_at IS NOT NULL` disjunct the two shape CHECKs carry, which leaves
the marker CHECK the only constraint that can fire on a row claiming to be
erased. Without it Postgres would name the pair rule instead, and the marker
CHECK's coverage of the coordinate could not be tested one column at a time.

`at_least_one_identifier` was **not** relaxed for this and does not need to
be: the marker is not NULL. It now also backstops the erasure in the one
direction that matters — an erasure that NULLed all three identifiers rather
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
| `customers.address_*_microdeg`                | the payer's position — the most sensitive of the eleven, and the one no literal scan can look for                         |
| `events.data`                                 | **every** `customer.*` body ever written stores the whole rendered object, and nothing prunes `events`                    |
| `charges.payer_ref` / `payer_ref_masked`      | the payer's MSISDN as the rail was given it — reachable from a customer only _through_ an intent                          |
| `charges.failure_raw` / `refunds.failure_raw` | **the rail's own message, verbatim** — a mobile-money rail declining a collection names the subscriber it declined it for |
| `idempotency_keys.response_body`              | the exact JSON a `POST /v1/customers` answered, kept 24 hours to replay                                                   |

`vpay_db::customers::erase_in_tx` rewrites all of them **in the transaction
that erases the customer**, because "vpay erased this payer" may not be true of
one table and false of four. The stored `customer.*` bodies have the payer's
position in them too, nested inside `data.object.address`: the redaction
replaces the whole `address` key with the redacted object rather than walking
into it, so there is no path by which a nested component survives — the nested
object is never read. `provider_requests` needs no statement and that is a
property of its schema rather than an oversight: it stores a status code and
an attempt number and no bodies (migration `0016`).

**The two `failure_raw` columns were added to that list on 2026-09-11, by the
review, after they survived an erasure in a test.** They are not identifier
columns, which is why the enumeration that produced this table — an
enumeration of "the copies that survived, in full" — did not have them: they
hold `"{code}: {message}"` as MTN's `Reason` and Orange's `raw_reason`
assemble it out of a body vpay does not author, kept so an unmapped decline
survives for whoever fixes the mapping table
([failures.md](../failures.md)). A rail that answers
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
`jsonb` column `information_schema` reports, before and after, for five
literals a fixture put there — including one the fixture writes into
`charges.failure_raw` and `refunds.failure_raw`, which is the assertion that
does not depend on anybody having thought of the column. A test that named
tables would have named the wrong ones, which is exactly what happened to the
issue.

**The scan has three stated limits, and the third is the coordinate's.** It is
`public` only; it can only find a copy of a literal the fixture wrote; and it
reads `text`, `character varying` and `jsonb` — so the two `BIGINT` coordinate
columns are outside it _in principle_. Widening it to numeric columns would
not help: every integer is a possible coordinate, so a hit would mean nothing
and a miss would mean nothing. So those two columns are asserted **directly,
by name, as NULL** after the erasure, both in that case and in
`a_customer_with_payment_history_is_anonymised_rather_than_deleted`. What the
scan _does_ cover is the rendered copies: a `jsonb` column cast to `TEXT`
renders a number as its digits, so the fixture's latitude is findable in
`events.data` and `idempotency_keys.response_body` before the erasure and must
not be after — which is what fails if the event-body redaction stops reaching
inside `data.object.address`.

### A delivery already in flight, and the digest that would have parked it

`webhook_deliveries` stores no payload — a `payload_sha256` and not the bytes
(`0022`) — so it holds no copy of the payer. It still gets a statement, and
for the opposite reason to a leak.

That digest is recorded by the **first signed attempt** and compared against
every later one, so that two different bodies can never go out under one
event id. Rewriting `events.data` changes the bytes a pending delivery would
re-render. A `customer.created` mid-ladder when the erasure lands — a
merchant's receiver having an outage, which is the case the ladder exists for
— therefore failed that comparison and was **dead-lettered**, with an
operator-facing message blaming "a renderer changed under a live delivery".
The merchant never learned the payer was erased, and un-parking a dead letter
is manual ([../runbooks/webhook-delivery-failures.md](../../runbooks/webhook-delivery-failures.md)).

So the erasure clears `payload_sha256` on the deliveries that can still be
attempted, in the same transaction, and the next attempt signs and sends the
redacted body. It narrows the digest guard in exactly one place: the one
change of bytes vpay makes on purpose. `succeeded` and `exhausted` deliveries
are left alone — nothing re-renders them, and the digest of what a merchant
was actually sent is forensics.
`an_erasure_mid_ladder_redelivers_the_redacted_body_instead_of_dead_lettering`
in `tests/webhooks.rs` is the proof; removing the statement turns its fourth
step into `JobError::Poisoned`. Found by the review, 2026-09-11.

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

### Two windows the erasure does not close, stated rather than implied

**One: a response body stored a few milliseconds after the erasure.** Every
write under `/v1` carries an `Idempotency-Key`, and the response is stored in
`idempotency_keys.response_body` for 24 hours **after** the handler's
transaction commits — `PostRequest::finish`, in `vpay_api::v1::payment_intents`.
A `POST /v1/customers/{id}` that commits, then loses the race to a `DELETE`
that erases the same customer, then stores its own response, writes the
payer's identifiers back into a table the erasure has already swept. The two
transactions serialise on the customer's row lock, so the window is only the
gap between one committing and its `finish` write — but it is real, it is not
closed, and the bound on it is `sweep_expired`'s deletion of every row past
`expires_at`: **24 hours**.

It is left open rather than closed because closing it belongs in the generic
idempotency store, which knows nothing about customers, and a resource-shaped
exception there is a worse thing to own than a bounded window somebody can
read about. Found by the review, 2026-09-11; it is a maintainer's call
whether 24 hours is acceptable.

**Two: vpay cannot erase the merchant's copy, and there is deliberately no
second event type for it.** The merchant received the payer's details in
`customer.created` and in every `customer.updated`, over signed bodies to
endpoints they configured. That copy is in their database, vpay cannot reach
it, and no mechanism vpay could ship would change that.

What vpay can do is **tell them**, and it already does: `customer.deleted` is
emitted in the erasure's own transaction, on both branches, and it carries the
`cus_…` in `data.object.id` and the instant of the erasure as the event's own
`created`. That is the whole of the signal a merchant needs to find their row
and erase it — the mapping from `cus_…` to their user is theirs, and they had
to keep it to use the object at all.

The review considered adding a `customer.redacted` type saying the same thing
and **declined** (2026-09-11), for [webhooks.md](../webhooks.md)'s standing rule:
a second label for one transition is a type a Stripe-shaped handler has no
branch for, and it would read as an enforcement vpay cannot perform. What is
missing is not a mechanism; it is a **contract**. Whether a merchant is
obliged to act on `customer.deleted` — and within what window — is a data
processing agreement, not a webhook, and it is a maintainer's decision. It is
not made here.

### What is logged, and by whom

|                                    | redacts                                                                                                                           | why                                                                                                                                                                                                                               |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `vpay_db::CustomerRow`'s `Debug`   | `name`, `email`, `phone` (lengths only), `address` (a component count, the coordinate counted as one component and never printed) | vpay's logs are not the merchant's. This struct reaches `tracing` fields, `anyhow` chains and every failing assertion — and a coordinate in a `tracing` field is a payer's home in vpay's logs for the life of the log retention. |
| `vpay_sdk::Customer`'s `Debug`     | nothing (derived)                                                                                                                 | the merchant collected this data, already holds it, and is responsible for it. Redacting it would hide their own data from them and do nothing about the copy in their database.                                                  |
| `@vaam-apps/vpay-sdk`'s `Customer` | nothing                                                                                                                           | same.                                                                                                                                                                                                                             |

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
