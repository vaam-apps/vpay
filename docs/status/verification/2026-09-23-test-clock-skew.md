# 2026-09-23 — Test fixtures that stamped a job with this host's clock and claimed it on the database's

**No capability moved, no production file changed, and no assertion was
weakened.** Test fixtures and test assertions only, in
`backends/crates/vpay-db/tests/repositories.rs` and five files under
`backends/tests/integration/tests/`. Nothing is `#[ignore]`d.

Branch `test/fanout-helper-db-clock`, based on `origin/master` at `b747e5d`.

## What prompted it

On 2026-09-23 two full workspace runs, straight after a Docker Desktop VM
restart, failed first **1** and then **5** `webhooks.rs` cases, each with the
panic `the fan-out job is claimable`. The binary passed alone, and later full
runs passed on `master` and on the branch.

`claim_fanout_job` enqueued the fan-out singleton at
`OffsetDateTime::now_utc()` — **this process's** clock — and then called
`vpay_db::Jobs::claim`, whose SQL is `WHERE run_at <= now()` — **the Postgres
container's** clock (`backends/crates/vpay-db/src/jobs.rs:437`). When the
container's clock reads further behind the host's than the few milliseconds
between those two calls, the row is not yet due and the claim is `None`.

This is the same asymmetry
[2026-09-18-macos-loopback-and-two-load-flakes.md](2026-09-18-macos-loopback-and-two-load-flakes.md)
§ 2 found behind a `checkout_sessions.rs` flake, fixed there for that one
helper and named for the rest. This page is the rest.

## What was demonstrated, and what was not

A scratch test (deleted before commit) against a real `postgres:16-alpine`
container on this machine:

| Step                                                                                          | Result                                              |
| --------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| `claim_fanout_job`'s exact shape, with `run_at` = host now **+ 200 ms**                       | `Jobs::claim` → **`None`**                          |
| the same, with `run_at` read from `SELECT now()` on the pool                                  | `Some("fanout:events")`                             |
| 300 × the helper's shape unmodified (host now, no offset), then `claim`                       | 0 misses                                            |
| 300 × the fixed shape (`SELECT now()`), then `claim`                                          | 0 misses                                            |
| lower bound on the container being **behind** this host (`host_before − db_now`), 300 samples | min −11.8 ms, median **+0.24 ms**, max **+1.02 ms** |

What that establishes:

- **The failure mode is real and deterministic.** A container clock behind
  this host's by more than the enqueue-to-claim gap turns the claim into
  `None` and the helper into exactly the panic seen. The +200 ms stands in for
  that skew, because a container's clock cannot be set from inside it.
- **This container was behind this host at the time of measurement.** In more
  than half the samples `SELECT now()` returned an instant _earlier_ than a
  host reading taken before the query was sent — impossible if the two clocks
  agreed — by up to about a millisecond.
- **The flake itself was not reproduced.** At about a millisecond of skew the
  enqueue-to-claim gap (a committed transaction, then a second round trip)
  was wide enough in all 300 tries. That the skew was larger just after the
  Docker Desktop restart is still a **hypothesis**, not a measurement. What
  this change removes is this _cause_; it does not claim the flake is gone.
- **The fix does not depend on the size of the skew.** Every "now" the fixed
  fixtures write, and every "now" the fixed assertions compare against, is
  read off the database's clock, so there is no second clock left in them for
  a skew to live between.

## The sites, and what changed at each

`support::db_now(pool)` (integration) and a private `db_now(pool)` (vpay-db)
are `SELECT now()`. Offsets from them (`- 60s`, `+ 1h`) mean what they meant.

### Written with the host clock, then claimed immediately — could fail

| File                                        | Site                                                                                                                                                                                                                                                                        | Change                                                                                                                                                                                                                                                                                                                                                                                |
| ------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `integration/tests/webhooks.rs`             | `claim_fanout_job` (14 callers)                                                                                                                                                                                                                                             | `run_at` from `support::db_now`; takes `&PgPool`. Also now **fails** if `claim` returns any row but the fan-out singleton, rather than handing the wrong row to `handle_fan_out`                                                                                                                                                                                                      |
| `integration/tests/support/mod.rs`          | `crashed_charge` (19 callers in `worker_recovery.rs`, 1 in `worker_e2e.rs`)                                                                                                                                                                                                 | poll job `run_at` is the charge's own `created_at`, which Postgres' `now()` wrote in the same transaction                                                                                                                                                                                                                                                                             |
| `integration/tests/worker_recovery.rs`      | `a_poisoned_job_is_parked_…`, `two_workers_claiming_together_…`                                                                                                                                                                                                             | `run_at` from `support::db_now`                                                                                                                                                                                                                                                                                                                                                       |
| `integration/tests/worker_recovery.rs`      | `the_housekeeping_jobs_are_seeded_once_…` — seeded by **shipping** `seed_singletons`, then stepped five times with no retry                                                                                                                                                 | `make_every_job_runnable` once, after seeding and before any step. It moves only rows still in the future, so it is a no-op when the clocks agree                                                                                                                                                                                                                                     |
| `vpay-db/tests/repositories.rs`             | `eight_concurrent_claims_over_one_job_…`, `…_over_eight_jobs_…` (job 0), `a_leased_job_is_invisible_to_claim`, `finish_with_the_wrong_worker_id_…`, `reschedule_clears_the_lease_…`, `reap_expired_leases_…` (`poll:fresh`), `set_payload_writes_only_for_the_lease_holder` | `run_at` from `db_now`                                                                                                                                                                                                                                                                                                                                                                |
| `integration/tests/worker_claim_latency.rs` | `enqueue_round`                                                                                                                                                                                                                                                             | `run_at` from `support::db_now`. Here the host clock did not fail the case, it **falsified the measurement**: `claim_curve` counts a row that is not yet due as already claimed, so a container behind this host recorded every probe as claimed at 0 ms. The file's header claimed the measurement was off any clock comparison; a dated note there now says since when that is true |

