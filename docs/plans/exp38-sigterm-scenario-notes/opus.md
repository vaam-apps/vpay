# exp38 — a third `worker_kill9` scenario: SIGTERM with work outstanding (issue #85)

Written 2026-09-10, on branch `claude/exp38-sigterm-scenario`, base `ff1f507`
(`master`). Tier `opus`, single pass, no draft to review.

## What the gap actually was

[../../flows/crash-safety.md](../../flows/crash-safety.md) said it in its own
words, and the sentence is why this task exists:

> **This is a measurement, not a test.** Nothing re-runs it, and a regression
> in the drain would be caught by no gate. Turning it into a third
> `worker_kill9` scenario is the obvious answer and was not done in this pass.

The measurement was the exp30 review's: `docker kill -s TERM` on the demo
stack's worker mid-settlement, exit **0** with the drain log lines, an
undelivered webhook that stayed undelivered while no worker ran, and the same
container restarted delivering it in ~6 s, signed.

Every SIGTERM in every suite went through `worker_kill9.rs`'s
`stop_worker_cleanly`, whose own assertion string says *"a worker with nothing
in flight"* — so the empty case was covered and the interesting one was not.

## The scenario

`a_worker_sigtermed_mid_delivery_drains_it_and_the_merchant_is_told_exactly_once`,
in the same file, bounded like the other two.

1. An ordinary confirm on `237600000c15`, a documentation MSISDN that arms
   **no** rail mapping: the rail answers a plain 202 and the catch-all
   `SUCCESSFUL`, because the thing made slow here is the merchant's receiver,
   not the rail. `the_sigterm_scenario_confirms_with_an_msisdn_that_arms_no_rail_mapping`
   is a container-free guard that keeps that true — it parses every mapping in
   the shared `mtn` tree and fails if any `request` block ever matches on that
   number.
2. **Two** shipping workers run for the whole scenario. One settles the
   charge, fans the event out and claims the `deliver_webhook` job; which one
   is read off `jobs.locked_by`, whose `worker_id` carries the pid, never
   assumed.
3. The POST is in the receiver's journal and the job is leased — two witnesses
   outside the process holding it. `slow-ack.json` holds the `200` for six
   seconds.
4. `SIGTERM`, to the claimant.
5. Exit **0**, with no signal (SIGTERM is caught, so a signalled death would
   be the regression) and no `Drain::TimedOut` warning.
6. Exactly **one** POST at the receiver, which `vpay_sdk::webhooks::verify`
   accepts under the configured secret; one delivery row `succeeded` with
   `attempt = 0`; its job deleted; and the same four-record exactly-once
   invariant the two `SIGKILL` cases assert.
7. The survivor is then watched past `vpay_worker::delivery_delay(0)` and must
   send nothing.

## Determinism, which was the hard part

A flaky scenario is worse than none, and "six seconds is probably enough" is
not an argument. Three things carry it, and none of them is a margin.

**The delay is pinned between two shipping budgets by the compiler.**

```rust
const _: () = assert!(
    RECEIVER_ACK_DELAY.as_secs() < vpay_worker::webhooks::WEBHOOK_REQUEST_TIMEOUT.as_secs(),
    "the receiver must answer before the delivery client's own deadline"
);
const _: () = assert!(
    RECEIVER_ACK_DELAY.as_secs() < SHUTDOWN_GRACE_SECONDS,
    "the drain must outlast the receiver, or the exit code under test changes"
);
```

Above the first (10 s) the request times out, the delivery walks a rung of the
ladder and a second POST goes out; above the second (20 s) the drain aborts the
task and the process exits `1`. Both are real behaviours and neither is this
case. Lowering either shipping constant now breaks the **build** rather than
making the test flaky, and `SHUTDOWN_GRACE_SECONDS` is the same constant
`spawn_worker` puts in the process's environment, so the comparison cannot go
stale.

**The signal is proved to have landed mid-delivery, from the victim's own
transcript.** Both lines go to stdout — `vpay-server` logs to stdout in every
long-running mode — and one thread reads that pipe, so the order in `seen` is
the order the process wrote them:

```
received SIGTERM, starting graceful shutdown      (vpay_config::signal)
shutdown signalled; draining in-flight jobs       (vpay_worker::run_loop)
webhook delivered                                 (vpay_worker::webhooks)
graceful shutdown complete, exiting               (vpay_server::worker)
```

The case fails unless those four appear in that order. A `webhook delivered`
before the drain began means the receiver answered before the signal arrived,
and the run is rejected with a message saying to raise `RECEIVER_ACK_DELAY`
rather than to relax the assertion. So the failure mode of a slow machine is
*a loud, self-describing failure*, not a green run that proved nothing.

**The overlap that the lease assertion needs is by construction.** See below.

