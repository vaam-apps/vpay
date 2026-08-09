# Payment lifecycle

## Two flow shapes

The MVP rails have genuinely different payer journeys, and the core selects
between them on a **capability value** (`ProviderFlow`), never on a rail name.

| | **push** (MTN MoMo) | **redirect** (Orange Money) |
|---|---|---|
| How the payer acts | Prompt on their handset; they enter a PIN | Browser redirect to the rail's hosted page; they enter an OTP from USSD |
| Who holds the payer identifier | We do — it is an input to submit | The rail does. We may never learn it |
| Submit returns | An acknowledgement, no id | A `pay_token` and a URL to redirect to |
| Status after `confirm` | `processing` | `requires_action` |
| Can the payer act before we persist? | **Yes** | **No** |

That last row is the whole reason `docs/flows/crash-safety.md` has two sections.

## States

```mermaid
stateDiagram-v2
    direction TB
    [*] --> requires_payment_method : create
    requires_payment_method --> canceled : cancel
    requires_payment_method --> processing : confirm on a push rail
    requires_payment_method --> requires_action : confirm on a redirect rail
    requires_action --> processing : payer redirected, token durable
    requires_action --> failed : submit response lost, payer never redirected
    processing --> processing : timers only
    processing --> succeeded : rail says succeeded
    processing --> failed : rail says failed
    state "requires_payment_method + last_payment_error" as failed
    succeeded --> succeeded : refund
    succeeded --> [*]
    canceled --> [*]
    failed --> [*]
```

## What each transition means

**`confirm` always submits.** `requires_confirmation` is never emitted; it is
absent from the enum rather than present and unreachable.

**`requires_action` is redirect-only.** It carries Stripe's own
`next_action.redirect_to_url` shape, so merchants' existing redirect handling
works unchanged. Push rails never enter this state — there is nothing for a
browser to do while a payer types a PIN into their own handset.

**`processing` leaves only on a terminal answer from the rail.** Timers fire but
assert nothing. This is the crux of the design: a payment that is still pending
at minute 15 can resolve successfully at hour 30, and pretending otherwise is
how you double-charge.

**A rail-reported failure is the only thing that fails a payment.** The intent
returns to `requires_payment_method` with `last_payment_error` populated —
terminal in practice, because only one charge may ever exist per intent.

**`canceled` is reachable only from `requires_payment_method`.** Once a rail has
the request you cannot recall it.

**Refunds do not change intent status.** A refund is a separate object.

## One charge per intent, forever

```sql
CREATE UNIQUE INDEX one_charge_per_intent ON charges (payment_intent_id);
```

A plain unique index, not a partial one. Scoping it to live states leaks: the
moment a charge moves to `failed`, the predicate stops covering it and a second
charge becomes insertable — and "failed" can mean a state we reached *before*
the rail's answer was final.

**Retry means a new PaymentIntent.** This is the one place the API deviates
noticeably from Stripe's ergonomics, and it is deliberate.

## Status

Types and the flow-selection logic are implemented and tested
(`vpay-core::state`). No transition is driven by real data yet — see
[../status.md](../status.md).
