# ADR-0026: The database's clock schedules jobs

- **Status:** Accepted in part.
  - **The rule** — "the database's clock must be the one clock for anything
    the database compares" — is the maintainer's decision of 2026-09-23,
    taken on the evidence of
    [#254](https://github.com/vaam-apps/vpay/pull/254)'s verification page
    ([2026-09-23-test-clock-skew.md](../status/verification/2026-09-23-test-clock-skew.md)),
    which found ten production sites that stamp a scheduling instant with the
    application host's clock for Postgres to judge with its own.
  - **D1–D8 below** are how the implementing agent applied that rule. The
    maintainer directed the work and named the likely shape ("repository
    methods take a relative delay, and SQL computes `now() + $delay`"); the
    boundaries — which instants are scheduling and which are facts (D6), and
    what was left out (D8) — were drawn by the agent and **need the
    maintainer's confirmation in review**. Their reasons are recorded here so
    that confirmation has something to read.
- **Implementation:** the pull request that carries this ADR,
  `fix/single-clock-scheduling`, stacked on #254. Its verification page,
  [2026-09-23-single-clock-scheduling.md](../status/verification/2026-09-23-single-clock-scheduling.md),
  is the statement of what was run; this ADR records decisions.
- **Date:** 2026-09-23
- **Number checked at branch time, not assumed:** `ls docs/adr` on this
  branch returns `0001`…`0024`. `0025` is reserved for #253 (open), and
  `0027` may be taken by an erasure change in flight; `0026` was reserved for
  this change.

## Context

A job is claimed by `vpay_db::Jobs::claim`, whose predicate is
`run_at <= now()` — **Postgres'** clock. Until this change, most of the
writers of `run_at` computed it in Rust as `OffsetDateTime::now_utc()` plus a
delay — **the application host's** clock. The same shape held for a webhook
delivery's `next_attempt_at` (judged by the delivery backstop's
`next_attempt_at <= now()`), for the live-charge backstop's staleness cutoff
(compared with `charges.updated_at`, which Postgres' `now()` writes), and for
the queue-age gauge (Postgres-written `run_at` subtracted from the host clock).

With skew between an application host and the database host, every one of
those decisions moves by the skew. In production that is a delay or an
advance, never a failure — an early job is simply not claimed until the
database catches up, and the loop idles and claims again — but it is a
correctness property that depends on an operational one nothing checks
(NTP on every host), and it is not visible anywhere. #254 found the test
fixtures failing on exactly this after a Docker Desktop VM restart, fixed the
fixtures, and named the production sites without changing them.

The repository already had the right shape in places. `Jobs::reschedule` is
`run_at = now() + $delay`; `pull_forward_in_tx` is `run_at = now()` guarded by
`run_at > now() + $floor`; `reap_expired_leases` and
`WebhookDeliveries::pending_due` take their windows as durations; and the
worker's charge-age decisions read `now()` off the same `SELECT` as the row
(`vpay_db::Charges::get_by_id_as_of`, Step 8 lane H). This ADR makes that the
rule instead of the majority.

## Decisions

### D1 — A scheduling instant is computed by Postgres, from a duration

A repository method that writes an instant which Postgres will later compare
with its own `now()` takes a `std::time::Duration` relative to that `now()`,
and the statement computes `now() + $delay`. It never takes an
`OffsetDateTime`. Applied:

| Method                              | Was                                       | Is                                                                         |
| ----------------------------------- | ----------------------------------------- | -------------------------------------------------------------------------- |
| `TxRepositories::enqueue_in_tx`     | `run_at: OffsetDateTime`                  | `delay: Duration` → `now() + delay`                                        |
| `WebhookDeliveries::record_attempt` | `next_attempt_at: Option<OffsetDateTime>` | `retry_after: Option<Duration>` → `now() + retry_after`, `NULL` for `None` |

Already of this shape and unchanged: `Jobs::reschedule`,
`TxRepositories::pull_forward_in_tx`, `Jobs::reap_expired_leases`,
`Jobs::dead_letter` (`'infinity'`).

**Why a duration and not "read `SELECT now()` first and pass that".** The
second keeps an instant parameter, so the next caller can pass the wrong
clock again and the compiler cannot say so; it also costs a round trip. A
signature with no instant in it is the only version of the rule a reviewer
does not have to re-check.

### D2 — A window compared with a database-written column is a duration too

`Settlement::live_charges_stale_since(cutoff: OffsetDateTime, …)` became
`live_charges_stale_since(stale_after: Duration, …)`, and the predicate is
`updated_at < now() - $stale_after`. `charges.updated_at` is written by
`now()` on every transition, so the cutoff has to be on that clock.
`pending_due`'s `lease` and `refunds::cancel_in_tx`'s in-flight window were
already written this way.

### D3 — Unsigned: nothing in production schedules into the past

`std::time::Duration` cannot be negative, so "due now" is `Duration::ZERO` and
nothing can enqueue a job already overdue. No production caller wanted one.
Tests that need rows at chosen points around the database's `now()` — two
runnable rows in a known order, an hour-old backlog — insert through the
shipping enqueue at `ZERO` and then write `run_at` directly from
`SELECT now()` (`vpay-db/tests/repositories.rs`, `enqueue`). That is a
fixture's privilege, not an API.

### D4 — `now()`, not `clock_timestamp()`

`now()` is the transaction's start. Every existing statement in `jobs` uses
it, and a job enqueued inside a long transaction (a fan-out over many
endpoints) is therefore due from that transaction's start — which is only
ever **earlier** than the commit that makes the row visible, so it cannot
delay work. It also makes `run_at` equal to the row's own `created_at`
(`DEFAULT now()`) plus the delay **exactly**, which is what the new test
checks: a mutation to `clock_timestamp()` fails it by 0.6 ms (measured, see
the verification page).

### D5 — The queue-age gauge is subtracted in SQL

`Jobs::oldest_runnable_run_at() -> Option<OffsetDateTime>` became
`Jobs::oldest_runnable_age() -> Option<time::Duration>`:
`(EXTRACT(EPOCH FROM (now() - min(run_at))) * 1000000)::BIGINT` over the same
unleased, unparked rows. `EXTRACT` returns `numeric` since Postgres 14, so the
scaling is exact and no float crosses the wire (ADR-0007). The worker's
`queue_age` lost its `now` parameter. The sign convention is unchanged: the
gauge reads negative when the next job is in the future
([vpay-core.md](../reference/vpay-core.md) § "The queue gauge goes negative").

### D6 — Facts stay as the application observed them

Not every timestamp is a schedule. A **fact** is an instant recorded as the
application observed it and handed to a merchant, a payer or a rail as part
of a contract: `checkout_sessions.created_at` / `expires_at` (returned in the
create response), `customers.last_used_at` / `anonymized_at`, the invoice
lifecycle stamps and `due_date`, a refund's `updated_at` as the route renders
it, the `t=` in a webhook signature (judged by the _receiver's_ clock, a
third clock the database has no better claim to than the sending host), and
every `events` body.

These keep the application's clock, for two reasons:

1. **The database is not a party to the comparisons made against them.**
   `CheckoutSessions::due_for_expiry(now, …)` compares an app-written
   `expires_at` with an app-read `now`; `vpay_api::browser` gates a payer on
   `now_utc() >= session.expires_at`; `Customers::idle_since(horizon, …)`
   compares an app-written `last_used_at` with an app-computed horizon. Each
   comparison has one clock _class_ on both sides.
2. **Moving one side alone would create the defect this ADR removes.** A
   sweep that read `SELECT now()` would judge `expires_at` on the database's
   clock while the browser gate judged it on the API host's, and the two
   would disagree about the same session by the skew.

**The residual, stated plainly:** these comparisons can still span two
_application_ hosts — the replica that created a session and the worker that
sweeps it — whose clocks may differ. Moving a fact fully onto the database's
clock means stamping it in SQL and reading `now()` beside the row wherever it
is judged (the `get_by_id_as_of` shape), which changes merchant-visible
values and several call sites outside scheduling. That is a separate
decision, not taken here.

### D7 — The `now` parameters on sweeps over facts stay

`due_for_expiry(now, …)`, `expire_due(now, …)` and `idle_since(horizon, …)`
keep their instant parameter, for D6's reason, and it is what lets a test
sweep "a future instant" without waiting. They are the one remaining place a
repository method takes an instant to compare, and each compares it with a
column Rust wrote.

### D8 — Found, of the same class, and deliberately left out

Two more columns are written from the application's clock and compared with
Postgres' `now()`. Neither is job scheduling, both sit behind authentication
decisions this change was not briefed on, and each is named here so the next
change finds it:

- **`oauth_signing_keys.expires_at`** — written as the API host's
  `now_utc() + ROTATION_OVERLAP` (24 h) by
  `vpay_api::op::keys::ensure_active_in_database`, and compared
  `expires_at > now()` by `SigningKeys::publishable_signing_keys`. Skew
  lengthens or shortens a 24-hour publication overlap by the skew. D1's shape
  would fix it (`retire_previous_after: Duration`).
- **`oauth_client_assertion_jtis.expires_at`** — the client assertion's own
  `exp`, a fact from the client's clock, deleted by `expires_at < now()`.
  Whether a database clock running ahead of the API host can drop a spent
  `jti` while the API still accepts its assertion depends on the validator's
  leeway, which this change did not analyse. Named, not concluded.

## Consequences

- **No wire, schema or migration change.** Three trait methods changed
  signature (`enqueue_in_tx`, `record_attempt`, `live_charges_stale_since`)
  and one was renamed with a new return type (`oldest_runnable_age`); every
  implementation stays `pub(crate)` (ADR-0016 standard 5). No statement
  gained an `AssertSqlSafe`: `sql_audit`'s `EXPECTED_ASSERT_SITES` stays 71.
- **What the tests prove, and what they cannot.** The repository test
  `a_job_is_due_at_the_databases_now_plus_its_delay_and_a_zero_delay_claims_at_once`
  shows `run_at - created_at` equal to the delay to the microsecond and a
  zero-delay job claimed by the next statement; `record_attempt_…` shows
  `next_attempt_at - sent_at` equal to the rung exactly; the housekeeping
  case in `worker_recovery.rs` shows every shipped seed's `run_at` equal to its
  `created_at` and claims them with #254's workaround removed. **None of them
  runs a skewed host** — nothing here can set a testcontainer's clock apart
  from this machine's. That half rests on the signatures: no scheduling
  method accepts an instant, so there is no parameter through which the
  application's clock can reach `run_at` or `next_attempt_at`.
- **What this does not change.** Facts (D6), the sweeps over them (D7), and
  the two authentication columns (D8). An operator running NTP is still wise;
  scheduling no longer depends on it.
