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

**Not started.** See [../status.md](../status.md).
