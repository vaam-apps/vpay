# Lane D notes — a real kill test

Branch `claude/step8-lane-d-killtest`, starting point HEAD `199cee0` (an interrupted agent's
checkpoint: the WireMock mapping, the test file and the `serde_yaml_ng` dev-dep were already
written but **never verified against real infrastructure** — the commit message says so
itself, "interrupted by API overload"). Everything in this note was re-verified from scratch
against real Postgres + WireMock + the shipping `vpay-worker-bin`/`vpay-server` binaries in
this working tree, not trusted from the interrupted transcript.

## What the checkpoint got wrong

Building compiled cleanly (the file only calls real, existing functions), but running it
against a real database failed both cases, for two independent reasons, both fixed in commit
`464e0cc`:

1. **`attempts()` selected columns that do not exist.** `provider_requests` (migration
   `0016_create-provider-requests.sql`) has `error_kind`/`sent_at`; the checkpoint's query
   asked for `error_code`/`started_at`. Every run failed before either test case's own
   assertions ran — `column "error_code" does not exist`. This is exactly the class of bug
   `cargo build` cannot see (the query is a runtime string, not a `sqlx::query!` macro) and
   only a real Postgres catches.
2. **The poll-kill case asserted the wrong status code for a successful submit.**
   `vpay_provider::Submitted` does not carry the rail's HTTP status line (ADR-0002: the core
   must not branch on a transport detail), so `vpay_db::provider_requests` records a
   *successful* submit with the `0` sentinel (`STATUS_CODE_NOT_CARRIED_BY_THE_PORT`,
   documented at length on that constant, migration `0020`) rather than MTN's real `202`. The
   checkpoint asserted `Some(202)`; the real row is `Some(0)`. Fixed by transcribing the same
   `ANSWERED_SENTINEL` constant `worker_recovery.rs` already uses, for the same documented
   reason.

Both were caught by actually running the test against a container, per this task's own
instruction to verify rather than trust the interrupted transcript — a `cargo build` alone
would have reported this file as finished.

## What is proven, and how

`backends/tests/integration/tests/worker_kill9.rs`, two `#[tokio::test]`s, no `#[ignore]`,
Unix-only (`#![cfg(unix)]`, the whole file — its subject is a POSIX signal):

