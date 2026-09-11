# Outbound webhooks

Stripe's scheme, copied exactly, so merchants' existing verification code works.

**Header:** `Vpay-Signature: t=1753401600,v1=<hex hmac>`
**Signed payload:** `"{timestamp}.{raw_body}"`, HMAC-SHA256, hex-encoded.
Constant-time comparison; reject a timestamp older than 5 minutes.

## Only real Stripe event types

`payment_intent.created`, `payment_intent.processing`,
`payment_intent.succeeded`, `payment_intent.payment_failed`,
`payment_intent.canceled`, `charge.refunded`, `charge.refund.updated`,
`checkout.session.expired`, `customer.created`, `customer.updated`,
`customer.deleted`, `invoice.created`, `invoice.finalized`, `invoice.paid`,
`invoice.voided`.

A custom type is silently dropped by any merchant using `stripe-node`'s typed
event union or an exhaustive `switch`. This is why a late success emits a plain
`payment_intent.succeeded`: an event merchants structurally tend to ignore is
the worst possible carrier for "money actually arrived".

### Which of them is written, and by what

**Eleven of the fifteen are written, and only eleven.** Every writer below puts
its `events` row in the _same transaction_ as the transition it reports;
there is no other shape in this repository, and TX 1 below is the reason.

| Type                            | Written by                                                                                                                                                                                                                                               | Since                                                                                           |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `payment_intent.created`        | — nothing                                                                                                                                                                                                                                                | —                                                                                               |
| `payment_intent.processing`     | — nothing                                                                                                                                                                                                                                                | —                                                                                               |
| `payment_intent.succeeded`      | `vpay_db::settlement::apply_succeeded` (TX 1)                                                                                                                                                                                                            | 2026-09-03                                                                                      |
| `payment_intent.payment_failed` | `vpay_db::settlement::apply_failed` (TX 1), **and** `vpay_api::v1::payment_intents::persist_decline` for a decline at submit                                                                                                                             | 2026-09-03; the submit path **2026-09-10** ([#57](https://github.com/vaam-apps/vpay/issues/57)) |
| `payment_intent.canceled`       | `vpay_api::v1::payment_intents::cancel_with_event`                                                                                                                                                                                                       | **2026-09-10** ([#57](https://github.com/vaam-apps/vpay/issues/57))                             |
| `charge.refunded`               | — nothing                                                                                                                                                                                                                                                | —                                                                                               |
| `charge.refund.updated`         | — nothing                                                                                                                                                                                                                                                | —                                                                                               |
| `checkout.session.expired`      | `vpay_db::checkout_sessions::expire_due`, from the hourly sweep                                                                                                                                                                                          | 2026-09-04                                                                                      |
| `customer.created`              | `vpay_api::v1::customers::create_with_event`                                                                                                                                                                                                             | **2026-09-10** ([#66](https://github.com/vaam-apps/vpay/issues/66))                             |
| `customer.updated`              | `vpay_api::v1::customers::update_once`, under the row's lock                                                                                                                                                                                             | **2026-09-10** ([#66](https://github.com/vaam-apps/vpay/issues/66))                             |
| `customer.deleted`              | `vpay_db::customers::erase_idle` (the retention sweep) and `vpay_api::v1::customers::delete_once` (the route, **2026-09-10**, [#96](https://github.com/vaam-apps/vpay/issues/96) item 2 — until then `DELETE /v1/customers/{id}` emitted nothing at all) | 2026-09-06                                                                                      |
| `invoice.created`               | `vpay_api::v1::invoices::write_with_event`                                                                                                                                                                                                               | 2026-09-07                                                                                      |
| `invoice.finalized`             | `vpay_api::v1::invoices::write_with_event`                                                                                                                                                                                                               | 2026-09-07                                                                                      |
| `invoice.paid`                  | `vpay_db::settlement::apply_succeeded` (TX 1)                                                                                                                                                                                                            | 2026-09-07                                                                                      |
| `invoice.voided`                | `vpay_api::v1::invoices::write_with_event`                                                                                                                                                                                                               | 2026-09-07                                                                                      |

The four with no writer are documented shapes nothing emits — events are
written for terminal transitions only, and `created`/`processing` are
progress. The two refund types have no writer because no rail in this
repository refunds anything (`../status.md`).

**`payment_intent.payment_failed` has two writers, and that is deliberate.**
A rail can refuse a charge in two places — at the submit, before the charge
was ever polled (`persist_decline`, the `409 charge_declined` a merchant gets
synchronously), and at a later status query (`apply_failed`, the worker's poll
ladder). To a merchant they are one thing: this payment did not go through,
the intent is back at `requires_payment_method`, `last_payment_error` says
why. A second type would be a type a Stripe-shaped handler has no branch for,
which is this document's standing rule. A merchant cannot receive both for one
intent: there is one charge per intent, forever, and the submit path runs only
when the rail refused it before anything polled it.

Until 2026-09-10 only the poll path emitted, and the submit path was the one
terminal outcome no signed event reported
([#57](https://github.com/vaam-apps/vpay/issues/57)). The visible cost was in
`examples/shop`: MTN's documented test number `237600000400` is refused at
submit, so the shop's order stayed `unpaid` for ever and its README had to say
so. **One case is fail-closed rather than emitting:** if the intent moved
between the rail's refusal and the write — which `cancel`'s live-charge
`NOT EXISTS` makes unreachable while a charge is `submitting` — the
`last_payment_error` stamp matches no row, so there is no committed intent to
render and no event is written. The alternative would be a body that is either
stale or invented. It is a `WARN` naming both omissions.

**`payment_intent.canceled` was in this vocabulary for seven days short of a
week of releases with nothing writing it**, which is exactly the state
migration `0023`'s lockstep rule exists to prevent and the one case that
predates the rule. It came in with `0018`, both merchant SDKs carried the
variant, this document listed it — and `POST /v1/payment_intents/{id}/cancel`
was a single pooled statement that moved the row and told nobody. A merchant
who settles from signed events could not reach a cancelled state at all
(`examples/shop`'s order page was the visible half). Since 2026-09-10 the
cancel runs in a transaction and the event is written inside it; the pooled
`vpay_db::PaymentIntents::cancel` was **deleted** rather than left beside the
transactional one, so "cancel without an event" is no longer expressible. A
cancel the compare-and-swap refuses — a status that forbids it, or a charge
the rail may still be acting on — writes no event, which is the other half.

**The four `invoice.*` bodies carry `lines.data` EMPTY, and the `/v1` object
does not.** The event's `data` is rendered inside the transaction that wrote
the row, and reading an invoice's lines there would put a second query on a
connection holding the number sequence's row lock. A merchant who needs the
lines reads `GET /v1/invoices/{id}`, which always carries them. This is a real
difference between what a webhook says and what the API says about the same
object, and it is stated here rather than discovered
([invoices.md](invoices.md)).

**Two Stripe invoice types are deliberately absent.**
`invoice.marked_uncollectible` and `invoice.payment_failed` are not in the
list, for the reason `customer.created` was absent until 2026-09-10: nothing
writes them.
`POST /v1/invoices/{id}/mark_uncollectible` is a single statement with no
transaction to put an event in, and a failed intent leaves the invoice `open`
with the merchant already receiving `payment_intent.payment_failed`. A
merchant learns about a write-off from
`GET /v1/invoices?status=uncollectible`.

**`customer.deleted` is the one type a merchant cannot substitute polling
for.** Every other event describes a row that is still there afterwards, so a
merchant who missed one can re-read the object. This one describes an
**erasure**: a customer nothing references is removed and a
`GET /v1/customers/{id}` afterwards is byte-identical to one for an id that
never existed, and one an intent, a session or an invoice references is
anonymised in place. It is written inside the same transaction as the write it
describes, so a crash cannot leave a customer erased with nobody told; see
[customers.md](customers.md).

**Its `data.object` carries no identifier of the payer's**, and that reverses
what this paragraph said until 2026-09-10 ([issue
#68](https://github.com/vaam-apps/vpay/issues/68)). It said `name`, `email`
and `phone` were included "because after the delete there is nothing else to
read". They are `[redacted]` now, and so are the stored bodies of that
customer's earlier `customer.created` and `customer.updated` events, rewritten
in the same transaction. The merchant did receive those identifiers when the
events were delivered and holds their own copy — that is theirs. What changed
is the conclusion that vpay may therefore keep its own copy for ever in a
table nothing prunes. `events` is never pruned, which made it the largest
surviving copy of a payer vpay had been asked to forget.

**`customer.created` and `customer.updated` joined the list on 2026-09-10**
([issue #66](https://github.com/vaam-apps/vpay/issues/66)), in migration
`0039`, in the same change that wrote them — which is migration `0023`'s
lockstep rule working as intended rather than an exception to it. This
paragraph used to say the opposite and to explain why: `POST /v1/customers`
and `POST /v1/customers/{id}` were single statements on the pool, emitting an
event meant putting the write and the event in one transaction, and adding the
labels ahead of a writer would have put two values in a closed vocabulary that
no code could produce. The transaction is what changed; the labels followed
it, not the other way round. **An external mirror of a merchant's customers no
longer has to poll.**

`customer.updated`'s transaction opens with `SELECT … FOR UPDATE` on the row,
and that is load-bearing rather than cautious. `metadata` is merged key-wise,
so the written value is a function of the stored one, and a pooled read left a
window in which two concurrent updates each adding one key lost one of them. A
merchant acting on an event describing the losing merge would be acting on a
state the database does not hold — see [customers.md](customers.md). A
**bodiless** update emits nothing: nothing was written.

**`charge.refunded` and `charge.refund.updated` carry a `refund`**, which
since 2026-09-05 ([issue #46](https://github.com/vaam-apps/vpay/issues/46)) is
ten keys rather than nine: the tenth is `fee`, what the rail charged to move
the money back. `data.object` is the wire object — the same
`vpay_api::model::RefundObject` that `GET /v1/refunds/{id}` renders (issue
#45) — so a webhook body and an API response cannot disagree about it, and
`the_api_response_and_an_events_payload_for_one_refund_are_byte_identical`
drives both refund event types against a real route to prove it. The key is always present
and, on every refund this deployment could produce, always `null`; **`null` is
not `0`** and a receiver must not treat it as one
(`a_refund_delivered_as_either_refund_event_carries_fee_present_and_null`;
[merchant-auth.md](merchant-auth.md) has the table). Neither type has ever been
emitted, as the paragraph above says.

**`checkout.session.expired` is the only one whose `data.object` is not a
`payment_intent` or a `refund`.** It carries a `checkout.session`: the thirteen
keys `docs/flows/hosted-checkout.md` documents, with `status` already
`expired`, `payment_status` whatever the money did, and `url` **always
`null`** — a hosted session's `url` carries its `client_secret` in the
fragment (D6), and a webhook body is stored, signed, delivered at-least-once
and replayed on every rung of the ladder. So `url: null` in an event does
**not** mean the session was embedded; read `ui_mode`. `client_secret` is
absent entirely, as is the `return_token`, which is a column and on no wire
object at all. Both merchant SDKs carry the type in their vocabulary
(`vpay_sdk::KnownEventType::CheckoutSessionExpired`, `@vaam-apps/vpay-sdk`'s
`KnownEventType`) with a narrowing accessor for the payload
(`Event::checkout_session`, `isCheckoutSessionEvent`), and both keep working
unchanged for a type they do not know: `type` is a `string` in both, not the
union.

The `object_id` on such a row is the `cs_…`, which made it the fourth prefix
that polymorphic column carries.

## Two-step outbox

```
TX 1 (the business transaction)
  UPDATE payment_intent SET status='succeeded'
  INSERT event (fanout_state='pending')

TX 2 (fan-out)
  scan events WHERE fanout_state='pending'
  INSERT webhook_delivery per matching endpoint
  UPDATE event SET fanout_state='done'

delivery, with retries: 10s → 30s → 2m → 10m → 1h → 6h → 24h
```

Both steps matter. Fan-out inline with the state change would make the business
transaction depend on reading the endpoint table. Fan-out without a
`fanout_state` column would leave no way to _find_ events never fanned out.
Either mistake produces a succeeded payment with no webhook.

That ladder is **8 POSTs over about 31 hours** — the first attempt plus seven
retries, 112,360 seconds of waiting in total — and then the delivery is
`exhausted`. Every non-2xx walks the whole of it, `4xx` included: a receiver
answering `410 Gone` is retried for 31 hours exactly as a `500` is. That is
Stripe's behaviour too, and it is deliberate — a `404` from a receiver that is
mid-deploy is indistinguishable from one that means "stop", and stopping early
on the wrong one loses the event.

**Delivery is at-least-once, and its order is not guaranteed.** Merchants must
dedupe by `event.id`, and must **not** assume that two events for one merchant
arrive in the order they happened. The fan-out preserves `seq` order when it
_creates_ the jobs, and nothing preserves it afterwards: N claim tasks take N
different jobs concurrently (`FOR UPDATE SKIP LOCKED`), and one delivery that
fails drops to the next rung of the ladder while later ones go out immediately.
A receiver that decides state from arrival order will settle a payment from a
stale event. `event.created` and the object's own `status` are what to reason
from.

## Status

**Eleven of the fifteen event types have a writer** (the table above says which,
and since when), the two-step outbox is real, and a signed delivery has been
read back out of a receiver's own journal. **Every receiver in this
repository's history is a WireMock host on a compose network; no merchant
endpoint outside this repository has ever been POSTed to.** The four pages
below are where that is measured rather than asserted, and the last of them is
what is _not_ built.

The Status section was 589 lines of dated measurement until 2026-09-11. It is
four pages now, in the order it was written, moved verbatim — every "Updated
2026-09-04", every caveat, and every "what is not built" paragraph:

- [webhooks/status-writers.md](webhooks/status-writers.md) — which transition
  emits which event, which of them have reached a receiver, and the three
  writers
- [webhooks/outbox-transactions.md](webhooks/outbox-transactions.md) — TX 1 and
  its twin, TX 2, the abandonment rule, delivery, and signing proven against
  the SDKs a merchant installs
- [webhooks/endpoints-and-egress.md](webhooks/endpoints-and-egress.md) —
  endpoints as configuration, what boot-time validation does **not** check, and
  the runtime egress guard
- [webhooks/events-api-and-recovery.md](webhooks/events-api-and-recovery.md) —
  `GET /v1/events`, what recovery does and does not reach, the two hand-written
  statements a replay costs, and **"What is not built"**

**If you read one, read the last.** Its closing "What is not built" paragraph
is the boundary every claim on the other three sits inside.
