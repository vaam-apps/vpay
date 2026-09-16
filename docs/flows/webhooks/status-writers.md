# Outbound webhooks — Status: which transition writes which event

_Split out of [docs/flows/webhooks.md](../webhooks.md) on 2026-09-11 by exp57, which broke a 811-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

**Updated 2026-09-10: three transitions that emitted nothing now emit, and the
vocabulary is fifteen types of which eleven have a writer** (issues
[#57](https://github.com/vaam-apps/vpay/issues/57) and
[#66](https://github.com/vaam-apps/vpay/issues/66)). Nothing about the
two-step outbox changed: each new event is one more row in `events`, written
in the transaction of the transition it describes, and TX 2 fans it out with
no branch on `type`. What changed is _which_ transitions have a writer — see
the table under "Only real Stripe event types" above, which is new and is the
thing to read rather than counting by hand. The four pooled statements these
replaced (`PaymentIntents::cancel`, `Customers::create`, `Customers::update`)
were **deleted** rather than kept beside their transactional twins, because no
gate in this repository objects to a `pub` method nobody calls.

**Two of the three have been driven to a receiver, and the `customer.*` pair
has not.** `a_cancel_emits_one_payment_intent_canceled_and_it_reaches_the_receiver`
and `a_submit_decline_emits_one_payment_failed_and_it_reaches_the_receiver`
(both in `backends/tests/integration/tests/webhooks.rs`) take their transition
through the shipping route, the shipping fan-out and the shipping delivery
handler, and read the bytes back out of the WireMock receiver's own journal —
the second one against a real MTN stub answering `400 PAYER_NOT_FOUND`, which
is the only way to reach `persist_decline` from the API without a test seam.
The second was added by the sabotage review of 2026-09-10; until then this
paragraph said "only one of the three", and the delivery of the submit-time
`payment_intent.payment_failed` was an argument. It was worth measuring
rather than arguing because that body is the only
`payment_intent.payment_failed` in the system rendered by `vpay-api` instead
of by `vpay_db::settlement`.

`customer.created` **has** now been driven to the receiver, and
`customer.updated` has not. That happened sideways, on 2026-09-11: the
sabotage review of the customer erasure needed a delivery in flight when a
payer was erased, so
`an_erasure_mid_ladder_redelivers_the_redacted_body_instead_of_dead_lettering`
takes a real `customer.created` through the shipping fan-out and the shipping
delivery handler, reads the bytes out of the WireMock receiver's journal
twice, and asserts what changed between them. It is recorded here rather than
left as a side effect, because "which types have been observed on a wire" is
the number this section exists to keep honest — it is five now, not four.

`customer.updated` is still asserted at the `events` row and no further. The
fan-out is type-agnostic — it reads by `seq` and branches on nothing — so
"it would deliver too" remains an argument for that one, and it is written
here in those words.

**Updated 2026-09-07: CrateStack 0.11.1 → 0.12.0 changed nothing here.** The
`events.data` blocker above is `Value::from_plain_json`'s `f64` demotion, and
`cratestack-core`'s `src/` is byte-identical between the two releases
(`value.rs:95-106`), so the hand-written `insert_in_tx` stays for the reason
it already had. `webhook_deliveries`' `upsert(..).do_nothing()` and the
`fanout_state` compare-and-swap both still run through CrateStack, unchanged,
and the whole suite is green at 0.12.0 (`../status.md` § CrateStack 0.11.1 →
0.12.0).

**Updated 2026-09-04: the housekeeping sweep is a third writer.** A checkout
session passing its 24-hour horizon with nothing driving it now produces one
`checkout.session.expired` — in the same transaction as the status flip — and
that event goes through the identical fan-out, signing, delivery client,
egress guard and retry ladder every other event does, because it is one more
row in the same table. Migration `0029` is what let the database accept the
type; nothing else in this document changed. `docs/flows/hosted-checkout.md`'s
"An expired session notifies nobody" is retired by it.

**Both transactions are real, and a signed webhook has been delivered to a
WireMock receiver and verified with both shipping SDKs — and, since Step 5b,
with the official `stripe` package. No merchant endpoint has ever been POSTed
to.** Updated 2026-09-03 (Step 5, then Step 5b). The receiver is a host
in configuration, reached over HTTP exactly as a merchant's endpoint would be
(ADR-0006) — which is the same limit the rails carry, and the reason
[../status.md](../../status.md)'s Webhooks row is 🟡.

**Updated 2026-09-04 (Step 8): since this step every delivery goes through the
egress guard first** (`vpay_worker::ssrf`), including the ones in the compose
stack — the sandbox profile _permits_ its private receiver explicitly rather
than the guard being absent. "No merchant endpoint has ever been POSTed to"
stands, and so does its corollary: **no deployment has ever refused one
either.**

**Updated 2026-09-04 (Step 9): the receiver in the demo stack is now a real
merchant handler, and the sentence above is narrowed rather than retired.**
`examples/shop` exposes `POST /api/vpay/webhook`, which verifies the
`Vpay-Signature` header with `@vaam-apps/vpay-sdk`, dedupes by event id, marks an order
`paid` on `payment_intent.succeeded` and `failed` on
`payment_intent.payment_failed`, and answers `2xx` only after the write. It is
the first thing in this repository's history to _act_ on a delivery rather than
record it in a journal, and lane 6's Cypress specs assert an order reaching
`paid` **only** through it — the payer's return page reads the shop's database
and takes no decision from the return trip. What has still never happened is a
POST to a merchant endpoint **outside this repository**: `vpay-shop` is a
container on the same compose network, permitted by the sandbox profile's
`webhooks.allow_private_targets`, and it is code this repo wrote and tests.

**Updated 2026-09-06: two of the outbox's three writes now run through
CrateStack, inside the same transactions this document already describes.**
Nothing about the two-step shape changed — TX 1 still commits the settlement
and the `events` row together, TX 2 still creates every delivery and flips
`fanout_state` together, and `TxOutcome::Abandon` is still what a lost race
returns. What changed is which layer issues two of the statements:

| Write                               | Layer                                | Why                                                                           |
| ----------------------------------- | ------------------------------------ | ----------------------------------------------------------------------------- |
| the `events` row in TX 1            | raw `sqlx`                           | `events.data` is `JSONB NOT NULL`; see below                                  |
| `webhook_deliveries` insert in TX 2 | CrateStack `upsert(..).do_nothing()` | the `ON CONFLICT (event_id, endpoint_id) DO NOTHING` this document depends on |
| the `fanout_state` flip in TX 2     | CrateStack `update_many(..)`         | the compare-and-swap, guard included                                          |

`events.data` did **not** move, and the reason is worth stating in this
document rather than only in the reference: it is the exact object that is
signed and delivered, and CrateStack 0.12.0's `Json` scalar round-trips
through `cratestack::Value`, whose number decoding falls back to `f64` for
anything that is not an `i64`. Merchant-authored `metadata` travels inside
`data`. The insert therefore stays one hand-written statement in the same
transaction — ugly and deliberate — and two tests pin the blocker: they go red
the day somebody declares the column, which is the mistake worth catching
before a lossy number conversion reaches a signed payload. (They do _not_
notice an upstream fix on their own; the drift report's unmappable-column
count is what does. Corrected 2026-09-06.) See
[../reference/vpay-db.md](../../reference/vpay-db.md) § CrateStack.

The duplicate-delivery guard this document rests on is unchanged and is still
proved the same way. `an_abandoned_fan_out_leaves_no_delivery_and_the_event_still_pending`
is the new case that makes the transaction seam itself assertable: both
CrateStack writes happen, the transaction is abandoned, and neither survives.
Running either write on its own connection instead makes it red in about a
second.

**One caveat this change adds, and it is a real one:** `events.type` is still
closed by a hand-named database CHECK that `schemas/vpay.cstack` cannot
express, so the drift report would report a _lower_ number if the constraint
were dropped. `an_undocumented_event_type_is_refused_by_the_database` is now
the only thing that would notice. It did not exist before 2026-09-06, and
this document's "only real Stripe event types" rule had been resting on it.