## Why the second worker co-runs rather than restarting

The hand measurement restarted the container, and the obvious transcription is
"drain, then boot a fresh worker, then assert it sends nothing". That version
cannot prove the property it looks like it proves.

`Jobs::claim` matches `locked_at IS NULL` under `FOR UPDATE SKIP LOCKED`. For a
test to show that this is what stops a second worker double-sending, a second
worker has to be **asking** while the first still holds the lease. A worker
spawned after the signal spends its first seconds connecting, migrating and
reconciling; by the time it claims anything the drain is over and the job has
been deleted. The lease would be respected vacuously, on some runs and not
others, with nothing in the test able to tell which.

A worker that is already running when the signal lands overlaps by
construction. What is lost is the restart itself — and that is recorded as a
gap rather than glossed: **no case restarts a worker after a graceful stop.**
That half of the hand measurement is still only a measurement.

## Decisive mutations

Both were run against the shipping code and reverted; neither is argued.

| Mutation | Result |
|---|---|
| **Remove the drain** — `drain_tasks` aborts every task and returns `(Drain::Clean, 0)` the instant the signal is seen | **FAIL**: *the worker never logged `webhook delivered`*. The transcript shows `received SIGTERM` → `shutdown signalled` → `job loop stopped … drain=Clean` with no delivery between them. The receiver had already accepted the POST, so this is the lost delivery that becomes a double send as soon as anything retries |
| **Remove the lease respect** — `locked_at IS NULL` and `FOR UPDATE SKIP LOCKED` dropped from `vpay_db::Jobs::claim` | **FAIL** on the double-send assertion, `left: 2  right: 1`, with two byte-identical signed POSTs of the same `evt_…` in the receiver's journal |

The second mutation is also why the assertions are ordered the way they are.
On the first attempt it failed one assertion earlier, on *"a succeeded
delivery's job must be deleted"* — true, but internal bookkeeping. The
receiver's journal is what a merchant experiences, so that block was moved
ahead of the delivery-row block and the mutation re-run to confirm it now
reports the double send first.

## Ten consecutive runs

`cargo nextest run -p vpay-tests-integration -E 'test(a_worker_sigtermed_mid_delivery)'`,
ten times in a row, rootless Docker, no retries configured. On `841cedc`,
which is this commit before it was amended: `just ci`'s `fmt-check` then asked
for one call to be wrapped differently, so the amended head differs from the
tested one by whitespace and by where a line breaks inside a panic message.
The case itself runs again in the full gate below, on the final head.

| Run | Exit | Test wall clock |
|---|---|---|
| 1 | 0 | 48.3 s |
| 2 | 0 | 35.9 s |
| 3 | 0 | 33.9 s |
| 4 | 0 | 31.6 s |
| 5 | 0 | 38.8 s |
| 6 | 0 | 39.5 s |
| 7 | 0 | 30.4 s |
| 8 | 0 | 30.7 s |
| 9 | 0 | 32.2 s |
| 10 | 0 | 99.8 s |

**10 passed, 0 failed, 0 skipped.** Run 10 is three times the median and still
green, which is the run worth naming: the ordering assertion was armed for all
ten, so a machine slow enough to let the receiver answer before the signal
arrived would have failed rather than passed quietly.

## What this does NOT cover

Stated here rather than left to be discovered.

- **`Drain::TimedOut` under a real signal.** The grace period elapsing with a
  job still in flight — exit `1`, leases handed back — is proven by
  `a_drain_that_runs_out_of_grace_releases_every_lease_it_still_holds` in
  `worker_e2e.rs`, which drives `run_loop` in-process. No signalled shipping
  binary reaches that branch in any suite.
- **A restart after a graceful stop.** See above.
- **The rail is a WireMock container and the receiver is another one.** No real
  rail and no real merchant endpoint has ever been called, which is the same
  limit every other case on this page carries.
- **Orange Money.** Like the two `SIGKILL` cases, this one is `mtn_momo` only.
- **`SIGINT`.** `vpay_config::signal` handles both and the drain does not know
  which arrived; only SIGTERM is signalled here.

## Files

- `backends/tests/integration/tests/worker_kill9.rs` — the scenario, the
  MSISDN guard, and the helpers both needed (`receiver_posts`, `deliveries`,
  `delivery_job`, `Proc::{absorb, absorb_until_closed, said}`).
- `backends/tests/webhook-receiver/wiremock/mappings/slow-ack.json` — the
  slow acknowledgement, on a path of its own so `tests/webhooks.rs`'s ladder
  timings against the same mounted tree are untouched.
- `docs/flows/crash-safety.md` — the hand measurement converted into the named
  test, with the gaps that remain.
- `docs/status.md` — the retired paragraph and the re-titled table row.
