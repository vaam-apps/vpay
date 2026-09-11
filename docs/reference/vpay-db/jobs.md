# `vpay-db` — `jobs`: the lease, the dead letter, and `pull_forward_in_tx`

_Moved out of [docs/reference/vpay-db.md](../vpay-db.md) on 2026-09-11 by exp57, which split a 3 330-line reference into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## `jobs`

### The lease is the whole design

A job is _claimed_ by an `UPDATE` that stamps `locked_at`/`locked_by` on exactly
one runnable row, and it is only ever finished or rescheduled by a statement
that also names the same `locked_by`. That guard is not decoration: without it, a
worker whose lease was reaped mid-run (it hung, the reaper freed the row,
another worker picked it up) would `DELETE` a job the second worker is in the
middle of executing, or reschedule it out from under them. This is ABA, and
`idempotency::claim` closes the same hole the same way with its `claim_id`.

### `enqueue_in_tx` exists only in the transactional form

The queue's one hard requirement is that the job and the write that creates the
work commit together. `confirm` opens its charge row before calling the rail
([crash-safety.md](../../flows/crash-safety.md)); enqueueing the poll in that same
transaction is what makes _all three_ of that document's kill points leave a job
behind. A pooled `enqueue(pool, …)` would let a caller write the job on a second
connection that commits independently, which reintroduces both halves of the
failure it exists to prevent — a job for a charge that rolled back, and a
committed charge with nothing to drive it. So there is no such function.

It is deliberately not an upsert either. `DO UPDATE SET run_at = …` would let a
backstop scan drag a job already scheduled for an hour's time back to now, which
is how a poll ladder silently becomes a hot loop. `Ok(false)` — the `dedupe_key`
was already queued — is the normal answer for the backstop scan and for a
re-enqueue after a crash, not an error.

### `pull_forward_in_tx` is the exception, and it has to be asked for

Step 8 lane C added one write that _does_ move a scheduled job back to now:
`UPDATE jobs SET run_at = now() WHERE dedupe_key = $1 AND locked_at IS NULL
AND run_at > now() + $2 AND run_at < 'infinity'`. Its only caller is
`vpay_api::provider_callback` — a rail said something happened, and the point
of a callback is to ask the rail _now_ instead of at the ladder's next rung,
which is ten seconds away at best and fifteen minutes away after half an hour.

It is a separate method rather than the `DO UPDATE` the section above rules
out, and that is the whole distinction: an upserting `enqueue_in_tx` would
apply the pull-forward to every caller, including the backstop scan that
re-enqueues every live charge's key every ten minutes. One caller asking for
it is a callback; every caller getting it is a hot loop.

The three guards are each refusing a different thing:

- `locked_at IS NULL` — a leased job is being polled right now, and that poll
  will see the rail's answer. It is also the only way this write stays out of
  the `locked_by` discipline the lease section describes: it never touches a
  row someone holds.
- `run_at > now() + $2` — a job whose time has come needs nothing, and
  skipping the write is what makes a burst of duplicate callbacks (which both
  rails send) free rather than a queue of writers contending for one row lock.
  `$2` is the **floor**, added by Step 8's review: a job due within it is
  about to run, so moving it buys the rail nothing and costs an
  unauthenticated caller one rail request. The value is the poll ladder's
  fastest rung, and it is a _parameter_ because the ladder is
  `vpay_worker::poll_delay` — a policy about how often a rail is asked
  anything, which this crate must not hold (ADR-0002). The caller passes
  `vpay_api::provider_callback::PULL_FORWARD_FLOOR`, and
  [vpay-api.md](../vpay-api/provider-callback.md#what-an-anonymous-caller-can-and-cannot-get-out-of-it)
  states what it does and does not bound.
- `run_at < 'infinity'` — a dead letter stays parked. The section below states
  that the occupied `dedupe_key` is what keeps a scan _or a callback_ from
  re-creating work a human has to look at first; this is the clause that makes
  the "or a callback" half true.

`Ok(false)` is therefore "nothing to do" in all of these cases and never a
failure, which matters because the caller always calls `enqueue_in_tx` first:
a job it just inserted at `now()` is the ordinary `false`.

### Why claiming does not consider lease expiry

`claim`'s predicate is `locked_at IS NULL`, full stop, so it matches
`jobs_claimable_idx` exactly. "Unlocked _or_ the lease has expired" depends on
`now()` and cannot be an index predicate, so it would turn every claim into a
scan over every leased row. Expiry is therefore a separate, periodic pass —
`reap_expired_leases` — which frees a stale lease _once_ and lets the ordinary
claim path pick the row up on its next turn. Its callers are described in
[vpay-worker.md](../vpay-worker.md#two-lease-reapers-on-purpose).

### Why a dead letter is parked and not deleted

A job that is done is deleted (`finish`); a job that is not done is rescheduled
with its error recorded (`reschedule`). A job that _cannot_ be done —
`JobError::Poisoned`, or anything else `Classify::retry` answers `Retry::Never`
for — is neither, and `dead_letter` is the third write.

It exists because deleting one is not safe for a _payment_ queue. `poll_charge`
is the only thing driving a live charge to a terminal state; delete its row and
the charge is unattended, with nothing in the database saying why. The backstop
scan would then re-enqueue the same `dedupe_key` at its next pass and the same
failure would repeat every ten minutes, forever, with a fresh `attempts = 1`
each time — a hot loop that reads as a flapping rail rather than as a
permanently broken row.

Parking is `run_at = 'infinity'` (a real `timestamptz` value, not a sentinel
year) with the lease cleared. That single write is all four properties at once:
`claim`'s `run_at <= now()` can never match it, `reap_expired_leases`'
`locked_at` predicate can never resurrect it, the `dedupe_key` stays occupied so
no scan or callback re-creates the work, and `last_error` keeps the reason where
the operator handling the page is already looking. A `dead_lettered_at` column
would carry no fact these do not, and every reader of the table would have to
learn to exclude it.

The cost, stated plainly: a parked job is invisible to `oldest_runnable_run_at`
and to every other `run_at`-ordered query, so the _only_ way an operator learns
one exists is the alert the loop raises when it parks it, and
`SELECT * FROM jobs WHERE run_at = 'infinity'`. Requeuing one is an
`UPDATE jobs SET run_at = now()` by hand, which is deliberate: it should follow
a human deciding the underlying data is fixed.

`last_error` carries `vpay_core::error::source_chain` and not `Display` alone
(ADR-0011's amendment) — `ProviderError::Transport` keeps the `reqwest` error as
a `#[source]`, so the column would otherwise say "the request to the rail
failed" and never "operation timed out".
