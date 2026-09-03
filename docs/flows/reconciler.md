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
callback endpoint are **not started** — see [../status.md](../status.md).

The loop's *error contract* now exists ahead of the loop:
`vpay_worker::JobError::decision(attempt)` turns any job failure into
`RetryAfter { delay, alert }`, `Terminal`, or `DeadLetter`, derived only from
the error's classification ([errors.md](errors.md)). The 24-hour `unresolved`
escalation above is `JobError::Exhausted` → `RetryAfter { delay: 1 h, alert:
true }`: exactly this document's "still polled, once an hour, and now
raising an alert" — never a silent failure, and never a dead-letter, because
the late success at hour 30 is a normal transition. Nothing calls
`decision()` yet.

**Unchanged by Step 3 (2026-09-03), deliberately — but the adapter API this
document needs now exists.** No job loop, no escalation, no callback
endpoint: nothing polls a `submitted` charge, so a confirmed push intent sits
in `processing` indefinitely and a redirect intent in `requires_action`.
What Step 3 supplied is the half a reconciler cannot be written without:

- `ProviderAdapter::query_status` is implemented on both rails and is
  `async`, and it returns a canonical `ChargeStatus::NotFound` for a
  reference the rail has no record of — never a failure. That distinction is
  the whole basis of the recovery table in
  [crash-safety.md](crash-safety.md), and it is proven on both rails by the
  conformance case `not_found_is_never_on_its_own_a_failure`.
- A charge that is pending and later settles walks correctly:
  `pending_then_successful_walks_the_scenario` drives a WireMock scenario
  through two `query_status` calls.
- `parse_callback` is implemented on both rails and returns identifiers
  only. **There is still no callback route**, so nothing compares Orange's
  `notif_token` against the stored one, and MTN's callbacks are unsigned and
  unauthenticated in any case. Until that route exists, `parse_callback`
  is exercised by tests and by nothing else.
