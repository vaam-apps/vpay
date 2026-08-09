# Reconciler

Payer prompts expire. Callbacks may never arrive. So a worker drives every
payment to a terminal state — or to a human.

## Poll ladder

Implemented in `vpay_worker::poll_delay`, tested for monotonicity.

| Attempt | Delay |
|---|---|
| 0–5 | 10s, 20s, 30s, 45s, 60s, 90s |
| 6–19 | 120s |
| 20+ | 15 min, out to 24 hours |

## Timers assert nothing

**At `prompt_ttl_seconds` (default 900) still pending.** The *prompt* expired;
the *transaction* did not. Set `prompt_expired_at`, clear the intent's
`next_action`, emit `payment_intent.processing` with `expired: true` so the
merchant's UI can stop saying "check your phone". **The intent stays
`processing`. The charge stays live and stays polled.**

The TTL is config data, so a stub profile drives this path in seconds instead of
a quarter of an hour — same code, no test hook.

**At 24 hours still pending.** The charge moves to `unresolved`: still polled,
once an hour, and now raising an alert for a human to reconcile against the
rail's settlement statement. The intent remains `processing`. **The payment is
not lost, it is escalated.**

**A late success** — minute 40, or hour 30 from `unresolved` — is the normal
transition. The intent was `processing` throughout, so it moves to `succeeded`
and emits a plain `payment_intent.succeeded`. No special event type, no special
case. This is the payoff for not lying about finality at minute 15.

## Callbacks are hints

A callback never changes state. It only enqueues a status query.

```
POST /provider/{code}/callback
  → adapter.parse_callback() extracts identifiers ONLY
  → validate the reference exists
  → INSERT job (poll_charge, dedupe_key='poll:<ref>') ON CONFLICT DO NOTHING
  → 200 OK
```

Mobile-money callbacks are typically unauthenticated and unsigned, unreliable,
and sometimes duplicated. The authenticated status query is the only thing that
moves money in the ledger. `parse_callback` returns identifiers only, so the
port makes it *impossible* for an adapter to hand the core a status it read off
an unauthenticated request.

The `dedupe_key` is what stops duplicate callbacks becoming a job storm.

## Status

`poll_delay` is implemented and tested. The job loop, the escalation and the
callback endpoint are **not started** — see [../STATUS.md](../STATUS.md).
