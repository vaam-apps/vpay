# Outbound webhooks

Stripe's scheme, copied exactly, so merchants' existing verification code works.

**Header:** `Vpay-Signature: t=1753401600,v1=<hex hmac>`
**Signed payload:** `"{timestamp}.{raw_body}"`, HMAC-SHA256, hex-encoded.
Constant-time comparison; reject a timestamp older than 5 minutes.

## Only real Stripe event types

`payment_intent.created`, `payment_intent.processing`,
`payment_intent.succeeded`, `payment_intent.payment_failed`,
`payment_intent.canceled`, `charge.refunded`, `charge.refund.updated`.

A custom type is silently dropped by any merchant using `stripe-node`'s typed
event union or an exhaustive `switch`. This is why a late success emits a plain
`payment_intent.succeeded`: an event merchants structurally tend to ignore is
the worst possible carrier for "money actually arrived".

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
`fanout_state` column would leave no way to *find* events never fanned out.
Either mistake produces a succeeded payment with no webhook.

Delivery is at-least-once; merchants must dedupe by `event.id`.

## Status

**TX 1 is real. Everything after it is not. Updated 2026-09-03 (Step 4).**

**What is built.** The worker writes the `events` row *inside* the business
transaction, exactly as the sketch above requires: `vpay_db::settlement::`
`apply_succeeded`/`apply_failed` move the charge, move the intent and insert one
event in a single transaction, with `fanout_state = 'pending'`. Two types only,
both from this document's list — `payment_intent.succeeded` and
`payment_intent.payment_failed` — and the CHECK `type_is_a_documented_event`
(migration `0018`) refuses anything else at the database. `data` is the
`payment_intent` **wire object**, rendered through the same
`vpay_api::model::PaymentIntentObject` that `GET /v1/payment_intents/{id}`
returns, so the body a merchant will eventually receive cannot disagree with the
API's own about a field. `worker_e2e.rs` asserts the row: exactly one event for
the settled intent, its type, its `fanout_state`, and the contents of its
`data`.

**What is not built — all of TX 2 and everything downstream.** No fan-out loop:
the backlog query `vpay_db::events::pending_page` exists and is tested, and **no
shipping code calls it**, so every event ever written is still `pending`. No
endpoint registry, no `webhook_deliveries` table, no
signing, no `Vpay-Signature` header, no retry schedule, no
`GET /v1/events` route. `deliver_webhook` is deliberately **absent** from
migration `0021`'s `kind_is_known` CHECK, so this build cannot enqueue a
delivery by accident and then silently never run it; Step 5 adds the kind in
its own migration. **No merchant has ever received a webhook from this code.**

The three event types this document lists that nothing writes at all —
`payment_intent.created`, `payment_intent.processing`,
`payment_intent.canceled` — plus the two refund types, are unchanged: Step 4
writes events for terminal transitions only (decision 4 of
`docs/plans/2026-09-03-step4-worker.md`).

See [../status.md](../status.md).
