# Runbook: the worker's job queue

Everything the worker does is a row in `jobs` (migration `0021`). This runbook
covers the four things that go wrong with it: a **dead-lettered** job, a
**stranded lease**, a **charge escalated to `unresolved`**, and a **rail
contradicting a settled charge**. It also says how to read the gauge line the
loop emits every 60 s.

Every SQL statement below was run against a database with all 21 migrations
applied. Nothing here is a `just` recipe, because there is none for the queue
— the repository ships no operator CLI, and inventing one in a runbook would
be worse than a `psql` prompt. In the local stack that prompt is:

```bash
docker compose exec postgres psql -U vpay -d vpay
```

## Reading the gauge line

Each worker logs one line a minute, at `INFO`, with the message `job loop
gauge` (`vpay_worker::run_loop::gauge_loop`):

```
worker_id=vpay-worker-7f4c/1/9e21ab7c claimed=412 finished=380 rescheduled=30
dead_lettered=1 lost=0 queue_behind_seconds=4  "job loop gauge"
```

- `worker_id` is `hostname/pid/random`. Every field except the queue age is
  **this process's** cumulative tally, so two replicas emit two lines a minute
  and summing them without the id describes neither.
- `queue_behind_seconds` is `now() - min(run_at)` over the **runnable,
  unleased** rows — how far behind the queue is, not how many rows it holds. It
  is `null` when nothing is runnable (an empty queue, which is a different fact
  from "zero seconds behind"), and parked rows (`run_at = 'infinity'`) are
  excluded so one dead letter cannot peg it at infinity. A value drifting
  steadily upwards is the backlog signal, and the knob for it is
  `--worker-concurrency` or another replica.
- `lost` counts jobs whose lease was reaped **while this worker was running
  them**, so its answer was discarded. Any non-zero value means a handler
  outran the lease (five minutes by default); it is a real defect, not noise.

The same number, read from the database:

```sql
SELECT now() - min(run_at) AS queue_behind
FROM jobs
WHERE locked_at IS NULL AND run_at <= now() AND run_at < 'infinity';
```

## Dead-lettered jobs

### Alert

`disposition=DeadLettered` on a `job failed:` line, always with `alert = true`.

### What it means

The loop parked the job at `run_at = 'infinity'` with its lease cleared and the
reason in `last_error` (`vpay_db::jobs::dead_letter`). A park means
`JobError::decision` answered `DeadLetter`, i.e. the error's classification is
`Retry::Never`: a poisoned row (a payload that does not deserialise, a
`charge_id` naming no charge, a `charges.state` outside the enum), an
unimplemented adapter, a broken migration. **Re-running it unchanged produces
the same failure**, which is why it is parked rather than retried.

A parked `poll_charge` job means **nothing is driving that charge any more.**
If the charge is still live, that is the thing to fix, not the job row.

### Find them

```sql
SELECT id, kind, dedupe_key, payload, attempts, last_error, created_at
FROM jobs
WHERE run_at = 'infinity'
ORDER BY created_at;
```

`dedupe_key` names the work: `poll:<charge_id>`, `resubmit:<charge_id>`,
`sweep:expired`, `scan:live`. For a charge job, read the charge next:

```sql
SELECT id, state, provider_code, provider_reference_id, amount, currency_code,
       failure_code, failure_raw, created_at, updated_at
FROM charges WHERE id = 'ch_…';
```

### Re-run one, after fixing the cause

Fix what `last_error` names first. Then:

```sql
UPDATE jobs
SET run_at = now(), attempts = 0, last_error = NULL
WHERE id = '…' AND run_at = 'infinity';
```

`attempts = 0` puts the poll ladder back on its first rung (10 s) — which is
what you want for a charge that has been sitting unattended, and is also what
makes a job that is still broken fail fast rather than an hour from now. The
`run_at = 'infinity'` predicate is deliberate: it refuses to touch a row a
worker has since picked up.

### Do not

- Do not re-run a parked job without changing anything. It will park again,
  and the second `last_error` overwrites the first.
- Do not `DELETE` a parked `poll_charge` row to "clean up". The row is the only
  record that a live charge stopped being driven; deleting it makes the charge
  invisible until the hourly backstop scan (`scan:live`) notices it, and only
  if the charge is more than ten minutes stale.

## Stranded leases

### Alert

`freed job leases whose worker never came back` (a `WARN` from the reaper,
logged only when it freed something), or a job that is claimed and not moving.

### What it means

A worker that was `SIGKILL`ed — or that outlived its grace period without
handing its leases back — leaves rows with `locked_at`/`locked_by` set.
`vpay_db::jobs::claim` matches only `locked_at IS NULL`, so those rows are
unclaimable until a reaper frees them.

