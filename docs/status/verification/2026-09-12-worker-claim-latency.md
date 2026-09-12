# Verification log — 2026-09-12, what a single worker's claim latency actually is

Last verified: 2026-09-12, on branch `claude/issue-100`, off `7914fc2a`.
This closes [issue #100](https://github.com/vaam-apps/vpay/issues/100), which
was the unfinished sentence at the end of the exp38 review
([2026-09-10.md](2026-09-10.md),
[../../plans/exp38-sigterm-scenario-notes/opus-review.md](../../plans/exp38-sigterm-scenario-notes/opus-review.md)):
that review staged its `Drain::TimedOut` scenario against **one**
`--worker-concurrency 1` worker, watched the fixture fail to reach its
in-flight state on four attempts out of six, moved to two workers — 10/10 —
and wrote down plainly that it had **not** diagnosed the mechanism. The issue's
decision was: measure the single-worker claim latency under a small backlog,
pin an upper bound as a test, and fix the cadence or the batch **only** if the
bound is not met.

## What this note claims, and what it does not

It claims a measurement of `vpay_worker::run_loop`'s claim latency at
`concurrency = 1` and at `concurrency = 2`, taken against a real Postgres with
the shipping loop and the shipping housekeeping running; that the latency is
one `IDLE_SLEEP` plus milliseconds in both; that the bound now pinned is red
for the two regressions that would produce the starvation the issue describes;
and that **no shipping code was changed**, because the bound is met.

It does **not** claim a diagnosis of those four exp38 failures. Re-staging
that fixture with one worker was not attempted here — what is measured below
is the loop, not that scenario — so the honest statement about them is what
the numbers _rule out_, in "What this rules out" at the end, and not what they
were.

## The measurement

`backends/tests/integration/tests/worker_claim_latency.rs`. One
`vpay_worker::run_loop` against a real Postgres container; a backlog of **8**
jobs enqueued in one transaction through `UnitOfWork::enqueue_in_tx`, ten
times; the claimable set sampled every 5 ms, so each number carries at most
one sample interval of quantisation. The zero of every number is the **commit**
of the enqueue — before it no worker can see the rows at all, which also keeps
the measurement off any host-clock/database-clock comparison.

The probes are `scan_live_charges` rows under this suite's own dedupe keys: a
real kind (migration 0034's `kind_is_known` accepts it, and the insert is the
shipping one), whose handler on an empty `charges` table is one indexed read
and no write — so the interval is the loop's cadence and not a handler's work.
The five housekeeping singletons `run_loop` seeds are **live throughout**,
deliberately: exp38 blamed them.

```
cargo nextest run -p vpay-tests-integration --test worker_claim_latency -j 1 --no-capture
```

2026-09-12, rustc 1.98.0, `postgres:16-alpine`, on a host carrying other
agents' suites (load average 13 at the start of the session). 10 rounds × 8
jobs per concurrency; both cases green in 22.6 s.

| Concurrency | first claim (min / median / max) | last claim (min / median / max) | drain span, first→last (min / median / max) |
| ----------- | -------------------------------- | ------------------------------- | ------------------------------------------- |
| **1**       | 6.07 ms / 997 ms / **1.004 s**   | 11.7 ms / 1.015 s / **1.028 s** | 0 / 18.5 ms / **27.4 ms**                   |
| **2**       | 6.72 ms / 999 ms / **1.004 s**   | 13.1 ms / 1.005 s / **1.017 s** | 0 / 12.9 ms / **19.5 ms**                   |

Read it as: a backlog that lands while the task is inside its idle sleep waits
out the remainder of that one second and no more (`997 ms`–`1.004 s`); one that
lands while the task is awake is claimed in **6–12 ms**; and once claiming, all
eight jobs are gone in **under 30 ms**, because after a non-empty claim the
loop goes straight back to `Jobs::claim` with no sleep at all. Two earlier runs
of the same case on a busier host gave the same shape, with one outlier at
**1.837 s** — the number the bound's headroom is sized for.

**The housekeeping cost, measured rather than argued.** Both runs report
`claimed = 86` for 80 probes: **6 housekeeping claims** over the ~9 s of
measuring. In steady state the only recurring singleton is the outbox drain
(`FAN_OUT_IDLE`, 5 s); `scan_live_charges` and `scan_deliveries` are 10 minutes
apart and the two sweeps an hour. At the per-job service cost the drain span
shows (~2–3 ms), a single claim task spends well under 1% of itself on the
housekeeping.

## The bound that is now pinned

Two, in the same file, asserted on every round of both cases:

- **`CLAIM_BOUND` = 3 s** — every job of the backlog is claimed within three
  seconds of becoming claimable. Three times the only wait the loop can impose
  on an arrival (`IDLE_SLEEP`, 1 s, paid once), which leaves it insensitive to
  a loaded host and still red for anything that would starve a queue.
- **`DRAIN_SPAN_BOUND` = 1 s** — first claim to last claim of the eight.
  Measured in tens of milliseconds. This is the one that distinguishes "the
  loop waits once" from "the loop waits per job": a per-job cadence smaller
  than `CLAIM_BOUND / 8` would slip past a total-time bound and still starve a
  real queue.

Both are literals, not expressions over `IDLE_SLEEP`: a bound spelled
`IDLE_SLEEP * 3` would widen itself the moment somebody widened the cadence,
which is the one change it exists to catch.

## The mutations, and what each printed

Both applied to `backends/crates/vpay-worker/src/run_loop.rs`, measured, and
reverted; `git diff` was checked empty after each.

| Mutation                                                          | Result                                                                                                                                                                                                          |
| ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `IDLE_SLEEP` 1 s → **5 s** (slow the poll cadence)                | **FAIL**, round 2: _"the last of 8 jobs was claimed 5.011795968s after the backlog became claimable, over the 3s bound"_. first/median/max claim `5.001 s` / `5.011 s` / `5.039 s`                              |
| `idle()` after **every** claimed job, not only after an empty one | **FAIL**, round 0, and the curve is the starvation shape itself: `[6.6ms, 1.013s, 2.016s, 3.016s, 4.019s, 5.026s, 6.034s, 7.030s]` — one second per job. Busts `DRAIN_SPAN_BOUND` (7.03 s) as well as the total |

The second is the exact failure mode issue #100 names — "the loop did not claim
the delivery in time" — staged deliberately, and the case is red for it.

## The verdict: the loop was not changed

`CLAIM_BOUND` is met with roughly a factor of three in hand, at both
concurrencies, on a loaded host. `IDLE_SLEEP` stays **1 s**, the claim stays
**one row per statement**, and nothing in `run_loop.rs` or `vpay_db::jobs`
moved. A change to the cadence or the batch here would have been a fix for a
defect that is not there — and it would have cost idle database load on every
deployment, which is what that constant's doc comment says it trades.

## What this rules out, and what it leaves open

- **Ruled out: "the singleton jobs".** The exp38 note's stated hypothesis was
  the settlement poll "waiting tens of seconds behind the singleton jobs the
  single `--worker-concurrency 1` task also has to run". Six housekeeping
  claims in nine seconds, at ~2–3 ms each, cannot account for tens of seconds.
- **Ruled out: the claim path being the one-vs-two difference.** Two claim
  tasks measure the same as one, round for round, and the case that says so is
  in the suite (`two_workers_are_held_to_the_same_bound_as_one`).
- **Left open, and it is real:** with `concurrency = 1` a worker runs **one job
  at a time**, so anything ahead of an arrival delays it by its whole duration
  — in that fixture, a delivery POST the receiver holds for
  `RECEIVER_ACK_DELAY` (6 s). That is a property of the concurrency a
  deployment chooses, not of the loop's cadence, and it is not what the bound
  above is about.
- **Not attempted:** re-staging exp38's timed-out scenario with a single
  worker. What those four failures were is therefore still unwritten; what they
  were _not_ is the two bullets above.

## Gates run

`just ci` was **not** run: the brief for this work forbade it (five agents
were building on this host, and a workspace build has OOM-killed it before).
What was run, scoped:

| Command                                                                                                                                                                               | Result                                                                                       |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `cargo nextest run -p vpay-worker`                                                                                                                                                    | **75 run, 75 passed, 0 skipped, 0 ignored**                                                  |
| `cargo nextest run -p vpay-tests-integration --test worker_claim_latency -j 1`                                                                                                        | **2 run, 2 passed**, 22.6 s; re-run on the commit this page is in: **2 passed**, 19.6 s      |
| `cargo nextest run -p vpay-tests-integration --test worker_kill9 -j 1 -E 'test(sigterm) + test(a_drain_that_runs_out_of_grace_under_a_real_signal_exits_1_and_hands_the_lease_back)'` | **4 run, 4 passed**, 177.8 s (the two SIGTERM scenarios and their two container-free guards) |
| `cargo clippy -p vpay-tests-integration --all-targets -- -D warnings`                                                                                                                 | clean                                                                                        |
| `cargo fmt --check`                                                                                                                                                                   | clean                                                                                        |

The re-run on the commit this page is in gave `1.017 s` and `0.999 s` for the
two last-claim maxima, i.e. the same numbers as the table to within a
millisecond or two — which is the point of a bound with a factor of three in
hand.

**Two container-start failures on this host, neither of them a property of
anything under test.** The first attempt at the `worker_kill9` filter failed on
`failed to create a container: Timeout error` with eleven containers already
running (load average 13); it passed on the retry with `-j 1`. One attempt at
the new suite failed the same way, at the same 120 s testcontainers ceiling,
and the attempt after it saw a container take about 100 s to start and then
pass. That is the same environmental failure the exp38 review recorded twice,
and it is recorded here because a measurement taken on a host in that state
has to say so. Neither failure is specific to this suite: every
container-backed test in the repository starts its own `postgres:16-alpine`,
and `.config/nextest.toml` already serialises those starts for exactly this
reason.