### A database-written instant compared against the host clock — could fail

| File                                     | Site                                                                                                  | Tolerance before                   | Change            |
| ---------------------------------------- | ----------------------------------------------------------------------------------------------------- | ---------------------------------- | ----------------- |
| `integration/tests/provider_callback.rs` | `pulled_to <= now_utc()` (`run_at` set by `pull_forward`'s `now()`)                                   | **none** if the container is ahead | `support::db_now` |
| `integration/tests/provider_callback.rs` | `first_rung_at - now >= 8s`, `parked_at - now >= 25s`, `… > now_utc()`                                | 2 s, 5 s, 10 s                     | `support::db_now` |
| `vpay-db/tests/repositories.rs`          | `pull_forward_…`: `moved <= now_utc()`; fixtures at `now`, `+5s`, `+30s` against `now() + floor`      | none / ~5 s                        | `db_now`          |
| `vpay-db/tests/repositories.rs`          | `reschedule_clears_the_lease_…`: `run_at > now_utc()`                                                 | 60 s                               | `db_now`          |
| `vpay-db/tests/repositories.rs`          | `ensure_active_signing_key_refuses_to_reactivate_…`: `retired_at <= now_utc()` (`updated_at = now()`) | **none** if the container is ahead | `db_now`          |

### Converted for consistency, not because they could fail

`claim_takes_the_earliest_runnable_job_…` (runnable rows 10 s and 60 s in the
past), `oldest_runnable_run_at_…` (hours), `enqueue_in_tx_dedupes_on_dedupe_key`
(an hour). All in the vpay-db jobs section, so the section has one rule.

### Looked at and left alone

- `webhooks.rs` `claim_scan_deliveries_job` and `claim_delivery_job` claim **by
  dedupe key** with no `run_at` predicate; no clock is compared.
- `checkout_sessions.rs` `run_until` already retries on a five-second deadline
  (2026-09-18 § 2); `drain_worker` calls `make_every_job_runnable` first.
- `customers.rs` `run_the_sweep` seeds with the host clock but pulls the one
  job it needs to `now()` on the database's clock before claiming.
- `browser_checkout.rs`, `worker_e2e.rs:892`, `provider_callback.rs`'s steps
  after a confirm: the confirm's `persist_submitted` has already pulled the
  poll job to the database's `now()`.
- `vpay-db` `pending_due_returns_the_deliveries_nothing_is_driving` (± 1 h),
  the JTI and signing-key-publication cases (60 s, 30 min, or `now()`-relative
  SQL), `confirm_rails.rs:2191` (a ±5 s window over shipping code's own
  host-clock `run_at`).
- Every case that runs the shipping `run_loop` (`worker_e2e.rs`,
  `worker_kill9.rs`, `webhooks.rs`'s real-loop cases): the loop idles and
  claims again, so skew there is a delay under a much larger timeout, not a
  failure.
- The metrics-counter cases in `vpay-db` and the migration-0022/0023
  vocabulary cases enqueue at the host's clock and never claim; neither does
  `vpay-db/src/repository.rs`'s `closure_shape` module, which is compiled
  and never run.

## Production code with the same two clocks — found, **not** changed

Each writes `run_at` (or `next_attempt_at`) from the process's
`OffsetDateTime::now_utc()` and is later compared with Postgres' `now()`. In
production the consequence is **a delay equal to the skew**, never a failure:
the loop's `Ok(None)` idles `IDLE_SLEEP` and claims again. Whether to derive
these instants server-side is a design question for shipping persistence, and
this change did not open it.

| Site                                                      | What is stamped                               | Compared at                                                                            |
| --------------------------------------------------------- | --------------------------------------------- | -------------------------------------------------------------------------------------- |
| `backends/crates/vpay-worker/src/run_loop.rs:484`         | the five singletons at boot                   | `vpay-db/src/jobs.rs:437` (`claim`)                                                    |
| `backends/crates/vpay-worker/src/handlers.rs:894`         | the poll job `commit_resubmission` writes     | `jobs.rs:437`                                                                          |
| `backends/crates/vpay-worker/src/handlers.rs:1269`        | the backstop scan's re-enqueued poll jobs     | `jobs.rs:437`                                                                          |
| `backends/crates/vpay-worker/src/handlers.rs:1712`        | a `resubmit_charge` job                       | `jobs.rs:437`                                                                          |
| `backends/crates/vpay-worker/src/webhooks.rs:522`         | every `deliver_webhook` job a fan-out creates | `jobs.rs:437`                                                                          |
| `backends/crates/vpay-worker/src/webhooks.rs:619`         | the delivery backstop's re-enqueued jobs      | `jobs.rs:437`                                                                          |
| `backends/crates/vpay-worker/src/webhooks.rs:1110`        | a delivery's `next_attempt_at`                | `vpay-db/src/webhook_deliveries.rs:533` (the backstop scan)                            |
| `backends/crates/vpay-api/src/provider_callback.rs:295`   | a callback's poll job, if none exists         | `jobs.rs:194` (`pull_forward`'s floor) and `:437`                                      |
| `backends/crates/vpay-api/src/v1/payment_intents.rs:1963` | the confirm's poll job, `now + grace`         | `jobs.rs:437` — pulled to `now()` by `persist_submitted` on every path that reaches it |
| `backends/crates/vpay-worker/src/run_loop.rs:968`         | the queue-age gauge's "now"                   | `oldest_runnable_run_at` — the gauge reads off by the skew; observability only         |

The worker's charge-age decisions already read `db_now` off the same
statement as the row (`vpay-db/src/charges.rs:604`); that is the precedent a
decision here would follow.

_(Added the same day, by the change stacked on this one: the decision was
taken — [ADR-0026](../../adr/0026-the-database-clock-schedules-jobs.md) — and
all ten sites above now schedule on the database's clock, plus an eleventh
this table did not list, `scan_live_charges`' staleness cutoff
(`handlers.rs`, compared with the Postgres-written `charges.updated_at`).
[2026-09-23-single-clock-scheduling.md](2026-09-23-single-clock-scheduling.md)
is that change's evidence. The table above is left as it was measured.)_

## Evidence

macOS 26.5.1, `rustc 1.98.0`, `cargo-nextest 0.9.146`, Docker 29.1.3. Every
cargo invocation ran with `CARGO_INCREMENTAL=0
CARGO_PROFILE_DEV_DEBUG=line-tables-only
CARGO_PROFILE_TEST_DEBUG=line-tables-only`.

### The runs

`cargo nextest` reports ignored tests as skipped; every run below skipped
none, so every count is also "0 ignored".

| Command                                                                                                             | Runs | Each run                                     |
| ------------------------------------------------------------------------------------------------------------------- | ---- | -------------------------------------------- |
| `cargo nextest run -p vpay-tests-integration --test webhooks`                                                       | 3    | 21 run: 21 passed, 0 skipped                 |
| `… --test worker_recovery --test provider_callback --test worker_claim_latency --test worker_e2e` (23 + 11 + 2 + 3) | 3    | 39 run: 39 passed, 0 skipped                 |
| `cargo nextest run -p vpay-db` (`repositories.rs`, `postgres.rs` and the unit tests)                                | 3    | 246 run: 246 passed, 0 skipped               |
| `cargo nextest run -p vpay-tests-integration` (the whole crate)                                                     | 1    | 345 run: 341 passed, **4 failed**, 0 skipped |

The four failures are the four `staff_sign_in.rs` per-source-address cases,
every one with `support::client_bound_to`'s own message —
`127.0.0.20 is not assigned to a local interface … just loopback-aliases`.
This machine's `lo0` carried only `127.0.0.1` and the aliases need `sudo`,
which this work did not run. They are
[2026-09-18](2026-09-18-macos-loopback-and-two-load-flakes.md) § 1, unrelated
to this change, and CI (Linux) does not see them.

The first of the three `worker_recovery` runs was built before
`the_housekeeping_jobs_are_seeded_once_…` gained its `make_every_job_runnable`
line; runs two and three, and the whole-crate run, include it.

### The gates

`cargo clippy -p vpay-db -p vpay-tests-integration --all-targets -- -D
warnings`: clean. `cargo fmt --all`: applied. `just verify`: `verify: ok — the fifteen gates above passed`, after this page's own index entry moved `docs/status/README.md`'s marked file count from 64 to 65.

Doctests were not run: no doc comment example changed, and the doc comments
added here are on test-only items.

## What this does not do

- It does not prove the 2026-09-23 flake is fixed. It removes the one cause
  that was identified and demonstrates that cause deterministically; the skew
  after a Docker Desktop restart was never measured.
- It changes no production code, including the ten sites above.
- It adds no regression test that fails if a fixture goes back to the host
  clock: nothing can make a testcontainer's clock run behind this host's on
  demand, and a test that simulated it would only be testing its own offset.
  The rule lives in the two `db_now` doc comments.
- No `docs/flows/` page, route, gate or `NotImplemented` token moved, so no
  flow **Status** section or `vpay-skills` change is owed.