**In a healthy deployment you do not have to do anything.** A running worker
reaps leases older than `RecoveryPolicy::lease` (5 minutes) at boot, again
every `lease / 2` on its own timer, and once more inside the hourly
`sweep:expired` job. The boot reap is what covers the case where the dead
worker was holding `sweep:expired` itself.

### Look

```sql
SELECT id, kind, dedupe_key, locked_by, locked_at, now() - locked_at AS held_for, attempts
FROM jobs
WHERE locked_at IS NOT NULL
ORDER BY locked_at;
```

`locked_by` is `hostname/pid/random` — the same value the gauge line carries,
so it tells you which pod to look at. A `held_for` under a few minutes on a
worker that is still logging is a job in flight, not a strand.

### Free them by hand — only when no worker is running

```sql
UPDATE jobs
SET locked_at = NULL, locked_by = NULL, last_error = 'lease expired'
WHERE locked_at < now() - interval '5 minutes';
```

This is exactly `reap_expired_leases`. Keep the interval at or above the
deployment's lease: freeing a lease that is merely *slow* hands the same job to
a second worker while the first is still running it. That is survivable — every
handler is a compare-and-swap and the loser's write matches no row — but it
shows up as `lost` on the gauge and it wastes a rail call.

## Charges escalated to `unresolved`

The alert, what it means and the reconciliation are in
[unresolved-charges.md](unresolved-charges.md). What is new is that the
escalation now actually happens, and that there is a job row behind every one:

```sql
SELECT c.id, c.payment_intent_id, c.provider_code, c.provider_reference_id,
       c.amount, c.currency_code, c.created_at,
       j.run_at, j.attempts, j.last_error
FROM charges c
LEFT JOIN jobs j ON j.dedupe_key = 'poll:' || c.id
WHERE c.state = 'unresolved'
ORDER BY c.created_at;
```

The job is **rescheduled hourly, never parked** — `JobError::Exhausted` is
`RetryAfter { delay: 1 h, alert: true }`, because a late success at hour 30 is
a normal transition and a dead letter would stop the polling that catches it.
`j.last_error` on such a row reads `job … exhausted the poll ladder …`; that is
the escalation, not a failure to act on.

A `j.run_at` of `infinity` on an `unresolved` charge is the bad case: the
charge is live and nothing is polling it. Treat it as a dead-lettered job
(above).

## A rail that contradicts a settled charge

### Alert

```
alert=true job_id=… charge_id=… charge_state=succeeded rail_answer=failed
"the rail reports the opposite of this charge's settled state; vpay has not
changed the charge — reconcile it against the rail's settlement statement"
```

### What it means

A poll landed on a charge that had already settled, and the rail's answer went
the *other* way: `failed` against a `succeeded` charge, or `succeeded` against
a `failed` one. Only those two pairs raise this
(`vpay_core::settlement::contradiction`); a `pending` or `not_found` answer
against a settled charge is a rail that has not caught up with itself and is
ignored on purpose.

**vpay does not act on it.** A charge settles once; a poll that could flip
`failed` to `succeeded` would make the settlement compare-and-swap decorative.
The log line is the entire response, and it names both states so the
reconciliation can start from it alone.

### Steps

1. Read the charge, including the rail's own identifier:

   ```sql
   SELECT id, payment_intent_id, state, provider_txn_id, provider_reference_id,
          failure_code, failure_raw, amount, currency_code, created_at, updated_at
   FROM charges WHERE id = 'ch_…';
   ```

   `provider_txn_id` (migration `0021`) is written only by
   `vpay_db::settlement::apply_succeeded`, so a `succeeded` charge that has one
   is the identifier to quote to the rail.
2. Read the `provider_requests` timeline for the charge — steps 2 and 3 of
   [unresolved-charges.md](unresolved-charges.md) apply unchanged.
3. Reconcile against the rail's settlement statement. If the statement agrees
   with the rail's *later* answer and not with vpay's charge, the money moved
   differently from what the merchant was told: escalate, and do not resolve it
   from the API alone.

### Do not

- Do not `UPDATE charges SET state = …` to make the row agree with the rail.
  The intent, the `events` row and (from Step 5) the merchant's webhook have
  already been written from the settled state; changing the charge alone leaves
  four records disagreeing instead of two.

## Status

**Written against code that exists, and exercised only in part.** The queries
here were run against a database with all 21 migrations applied; the states
they look for are produced by the worker's own integration suite
(`backends/tests/integration/tests/worker_recovery.rs`) — dead-lettered
poisoned jobs, leases stranded by a crash and reaped at boot and on the timer,
and the `unresolved` escalation. **No part of this runbook has been followed
against a running deployment**, the contradiction alert's call sites are not
exercised by any test (see `docs/status.md`), and there is no dashboard view
for any of it: `psql` is the whole toolkit today.
