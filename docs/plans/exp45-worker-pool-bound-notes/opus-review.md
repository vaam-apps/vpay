# exp45 review — the worker concurrency bound, adversarially

Sabotage review of `claude/exp45-worker-pool-bound` at `6cfd78e` (base
`2c5ef8b` = master; five haiku commits). Everything below is a measurement
taken on this branch on 2026-09-10 against `postgres:16-alpine` under the
pinned 1.98.0 toolchain, cratestack 0.12.0 and Node 22.23.2 (`.nvmrc`), or a
citation into the sources named. Where a claim of the draft's is reproduced it
says so; where one is refuted, the command and its output are recorded.

PR #102 (exp42) had not merged when this review ran, so the rebase STEP 0
anticipates did not happen and no `ConfigError` conflict was resolved. The
variant this branch added to `ConfigError` is *removed* by F2 below, so that
conflict no longer exists in either direction.

---

## 0. The gate, as delivered — **it does not pass**

`just ci` recipe by recipe on `6cfd78e`, each run separately, exit code read
from a file:

| Recipe | Exit | Wall |
|---|---|---|
| `fmt-check` | 0 | 1 s |
| `clippy` | 0 | 51 s |
| `verify` | 0 | 14 s |
| `test-rust` | **100** | 538 s |
| `test-doc` | 0 | 6 s |
| `verify-ignored` | 0 | 1 s |
| `lint-web` | 0 | 22 s |
| `test-web` | 0 | 10 s |
| `deny` | 0 | 1 s |

```
FAIL [5.030s] (1328/1666) vpay-server::cli
  worker::a_worker_concurrency_of_pool_max_divided_by_two_boots_and_logs_that_concurrency
the boot must log the concurrency as accepted, got: vpay-server: connecting to
Postgres: failed to connect to Postgres: pool timed out while waiting for an
open connection
Summary [346.589s] 1328/1666 tests run: 1327 passed (1 slow), 1 failed, 0 skipped
```

The draft reported this case as delivered proof and did not run `test-rust`.
It is F1.

---

## 1. Findings

### F1 — the branch does not pass `just ci`: the new "boots" case asserts on a stream the worker does not log to · gate-hole

`a_worker_concurrency_of_pool_max_divided_by_two_boots_and_logs_that_concurrency`
read `Output::stderr` and looked for `job loop concurrency`. Worker logs go to
**stdout** — `main.rs`'s `logs_to_stderr` is a named function whose entire doc
comment is that a `worker` variant returning `true` would be "a container whose
logs vanish from `docker compose logs`". So the assertion searched an empty
string and could only fail.

Two further things were wrong with the case beneath that:

* Its name claims a boot it never performs. It supplied
  `UNREACHABLE_DATABASE_URL` and asserted exit **69**, which proves only that
  *some* guard did not fire before the pool was opened. A guard refusing 5 and
  a guard refusing 6 both leave that assertion green.
* Written the obvious second way it fails again: `tracing_subscriber`'s text
  formatter colours field names *into a pipe*, so a line arrives as
  `\x1b[3mconcurrency\x1b[0m\x1b[2m=\x1b[0m5` and `contains("concurrency=5")`
  is a substring that is never present. Measured with `cat -A` on the real
  binary's stdout.

Fixed in `b8d7c90`: the case is now `with_live_postgres`, reads stdout through
a documented `strip_ansi`, asserts the accepted concurrency *and* the next boot
step (`database connected and migrations applied`), then SIGTERMs and requires
exit 0.

### F2 — the refusal was a `vpay_config::ConfigError`, which is the one home the enum next door argues against · conventions (blast radius)

`StartupError` in `vpay-server/src/main.rs` carries this exact class of failure
and says why in its own doc comment:

> **Defined in the binary, not in `vpay-config`, deliberately** — which inputs
> a process requires is a property of *that process*.

The rule joins a CLI flag to `vpay_db::MAX_CONNECTIONS`. `vpay-config` does not
depend on `vpay-db`, cannot see the constant, and would have had to carry a
sentence about connection pools in a public enum that is otherwise entirely
about the YAML document and the flags. It is also worker-only, exactly like
`StartupError::UnusableConcurrency` beside it.

