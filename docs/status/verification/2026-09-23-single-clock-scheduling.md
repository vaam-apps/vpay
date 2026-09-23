# 2026-09-23 — Every job scheduled on the database's clock (ADR-0026)

Branch `fix/single-clock-scheduling`, stacked on
[#254](https://github.com/vaam-apps/vpay/pull/254)'s
`test/fanout-helper-db-clock` at `0cc3142`. Production code, tests and docs.
**No migration, no wire type, no route, no gate, no `NotImplemented` token
moved.**

## What prompted it

[2026-09-23-test-clock-skew.md](2026-09-23-test-clock-skew.md) fixed the test
fixtures that stamped a job with this host's clock and claimed it on
Postgres' `now()`, and listed ten production sites with the same two clocks,
unchanged. The maintainer decided the same day that the database's clock is
the one clock for anything the database compares.
[ADR-0026](../../adr/0026-the-database-clock-schedules-jobs.md) records how
that was applied and what was deliberately left out.

## Every production site, and what it does now

| Site (before this change)                                   | Was                                            | Now                                                                                   |
| ----------------------------------------------------------- | ---------------------------------------------- | ------------------------------------------------------------------------------------- |
| `vpay-worker/src/run_loop.rs` `seed_singletons`             | five seeds at host `now_utc()`                 | `Duration::ZERO` → `now()`                                                            |
| `vpay-worker/src/handlers.rs` `commit_resubmission`         | poll job at host `now_utc()`                   | `Duration::ZERO`                                                                      |
| `vpay-worker/src/handlers.rs` `scan_live_charges` (enqueue) | re-enqueued polls at host `now`                | `Duration::ZERO`                                                                      |
| `vpay-worker/src/handlers.rs` `scan_live_charges` (cutoff)  | `now_utc() - 10 min` vs `charges.updated_at`   | `live_charges_stale_since(SCAN_INTERVAL, …)` → `updated_at < now() - 10 min`          |
| `vpay-worker/src/handlers.rs` `schedule_resubmit`           | `resubmit_charge` at host `now_utc()`          | `Duration::ZERO`                                                                      |
| `vpay-worker/src/webhooks.rs` `handle_fan_out`              | each `deliver_webhook` at host `now_utc()`     | `Duration::ZERO`                                                                      |
| `vpay-worker/src/webhooks.rs` `scan_deliveries_pass`        | re-enqueued deliveries at host `now`           | `Duration::ZERO`                                                                      |
| `vpay-worker/src/webhooks.rs` `record_failure`              | `next_attempt_at = now_utc() + rung`           | `record_attempt(…, Some(rung), …)` → `now() + rung`, same `now()` as `sent_at`        |
| `vpay-worker/src/run_loop.rs` `gauge_loop` / `queue_age`    | host `now_utc() - min(run_at)`                 | `Jobs::oldest_runnable_age()`: `now() - min(run_at)` in SQL; `queue_age` has no `now` |
| `vpay-api/src/provider_callback.rs` `callback`              | enqueue at host `now_utc()`, then pull forward | `Duration::ZERO`, then pull forward (unchanged, already `now()`)                      |
| `vpay-api/src/v1/payment_intents.rs` `insert_charge`        | `now_utc() + POLL_AFTER_CONFIRM_GRACE`         | `POLL_AFTER_CONFIRM_GRACE` → `now() + grace`                                          |

The cutoff row is the one #254's table did not list: `scan_live_charges`
computed `now_utc() - stale_after()` and compared it with
`charges.updated_at`, which every transition writes with `now()`. It was
found by grepping `backends/crates` and `backends/apps` for `now_utc()` and
reading each hit against the column it ends up compared with.

The repository signatures that carry it: `TxRepositories::enqueue_in_tx`
(`run_at: OffsetDateTime` → `delay: Duration`),
`WebhookDeliveries::record_attempt` (`next_attempt_at: Option<OffsetDateTime>`
→ `retry_after: Option<Duration>`), `Settlement::live_charges_stale_since`
(`cutoff` → `stale_after: Duration`), and `Jobs::oldest_runnable_run_at` →
`Jobs::oldest_runnable_age() -> Option<time::Duration>`. Every implementation
is still `pub(crate)`. No `AssertSqlSafe` was added or removed:
`sql_audit::EXPECTED_ASSERT_SITES` stays **71**.

The module header of `vpay-api/src/provider_callback.rs` said the route could
only "bring an already-queued `poll_charge` job forward to now". It also
enqueues one when none is queued, and has since Step 8; corrected with a dated
note.

## Left on the application's clock, on purpose

ADR-0026 D6–D8 has the reasoning; the list:

- **Facts handed to a merchant, payer or rail**: `checkout_sessions.created_at`
  / `expires_at`, `customers.last_used_at` / `anonymized_at`, invoice stamps,
  refund `updated_at` as a route renders it, the webhook signature's `t=`,
  `events` bodies. Every comparison against them has the application's clock
  on both sides (`due_for_expiry(now)`, the browser gate, `idle_since`), so
  the database's clock is not a party to them; moving only the sweep would
  create a two-clock comparison. Their `now` parameters stay, which is also
  what lets a test sweep a future instant.
- **`oauth_signing_keys.expires_at`** (API host's `now_utc() + 24 h`, judged
  `expires_at > now()`) and **`oauth_client_assertion_jtis.expires_at`** (the
  assertion's own `exp`, deleted by `expires_at < now()`). Same defect class,
  not job scheduling, behind authentication decisions this change was not
  briefed on. Named, not changed.

## The tests, and what they prove

| Test                                                                                                                | What it asserts                                                                                                                                                                                                                           |
| ------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `vpay-db` `a_job_is_due_at_the_databases_now_plus_its_delay_and_a_zero_delay_claims_at_once` (**new**)              | `run_at - created_at` is exactly `0` and exactly `10 s` for the two delays; the zero-delay job is claimed by the next statement, the other is not                                                                                         |
| `vpay-db` `record_attempt_bounds_the_excerpt_moves_the_ladder_and_then_exhausts` (tightened)                        | `next_attempt_at - sent_at` is exactly the 10 s rung (was: within 1 s of a host-clock instant)                                                                                                                                            |
| `vpay-db` `oldest_runnable_age_ignores_leased_and_parked_jobs_and_is_the_databases_subtraction` (renamed, extended) | the age lies between two `SELECT now()` readings minus the row's `run_at`; a queue whose only row is an hour out reads about −1 h — a case no test covered, and which its doc had wrongly said was excluded (corrected with a dated note) |
| `vpay-db` `live_charges_stale_since_honours_the_cutoff_and_the_live_state_set` (adapted)                            | the same selection with a 10-minute window measured from `now()`                                                                                                                                                                          |
| `worker_recovery` `the_housekeeping_jobs_are_seeded_once_and_reschedule_themselves` (tightened)                     | every shipped seed has `run_at = created_at`; #254's `make_every_job_runnable` workaround is **removed** and the five steps still claim                                                                                                   |
| `webhooks` `the_ladder_walks_delivery_delay_and_then_succeeds` (adapted)                                            | `next_attempt_at - before` with `before` read off the database's clock                                                                                                                                                                    |
| `vpay-worker` `an_empty_queue_logs_nothing_and_publishes_zero_while_a_failed_read_publishes_neither`                | unchanged cases, now fed an age rather than an instant and a `now`                                                                                                                                                                        |

**Two mutations, both measured, both red:**

| Mutation                                                                         | Result                                                                                                |
| -------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `enqueue_in_tx`'s insert computes `clock_timestamp() + delay` instead of `now()` | the new test fails: `left: Duration { seconds: 0, nanoseconds: 602000 }`, `right: … 0` — 0.6 ms apart |
| `record_attempt` writes `clock_timestamp() + rung`                               | `record_attempt_…` fails: `left: … seconds: 10, nanoseconds: 149000`, `right: … 10, 0`                |

Both are a _second clock on the same database_, less than a millisecond from
the first. An instant read on this host would have to agree with Postgres'
transaction start to the microsecond to pass, which is why the exact
equality is the assertion and not a tolerance.

**What is not proven, plainly.** No test runs a host whose clock is skewed
from the database's: nothing here can set a testcontainer's clock apart from
this machine's, and a test that simulated it would only test its own offset
(#254's page says the same). The claim that a skewed host no longer moves a
schedule rests on two things together: the signatures — no scheduling method
of `Jobs`, `TxRepositories` or `WebhookDeliveries` accepts an instant, so
there is no parameter through which this host's clock reaches `run_at` or
`next_attempt_at` — and the tests above, which show the value that does
reach them is the database's own `now()`.

`worker_claim_latency` still measures from the enqueue's **commit**, and its
rows are due at the transaction's own `now()`, which precedes that commit, so
no row is ever "not yet due" when it becomes visible: the measurement is off
any comparison between the two clocks, as its header says.

## Evidence

macOS 26.5.1, `rustc 1.98.0`, `cargo-nextest 0.9.146`, Docker 29.1.3. Every
cargo invocation ran with `CARGO_INCREMENTAL=0
CARGO_PROFILE_DEV_DEBUG=line-tables-only
CARGO_PROFILE_TEST_DEBUG=line-tables-only`. `cargo nextest` reports ignored
tests as skipped.

No test under `backends/` carries `#[ignore]` (`git grep`), so every
"skipped" below is a test filtered out on the command line, and none is
ignored.

| Command                                                                                                                                                     | Result                                                                                                                      |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `cargo nextest run -p vpay-tests-integration -E 'not (…the four macOS loopback-alias staff_sign_in cases…)'` (whole crate)                                  | **341 run: 341 passed, 4 skipped** (the four filtered), 929.8 s                                                             |
| — of which `worker_e2e` / `worker_kill9` / `worker_recovery` / `webhooks` / `provider_callback` / `worker_claim_latency`                                    | 3 / 6 / 23 / 21 / 11 / 2 passed                                                                                             |
| the same six binaries again, after the last source edits (a local rename in `seed_singletons` and doc comments in `run_loop.rs` and `provider_callback.rs`) | **66 run: 66 passed, 0 skipped**                                                                                            |
| `cargo nextest run -p vpay-db`                                                                                                                              | **247 run: 247 passed, 0 skipped** (246 before this change's new test)                                                      |
| `cargo nextest run -p vpay-worker -p vpay-api -p vpay-db`, after the last source edits                                                                      | **725 run: 725 passed, 0 skipped**                                                                                          |
| `just test-doc`                                                                                                                                             | **124 passed, 0 failed, 1 ignored** — the ignored one is `sdks/rust/src/lib.rs - ReadmeDoctests (line 503)`, untouched here |

The four excluded cases need `just loopback-aliases` (`sudo`) on macOS and are
unrelated to this change
([2026-09-18](2026-09-18-macos-loopback-and-two-load-flakes.md) § 1). They
were filtered rather than run and failed.

### The gates

`just fmt`: applied. `just clippy` (`cargo clippy --workspace --all-targets --
-D warnings`): clean. `just verify`: `verify: ok — the fifteen gates above
passed; the verify-docs report is advisory`, after this page's index entry
moved `docs/status/README.md`'s marked file count from 66 to 67.
`sql_audit`'s two tests ran inside the `vpay-db` count above with
`EXPECTED_ASSERT_SITES` unchanged at 71.

## What this does not do

- It does not run a skewed host (above).
- It does not move the facts in ADR-0026 D6, their sweeps (D7), or the two
  OAuth expiry columns (D8).
- No `docs/flows/` page was added or removed, no route or gate moved and no
  `NotImplemented` token was retired. Three trait signatures changed and one
  method was renamed; whether `vaam-apps/vpay-skills` cites any of them was
  not checked, and no skills PR was opened (out of scope for this change by
  instruction).
