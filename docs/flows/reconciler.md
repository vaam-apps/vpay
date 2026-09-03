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

**Changed 2026-09-03 (Step 4): the loop exists, and it settles payments.**
The poll ladder, the recovery table and the 24-hour escalation all run —
against a real Postgres and a real WireMock rail, in
`backends/tests/integration/tests/worker_{recovery,e2e}.rs`. What is still
unbuilt is named at the end, and the callback endpoint is still one of them.

**What is built.**

- **A durable queue.** `jobs` (migration `0021`), claimed with
  `UPDATE … WHERE id = (SELECT … FOR UPDATE SKIP LOCKED LIMIT 1)`, leased on
  `locked_by`, completed by `DELETE … WHERE id AND locked_by`. Every write
  that ends a lease is guarded on `locked_by`, so a worker whose lease was
  reaped mid-run discards its answer instead of stamping it over whoever holds
  the job now (`two_workers_claiming_together_never_take_the_same_job`).
- **The job is enqueued in the same transaction as the charge**
  (`vpay_api::v1::payment_intents`'s confirm path calls
  `vpay_db::jobs::enqueue_in_tx` inside the charge insert's transaction), so
  every kill point in [crash-safety.md](crash-safety.md) leaves work behind.
  The hourly `scan:live` job is a **backstop only** — it re-enqueues a live
  charge nothing has touched for ten minutes, and in a healthy deployment it
  finds nothing.
- **The ladder above, wired.** `vpay_worker::poll_delay(attempt)` is indexed by
  `jobs.attempts - 1` (the claim increments the counter), and a rung is one
  `UPDATE jobs SET run_at = now() + delay`.
- **The state table.** `vpay_core::settlement::settle(StatusKind, ChargeState)`
  is a `const fn`, total and wildcard-free in both dimensions; its 24-pair
  table is transcribed as a test, and a second test proves the transcription
  covers every pair exactly once. A terminal charge answers `None`: no rail
  answer moves one, in either direction.
- **The 24-hour escalation, and it does not stop the polling.**
  `RecoveryPolicy::unresolved_after` (default 24 h, measured from
  `charges.created_at`, which is written before the rail is called) moves the
  charge to `unresolved` and fails the job with `JobError::Exhausted` →
  `RetryAfter { delay: 1 h, alert: true }`: an hourly re-poll *and* an alert,
  never a dead letter
  (`a_charge_past_the_horizon_is_unresolved_polled_hourly_and_alerted_never_parked`).
  **The escalation does not depend on the rail answering.** A status query that
  fails — a rail whose endpoint is down, or misconfigured — used to keep the
  charge off the horizon entirely, riding the ladder quietly forever, because
  the horizon was only reached after a successful answer
  (`a_rail_that_never_answers_is_still_escalated_at_the_horizon`; a review
  finding, not a design property). Past the horizon every outcome short of a
  settlement leaves the charge `unresolved` — **and the worker keeps asking the
  rail, once an hour.** A terminal answer at hour 30 settles the payment
  through the ordinary path, exactly as "a late success is the normal
  transition" above requires, whether it is a success or a decline
  (`a_late_success_past_the_horizon_still_settles`,
  `a_decline_past_the_horizon_settles_an_unresolved_charge_and_clears_the_alert`
  — the second one is what pins `unresolved` inside
  `vpay_db::payment_intents::LIVE_CHARGE_STATES`, without which an escalated
  charge could never be settled at all). A **rail** failure, another
  non-terminal answer, or a `submitting` charge the recovery table wants to
  resubmit each leave the charge `unresolved` and raise the hourly alert again
  (`a_resubmit_past_the_horizon_still_escalates` — that last one also a review
  finding: the resubmit arm returned a ladder rung and never escalated). The
  re-escalation writes nothing once the charge is already there, so
  `charges.updated_at` keeps naming the last real change
  (`a_second_hourly_poll_of_an_unresolved_charge_re_alerts_without_writing_it_again`).
  A rail failure and nothing else: a poisoned job row or a Postgres error past
  the horizon keeps its own classification and is parked or retried as itself,
  because a composite re-deciding a leaf's category is precisely what ADR-0011
  forbids (`a_poisoned_job_past_the_horizon_is_parked_rather_than_rescheduled_hourly`).
- **A late success settles normally.** `succeeded` from any live state is one
  transaction: charge `succeeded` (plus `provider_txn_id`), intent `succeeded`
  with `amount_received = amount`, and one `payment_intent.succeeded` event —
  no special type, no special case.
- **Housekeeping is on a timer at last.** `sweep:expired` (hourly) deletes
  expired idempotency keys and client-assertion `jti`s and reaps expired job
  leases; both deletes used to run once at `vpay-server` boot and nowhere else.
  Lease reaping additionally runs at worker boot and every `lease / 2`, because
  the sweep is itself a row in `jobs` and a worker that died holding it would
  leave the only reaper unclaimable.

**What is not built.**

- **No callback endpoint.** `POST /provider/{code}/callback` does not exist, so
  nothing enqueues a poll from a callback and nothing compares Orange's
  `notif_token` against the stored one. `parse_callback` is implemented on both
  rails and is exercised by tests and by nothing else. The section above
  describes a design, not a route.
- **`prompt_ttl_seconds` / `prompt_expired_at` are not implemented** — the
  whole of "Timers assert nothing" above except the 24-hour rung. There is no
  `charges.prompt_expired_at` column, no config key, and no
  `payment_intent.processing` event with `expired: true`, so a merchant's
  "check your phone" UI has nothing to turn off. Deferred deliberately in Step
  4 (decision 6 of `docs/plans/2026-09-03-step4-worker.md`); it is a coherent
  unit with the fan-out in Step 5.
- **No fan-out.** The `events` rows this document's late success writes are
  inserted with `fanout_state = 'pending'` and nothing reads them —
  see [webhooks.md](webhooks.md).
- **The `contradiction` classifier is wired but its call sites are untested.**
  A rail that reports the opposite of a settled charge raises
  `error!(alert = true, …)` and changes nothing; the classifier's table is unit
  tested over the whole cartesian product, and neither call site is reached by
  any test (one is unreachable behind the terminal guard, the other needs a
  real multi-worker race). See [../status.md](../status.md).

The loop's error contract is unchanged and now has a consumer:
`vpay_worker::JobError::decision(attempt)` turns any job failure into
`RetryAfter { delay, alert }`, `Terminal` or `DeadLetter`, derived only from
the error's classification ([errors.md](errors.md)), and
`vpay_worker::run_loop` is the one place those three answers become the three
writes that end a lease.

See [../status.md](../status.md).