- **`a_worker_killed_mid_poll_settles_the_charge_exactly_once_after_its_lease_is_reaped`** —
  the case the plan and `docs/flows/crash-safety.md` both name as unproven. A merchant
  confirms through the real API with a documentation MSISDN
  (`237600000ce9`) that arms a WireMock scenario delaying the **status query** (not the
  submit) by 30 s — longer than `vpay_provider::DEFAULT_REQUEST_TIMEOUT` (20 s), so a kill
  that lands late cannot let the request quietly settle the charge behind the assertions'
  back. The shipping `vpay-worker-bin` is spawned as a real OS process
  (`--worker-concurrency 1`), claims the poll job, and is `Child::kill()`ed (`SIGKILL`) once
  **two independent witnesses** agree the query is in flight: WireMock's own request journal
  (`POST /__admin/requests/count`) has seen it, and `provider_requests` has the row the worker
  wrote *before* opening the socket (`status_code IS NULL`, `error_kind IS NULL`) and cannot
  complete if killed. The exit status is asserted `signal() == Some(9)`, `code() == None` — a
  process that chose to `exit(1)` could not stand in for one that was killed. What survives is
  asserted directly: the job's lease is still held by the dead worker's own `worker_id`
  (`host/pid/random`, matched against the killed process's real pid), `attempts == 1`, the
  charge is still `submitted`, the intent still `processing`, `amount_received == 0`, no
  event. A second worker process starts, reaps the lease at boot (its own log line, "freed job
  leases whose worker never came back", is waited for), re-runs the poll, and the charge
  settles **exactly once**, checked four independent ways: one `charges` row, one
  `payment_intent.succeeded` event, `amount_received` the full amount once, and the rail's own
  journal shows exactly one submit and exactly two status queries (the killed one and the one
  that answered) — no third rung, no resubmit. `assert_one_reference` closes the loop: every
  `provider_requests` row for the charge carries the one `provider_reference_id` the confirm
  minted. The surviving worker is then stopped with `SIGTERM` and asserted to exit `0`
  (a clean drain, proving the second worker's own lifecycle is unaffected by having reaped a
  crash).

- **`a_server_killed_mid_submit_leaves_a_charge_the_worker_settles_without_a_second_submit`**
  — the `submitting` kill point (crash-safety.md's kill point 2), staged against the
  **shipping `vpay-server`**, not written into the database. It *was* stageable against the
  real binary with no seam: a second documentation MSISDN (`237600000cf9`) arms a WireMock
  mapping whose `requesttopay` response itself is delayed 30 s, so the real server's confirm
  handler is genuinely blocked inside the POST when it is killed. The server is `SIGKILL`ed
  once the rail's journal has the POST **and** the charge is committed `submitting` with an
  unanswered attempt row — kill point 2, exactly. What survives: the charge and its reference
  (unchanged), one unanswered `provider_requests` row, the `poll_charge` job committed in the
  same transaction as the charge (unclaimed — the server never runs jobs), and the intent
  still `requires_payment_method` (the confirm never reached `persist_submitted`). A worker
  then recovers it — `SubmitAttempt::Unanswered` → poll, never resubmit — and the charge
  settles once, with the rail's journal showing **exactly one submit**, which is the assertion
  the retry rule (`docs/flows/crash-safety.md`, "a fresh reference on retry is how you
  double-charge a customer") is actually about.

Both cases pass repeatably: `cargo nextest run -p vpay-tests-integration -E 'binary(worker_kill9)'
--no-fail-fast` → **2 tests run: 2 passed, 0 failed, 0 skipped** on a quiet host (~20s total);
also run with `--retries 2` → same 2/2, zero retries consumed. On a contended host (this
machine, shared with other lanes' concurrent container-backed suites) individual cases were
observed as slow as ~190s but never hung or flaked to a wrong outcome — see "Environment
contention" below.

## The lease flag question, answered

The plan says: "a short lease (use the existing flag if one exists — read `cli.rs`; else say
so and bound the wait on the default)". **No such flag exists.** `vpay_config::WorkerArgs`
(`backends/crates/vpay-config/src/cli.rs`) declares exactly one worker-specific flag,
`--worker-concurrency`/`VPAY_WORKER_CONCURRENCY`; there is no lease-duration flag anywhere in
`CommonArgs` or `WorkerArgs`. `vpay-worker-bin`'s `main.rs` constructs
`RecoveryPolicy::default()` unconditionally (`backends/apps/vpay-worker-bin/src/main.rs`,
around the `let policy = RecoveryPolicy::default();` line) — a five-minute lease
(`backends/crates/vpay-worker/src/recovery.rs`), with no way to override it from outside the
process short of rebuilding it with different source.

So the test does not wait for the real five minutes. `age_the_dead_workers_lease` (in
`worker_kill9.rs`) moves the killed worker's `jobs.locked_at` ten minutes into the past,
`UPDATE`-guarded on `locked_by = <the dead worker's own worker_id>` so it cannot free anything
that belongs to a different job. This is the same technique, and the same justification, as
`support::make_every_job_runnable` (ages `run_at`) and
`worker_recovery.rs::strand_the_poll_job` (writes `locked_at` directly): the test controls the
queue's *clock*, not the queue's *code path* — the lease really was taken by a really-killed
process, `locked_by` really is that process's own `worker_id`, and the reap, the claim and the
re-run that follow afterward are all the shipping binary's own logic running unmodified. The
file's own module doc (`worker_kill9.rs:16-18`) states this plainly: "Nothing here is
simulated except the passage of time … which is the one thing this file cannot wait for."

This is the one and only thing in the file not driven by real elapsed time, and it is
documented as such rather than hidden. No CLI flag was added — adding one only to serve this
test would be exactly the kind of test-only seam AGENTS.md rule 1 forbids (a knob that exists
so a test can avoid the real code path, rather than a real deployment lever).

## Guard-failure proof

`vpay_db::jobs::reap_expired_leases` (`backends/crates/vpay-db/src/jobs.rs`) was temporarily
replaced with a no-op returning `Ok(0)`, and the poll-kill case re-run in isolation:

```
thread 'a_worker_killed_mid_poll_settles_the_charge_exactly_once_after_its_lease_is_reaped'
panicked at backends/tests/integration/tests/worker_kill9.rs:963:5:
the second worker must reap the dead one's lease at boot — `claim` matches only
`locked_at IS NULL`, so nothing else would ever pick this charge up
```

The second worker boots successfully, runs its housekeeping sweep
(`vpay_worker::handlers: housekeeping sweep … expired_leases=0`), and answers `/livez` — it
looks perfectly healthy — but never logs "freed job leases whose worker never came back" and
never claims the stranded job, because the reaper it depends on was disabled. The test fails
**deterministically, within its bound** (`BOOT_TIMEOUT` = 60 s on the reap-line wait; total
wall time 163 s including the earlier setup and in-flight waits) — not a hang, not a false
green. The change was reverted immediately after
(`git diff backends/crates/vpay-db/src/jobs.rs` is empty on the tree this note describes); the
revert was itself re-verified by rerunning both cases clean (2 passed, 0 failed).

## Environment contention encountered, and how it was handled

Two host issues surfaced while verifying this lane, both **environmental, not code defects**,
and both are documented here rather than worked around silently:

1. **A shared `CARGO_TARGET_DIR` corrupted this worktree's build.** The task's own environment
   variables point `CARGO_TARGET_DIR` at `/home/selast/dev/vpay/.claude/worktrees/step8-target`
   — the *same* directory another concurrently running lane (`step8-lane-b-ssrf`, a sibling
   worktree, observed live via `ps aux` adding a `webhooks` field to `vpay_config::Config` as
   part of its own SSRF work) was also building into at the same time. A run under that shared
   directory failed to compile with `error[E0063]: missing field 'webhooks' in initializer of
   'vpay_config::Config'` at four call sites in **this worktree's own, unrelated files**
   (`worker_kill9.rs`, `merchant_token_flow.rs`, `webhooks.rs`, `browser_checkout.rs`) — but
   this worktree's own `vpay_config::Config` (`backends/crates/vpay-config/src/config.rs`) has
   no such field; `webhooks` lives on `MerchantClient`, not `Config`. Building the identical
   source with an **isolated** `CARGO_TARGET_DIR` compiled cleanly on the first try, which
   confirms the failure was cross-contamination between two worktrees sharing one target
   directory, not a real error in this tree. Every build/test/clippy/fmt run in this note past
   that point used an isolated `CARGO_TARGET_DIR`
   (`$SCRATCHPAD/isolated-target`) to get a trustworthy result; the shared directory named in
   the task instructions is not safe to use for two lanes building overlapping crates at once.
   **This should be raised with whoever orchestrates the four lanes:** either serialise their
   builds, or give each lane its own `CARGO_TARGET_DIR`.
2. **Rootless-Docker inotify pressure** (previously recorded in this user's own memory,
   `vpay-testcontainers-docker-host.md`): `fs.inotify.max_user_instances` is 128 and this
   desktop session plus several concurrent test runs from sibling lanes were holding ~122 of
   them, which occasionally produced `testcontainers` `container startup timeout` /
   `failed to create a container: Timeout error` on an *unrelated* container start, not a
   failure of this suite's own logic. No `sudo` was available non-interactively to raise the
   limit, so this was not fixed at the host level; the mitigation applied was the one the
   memory note already recommends — remove `Created`-state container debris left by earlier
   failed starts, and treat a `container startup timeout` as evidence of host contention,
   re-running rather than treating it as a test failure. Every pass/fail reported above is
   from a run that reached the test's own assertions (no infrastructure-level failure), and
   the final gate run (`--retries 2`) needed zero retries.

Neither issue is specific to `worker_kill9.rs`; both are properties of the shared multi-lane
host, recorded here because they were encountered and worked around while proving this lane,
and because a future lane hitting the same "impossible" compile error should look here first
rather than doubting their own source.

## What is proven, stated plainly

- A real `SIGKILL` to the real `vpay-worker-bin`, mid-status-query, against a real Postgres and
  a real WireMock rail: the crash loses nothing, a second worker's boot-time reap recovers the
  lease, the poll re-runs, and the charge settles exactly once by four independent counts.
- A real `SIGKILL` to the real `vpay-server`, mid-`requesttopay`, staged against the shipping
  binary with no test-only seam: the write-first ordering
  (`docs/flows/crash-safety.md`) really does survive a signal that runs no destructor, and the
  worker's recovery table really does resolve it without a second submission.
- The reaper's necessity: disabling it produces an observable, bounded test failure rather
  than a silent gap or a hang.

## What is not proven / explicitly out of scope for this lane

- **Kill point 1** (crash before the `charges` insert / before any `provider_requests` row) is
  not staged against a real process here — there is no network call to delay at that point (it
  is the moment *before* the reference is minted), so there is nothing for a real `SIGKILL` to
  land "during"; `worker_recovery.rs` continues to be the only proof of that case, by writing
  the state directly. This narrows, rather than removes, `docs/flows/crash-safety.md`'s "the
  states are written, not caused" caveat — it now applies to one of the three kill points
  instead of all three.
- **Orange Money** is not exercised here — both cases use `mtn_momo`; the plan named MTN's push
  flow specifically ("its status query is in flight") and this lane did not extend the WireMock
  kill9 mapping tree to Orange. A redirect-rail kill test would need its own scenario (Orange's
  ordering is reversed — see `docs/flows/crash-safety.md`'s "Redirect rails" section) and is not
  attempted here.
- This is still a **WireMock rail**, as every other test in this repository is — nothing here
  calls a real MTN or Orange sandbox. The claim is "the recovery table is executed correctly
  under a real signal", not "the rails behave as documented".

## `docs/flows/crash-safety.md` sentences this lane retires (for lane E)

The **Tests** section currently reads (verbatim, as of this lane's start):

> **They are exercised by writing the state a crash leaves, not by killing a
> process.** … That is a weaker claim than "a `SIGKILL` at each point resolves cleanly", and
> it is stated this way on purpose: it proves the recovery table, not the process's behaviour
> under a signal. **Nothing in this repository kills a process mid-confirm.**

The final sentence, "Nothing in this repository kills a process mid-confirm", is now false —
`worker_kill9.rs` kills `vpay-server` mid-`requesttopay` (kill point 2) and `vpay-worker-bin`
mid-status-query (crash-safety's own "poll" recovery path). Lane E should retire that sentence
and the "weaker claim" framing for those two moments specifically, while keeping the true part
— kill point 1 is still proven only by writing the state, not by a signal (see "What is not
proven" above).

The **"What is still not built"** section's bullet:

> **No `SIGKILL` test.** See the Tests section above: the states are written, not caused. A
> real kill-and-restart test would additionally prove the process's behaviour under a signal,
> and nothing does.

should be removed (or narrowed to name kill point 1 only), and this lane's evidence — the two
`worker_kill9.rs` cases, the file:line references above, and the measured pass counts — cited
in its place.

`docs/status.md`'s own Step 4 note also says, in passing: "the crash tests write the state a
crash leaves rather than killing a process" (around the paragraph beginning "**What that pass
added was the worker**"). That clause is now only true of kill point 1 and should be narrowed
the same way.

## Status rows lane E should add/change (verbatim, for `docs/status.md`)

Suggested new row, under the Worker / crash-safety section, alongside the existing "Worker job
loop" and "Charge submission" rows:

| Real `SIGKILL` crash test (`backends/tests/integration/tests/worker_kill9.rs`) | ✅ (two of three kill points) | Real `Child::kill()` (`SIGKILL`) to the shipping `vpay-worker-bin` mid-status-query and to the shipping `vpay-server` mid-`requesttopay`, against real Postgres + WireMock, no test double. `a_worker_killed_mid_poll_settles_the_charge_exactly_once_after_its_lease_is_reaped`: the lease and an unanswered `provider_requests` row are the only trace after the kill; a second worker's boot-time reap recovers it and the charge settles exactly once (one `charges` row, one event, `amount_received` once, exactly one submit + two status queries in the rail's own journal). `a_server_killed_mid_submit_leaves_a_charge_the_worker_settles_without_a_second_submit`: the server dies with the POST issued and unanswered; the worker recovers by polling, never resubmitting (one submit in the journal). Guard-failure proof: disabling `vpay_db::jobs::reap_expired_leases` leaves the second worker booted, healthy and permanently unable to claim the stranded job — the test fails deterministically inside its bound. **Kill point 1** (before any `provider_requests` row exists) is not staged against a real process — there is no network call to interrupt at that moment — and remains proven only by `worker_recovery.rs` writing the state directly. Neither Orange Money nor a real rail is exercised here. |

## Gate results (this lane, isolated `CARGO_TARGET_DIR`)

- `cargo fmt --all -- --check` (this project's own `fmt-check` recipe — plain stable
  `cargo fmt`, not nightly; `rustfmt.toml` deliberately avoids nightly-only options) — **clean**
  after `cargo fmt --all` was run once to fix the checkpoint's own formatting (import
  line-wrapping, an `if let`/`.is_none()` chain, a closure body) — commit `464e0cc`.
- `cargo clippy -p vpay-tests-integration --all-targets -- -D warnings` — **clean, zero
  warnings**.
- `cargo nextest run -p vpay-tests-integration -E 'binary(worker_kill9)' --no-fail-fast
  --retries 2` — **2 tests run: 2 passed, 0 failed, 0 skipped, 0 retries consumed** (measured
  twice; a third run without `--retries` also 2/2).
- `just verify` (`verify-no-mocks` + `verify-status` + `verify-errors`) — see the top-level
  summary; this lane adds no mock, no new `NotImplemented` token, and no new `pub …Error` type,
  so it changes nothing these three checks look at.
- `just verify-ignored` — this lane adds **one** new test binary
  (`worker_kill9`, 2 tests, 0 ignored). `expected_suites` bumped 39 → 40 and the `min_tests`
  comment/history updated in the same commit as this note; see the justfile diff.