Moved in `b8d7c90` to `StartupError::WorkerConcurrencyExceedsPoolSize`. Same
`Category::Configuration`, same exit 78 — asserted by the same subprocess case
— and one fewer public variant on a library crate. It also removes the
`ConfigError` rebase conflict with exp42 (#102) the brief expected.

### F3 — the message named three numbers and not the rule · misleading-claim (low)

`worker concurrency 6 exceeds pool size (max 5, pool 10); pass
--worker-concurrency 5 or VPAY_WORKER_CONCURRENCY=5` satisfies "names both
numbers" and leaves an operator with no way to tell whether 5 is a property of
their deployment or of the build, and no idea what to do for more throughput.
The replacement says what a fan-out does to the pool, that the pool size is a
constant in the image, and that the answer above the ceiling is another
replica. Both spellings and all three numbers survive, asserted with `&&`.

### F4 — the rule itself was never measured, and two shipping documents said so in the present tense · misleading-claim (high)

`pool.rs` and `docs/reference/vpay-db.md` both read "Nothing enforces the
relationship between this constant and that flag, and nothing measures it under
load." The draft made the first half false and left both sentences standing.
Worse, `docs/flows/crash-safety.md` gained a *new* claim that is not true:

> Exceeding this limit causes **every** crash-recovery fan-out to queue on
> `ACQUIRE_TIMEOUT`, which turns recovery into a hang.

Measured (§2): it does not, and it cannot — `fan_out_events` is a **singleton**
job, so one worker process has at most one fan-out in flight whatever
`--worker-concurrency` says. The issue's "ten fan-outs want twenty connections"
describes a state the queue cannot produce.

Fixed in `2d86b29` (the measurement and the two Rust-side documents) and
`4df0f93` (crash-safety.md rewritten around the measured table).

### F5 — `ci/values-full.yaml` shipped `worker.concurrency: 8`, which this binary refuses · correctness

Found by the guard the brief asked for, on its first run. The chart's own
"everything turned on" example describes a release that installs and then
CrashLoopBackOffs. The draft checked only the *default* (4) and concluded the
chart satisfied the rule. Fixed in `76ce196`, along with the guard
(`worker-concurrency-pool`), its `ci/guards/` fixture and three stale guard
counts the change made wronger (`justfile` said seventeen against eighteen
files; the chart README and `docs/flows/deployment.md` §7 both said 15).

### F6 — the runbook told an operator to turn the knob that is now capped · correctness (operational)

`docs/runbooks/worker-queue.md` answered a rising `queue_behind_seconds` with
"the knob for it is `--worker-concurrency` or another replica". Following that
page during an incident now stops the pod that was draining the queue, with
exit 78. Fixed in `4df0f93`: capped at 5, with the alternative named.

### F7 — deliverables the draft did not produce · process

No `docs/plans/exp45-worker-pool-bound-notes/` entry (this file), no
`docs/status.md` row, no `deployment.md` sizing paragraph, no
`configuration.md` row, no Helm guard, and `just ci` never run. The draft
reported `verify` as "10 sub-gates"; it is **twelve** on this base, and the
recipe is the list.

---

## 2. The bound: derived, then measured

**Derived.** One fan-out on the already-exists branch holds two pooled
connections (`upsert_do_nothing_authorize.rs` re-checks the update policy on
`runtime.pool()` while the caller's transaction holds its own). A worker's
other database users hold one each: `concurrency` claim loops, the lease
reaper, the gauge loop. The arithmetic worst case is therefore
`2·concurrency + 2 ≤ MAX_CONNECTIONS`, i.e. `(MAX_CONNECTIONS - 2) / 2 = 4`,
which is *stricter* than the issue's `MAX_CONNECTIONS / 2 = 5`.

**Measured.** A throwaway probe (the committed case's shape, run at several
widths: N transactions opened and held on a barrier, then all N asking for
their second connection at once, with `reap_expired_leases` — the crash-recovery
path — issued against the same pool at that instant):

```
EXP45-PROBE n=4  pool=10 tx_opened=4  probe_failures=0  slowest_probe=10.353ms  reaper=ok     reaper_waited=1.489ms
EXP45-PROBE n=5  pool=10 tx_opened=5  probe_failures=0  slowest_probe=7.533ms   reaper=ok     reaper_waited=1.158ms
EXP45-PROBE n=6  pool=10 tx_opened=6  probe_failures=0  slowest_probe=832.69µs  reaper=ok     reaper_waited=1.497ms
EXP45-PROBE n=8  pool=10 tx_opened=8  probe_failures=0  slowest_probe=1.781ms   reaper=ok     reaper_waited=1.250ms
EXP45-PROBE n=10 pool=10 tx_opened=10 probe_failures=10 slowest_probe=5.0019s   reaper=FAILED reaper_waited=5.0017s
```

Three conclusions, and the middle one is the one that matters:

1. **The empirical bound is 10, not 5 and not 4** — it is *higher* than the
   arithmetic, not lower. The second connection is released as soon as the
   policy probe answers, so one free connection serves every waiting probe in
   under two milliseconds. Nothing queues until all `MAX_CONNECTIONS`
   connections are pinned, and then everything does, for exactly
   `ACQUIRE_TIMEOUT` (5.0017 s measured, 5 s configured).
2. **The `(MAX_CONNECTIONS - 2) / 2 = 4` worry is answered by the n=5 column**:
   with five simultaneous fan-outs the reaper was served in 1.2 ms. There is no
   measurement here that justifies tightening the ceiling to 4.
3. The brief's conditional — "if the empirical bound is lower than the
   arithmetic one, the check must use the measured one" — **does not fire**.
   The guard keeps issue #63's `MAX_CONNECTIONS / 2`, which is the maintainer's
   number, and the documents now say plainly that it is the conservative
   reading rather than a cliff.

`n=12` and `n=16` were attempted and hung: with a pool of 10, twelve tasks
cannot all open a transaction, so two of them never reach the barrier. That is
a property of the probe, not of the code under test, and it is why the
committed case pins exactly `MAX_CONNECTIONS`. The probe was deleted; the
committed case is
`the_boot_guards_maximum_concurrency_fits_the_pool_and_a_saturated_one_starves_the_reaper`
(`backends/tests/integration/tests/webhooks.rs`), which keeps the n=5 row and
the n=10 control.

### Left to the maintainer

`pool.rs` reserves the ratio between `MAX_CONNECTIONS` and
`--worker-concurrency` to the maintainer, and issue #63 decided one half of it
(refuse above `MAX_CONNECTIONS / 2`). What this review deliberately did **not**
decide, having measured it:

1. Whether the ceiling should be relaxed now that the cliff is measured at
   `MAX_CONNECTIONS` rather than at half of it. It was left conservative.
2. Whether `MAX_CONNECTIONS` itself should rise. It is 10 for a deployment that
   has never run, and N worker replicas is N × 10 connections against a
   Postgres whose own `max_connections` is typically 100.
3. Whether the upstream ask recorded in `docs/plans/exp18-notes/opus-review.md`
   §4 — authorise through the caller's transaction — makes the whole ceiling
   unnecessary. If CrateStack takes it, one fan-out holds one connection and
   this guard's reason evaporates.

---

## 3. Mutations

Every one was applied, run, and reverted.

| # | Mutation | Expected | Observed |
|---|---|---|---|
| M1 | Delete the guard from `worker::boot` | over-ceiling case fails, exit 69 | **FAIL**, `left: Some(69) right: Some(78)`; ceiling case still passes |
| M2 | Move the guard after `open_migrated_database` | over-ceiling case fails, exit 69 | **FAIL**, `left: Some(69) right: Some(78)` |
| M3 | `>` becomes `>=` (off-by-one at the ceiling) | ceiling case fails | **FAIL**, "a concurrency of exactly MAX_CONNECTIONS / 2 must be accepted and logged" |
| M4 | Neuter the chart guard's `fail` | `just helm-check` names it | `helm-check: FAIL — guard 'worker-concurrency-pool' did not fire` |
| M5 | (measurement, not a mutation) pin 8 connections instead of 10 in the control | reaper succeeds | reaper ok in 1.25 ms — the control's `expect_err` is not vacuous |

M1 is the mutation the brief named. M3 is the one that says the boundary is the
boundary: it is caught only by the case F1 rewrote, and would have passed
against the draft's version had that version run at all.

---

## 4. What this review did not do

* **Did not tighten the ceiling to 4**, and did not raise it to 10. Both are
  the maintainer's; §2 records what a decision would now be made on.
* **Did not touch `docs/plans/exp18-notes/opus-review.md`** — it is a dated
  record of what was known on 2026-09-06, and §3 item 4 of it was correct when
  written. This file is the answer to it.
* **Did not run `just docs-check-citations`** (network + token) or
  `just test-e2e` / `just demo-walk`; nothing here touches the compose stack or
  the browser suites. The chart is still a thing that has never been applied to
  a cluster, and `just helm-check` says nothing about one.
* **Did not measure under real load.** Every number in §2 is a barrier-driven
  test on one machine against a local container. It bounds the *connection
  arithmetic*, not throughput.
