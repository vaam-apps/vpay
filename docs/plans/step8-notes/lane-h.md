<!-- Per-lane notes for Step 8. Lane E (the orchestrator) edits docs/status.md,
docs/roadmap.md and docs/flows/*.md from these, so the lanes never fight over
one table. Everything in §6 and §7 below is written so it can be applied
verbatim. This file is history once Step 8 lands. -->

# Step 8, lane H — the correctness review's four confirmed findings

Branch `claude/step8-review-r1`, on the gate head `753cfb0`. This lane fixes
what Step 8's correctness review confirmed against lanes B, C, D and G, and
records the two findings it deliberately did **not** fix.

| # | Finding | Commit |
|---|---|---|
| F1 | The recovery window compared Postgres' `created_at` against the **worker host's** clock, so a worker ≥60 s fast made lane G's guard a silent no-op | `5ba6b11` |
| F3 | `RecoveryAction::Wait` rescheduled at `poll_delay(0)`, and every reschedule spends a rung, so a crashed charge burned ~6 rungs waiting the window out | `605f4da` |
| F4 | The callback route's pull-forward matched any unleased future job, so an anonymous caller drove rail traffic at their own rate — and the module doc said the opposite | `6987e31` |
| F6 | The SSRF classifier let `192.88.99.0/24`, `2001:1::/32`, `2001:2::/48` and `2001:20::/28` through as ordinary public addresses | `8508b31` |
| F5, F7 | Recorded below, **not fixed** | — |

---

## 1. F1 — one clock, and the proof that it is one

`recovery_step`'s age guard (lane G) and `past_the_horizon` both computed
`OffsetDateTime::now_utc() - charges.created_at`. The minuend was the worker
host's clock and the subtrahend was Postgres'. A worker sixty seconds ahead of
the database therefore measured every charge as a minute older than it was,
every live confirm passed the guard, and the whole of lane G was a no-op — on
precisely the deployment whose fleet clocks have drifted, with nothing in the
data looking wrong. `past_the_horizon` carried the same subtraction; its skew
leaned the milder way (escalating a charge to `unresolved` early, waking an
operator).

**The fix is one statement, not one clock discipline.**
`Charges::get_by_id_as_of` selects `now() AS db_now` beside the row and answers
a `ChargeAsOf`; `Charges::get_by_id` is now that read with the clock dropped,
so the two cannot drift on what they select. `recovery_step` takes
**durations** — `charge_age`, `not_found_streak_age` — and no instant at all,
so there is no parameter left for a caller to read off the wrong clock;
`past_the_horizon` takes the same age; and `first_not_found_at` is stamped from
`db_now`, so the `NotFound` streak's own window is on that clock too.

Two statements (`SELECT now()` beside the row read) would not have done: the
gap between them is a scheduling delay, and a scheduling delay is the quantity
being measured.

### 1a. Decisive proof (each mutation applied, measured, restored)

One test, `worker_recovery::a_young_push_charge_is_not_advanced_until_it_is_older_than_the_window`,
run four times on the same containers.

| # | Tree | Result |
|---|---|---|
| 0 | The fix, unmodified | `PASS [ 27.149s] 1 test run: 1 passed` |
| 1 | The **pre-fix** subtraction with the reviewer's skew: `charge_age = (now_utc() + 61s) - created_at` | `FAIL`, `left: Finished  right: Rescheduled(10s)` |
| 2 | The fix intact, **+61 s injected into every host-clock read left in `handlers.rs`** (both `enqueue_in_tx` run_ats and `scan_live_charges`' cutoff) | `PASS [184.704s] 1 test run: 1 passed (1 slow)` |
| 3 | The fix intact, **guard deleted** (`if charge_age < window` → `let _ = charge_age;`) | `FAIL`, `left: Finished  right: Rescheduled(10s)` |

Mutation 1's failure is the reviewer's own reproduction, and it is **the same
line lane G recorded for deleting the guard** — which is the point: a host
clock 61 s fast was indistinguishable from having no guard at all. Mutation 2
is the claim itself: with the age taken from Postgres, a worker an entire
window ahead of the database changes nothing about the decision. Mutation 3
shows the case is still decisive about the guard. All three restored; `git
status` clean afterwards.

Mutation 2 is also where a **second, milder** cross-clock use shows itself, and
it is recorded rather than fixed — see §5.

### 1b. Unit-level proof

- `vpay_worker::handlers::tests::the_age_is_measured_by_the_database_and_not_by_this_host`
  — a row written at the epoch, read by a statement whose `now()` was five
  seconds later, must be five seconds old. The second half is what makes it
  decisive rather than tautological: the age the *host* clock produces for the
  same row is measured too, and asserted to be past the 24-hour horizon, so an
  implementation that reached for `now_utc()` would escalate this charge to
  `unresolved` on its first poll and fail the case.
- `vpay-db`'s `the_charge_read_carries_the_databases_own_clock_beside_the_row`
  (container-backed) — the two reads answer the same row, a charge opened a
  moment ago reads as seconds old, and backdating `created_at` by 61 s moves
  the age across the sixty-second boundary the worker compares against.

---

## 2. F3 — one wait, priced at one rung

Every reschedule is re-claimed, `vpay_db::Jobs::claim` increments
`jobs.attempts`, and `poll_delay` is indexed by that count. `Wait`
rescheduling at `poll_delay(0)` therefore cost **six** claims to cross a
sixty-second window, and a genuinely crashed charge began its real recovery at
`poll_delay(6)` — two minutes a rung, with the fast end of the ladder already
spent on doing nothing.

`Wait` now carries `not_found_window - charge_age`, clamped into
`[0, not_found_window]`, and the handler reschedules once at that plus
`RECOVERY_WAIT_MARGIN` (one second, so the next claim is unambiguously past
the window rather than exactly on its boundary). **The first real recovery
pass is the second claim, and the rung after it is `poll_delay(1)` — twenty
seconds.** The upper clamp is not decoration: a row whose `created_at` leads
the database's `now()` has a negative age, and an unclamped `window - age`
would park a live charge's poll for as long as the skew reaches.

Tests: `a_wait_carries_the_rest_of_the_window_and_not_a_ladder_rung` pins 0 s →
60 s, 59 s → 1 s, 60 s → no wait at all (the table applies), plus an
`assert_ne!` against `poll_delay(0)`; the age table now asserts the exact
remaining time at every age; and both integration cases go through
`assert_waited_out_the_rest_of_the_window`. The push case also reads what was
**committed**: `jobs.attempts` is 1 and `run_at` is 55–61 s out, not 10.

---

## 3. F4 — what the callback route does and does not bound

`provider_callback`'s module doc claimed a hostile caller was "bounded by what
the ladder was going to do anyway". It was not: `pull_forward_in_tx` matched
any unleased future job, so each POST naming a live charge caused one real,
authenticated `query_status` within about a second.

`pull_forward_in_tx` now takes a `floor` and refuses a job already due within
it (`AND run_at > now() + $2`). The caller passes
`vpay_api::provider_callback::PULL_FORWARD_FLOOR` — ten seconds, the ladder's
own fastest rung. The floor is a *parameter* because how often a rail is asked
anything is not `vpay-db`'s policy (ADR-0002), and it is spelled out in
`vpay-api` rather than read from `vpay_worker::poll_delay` because the
dependency runs the other way; `the_pull_forward_floor_is_the_poll_ladders_first_rung`
(integration, where both crates are linked) is the join that fails if they
drift.

**The bound is now stated truthfully, and it is smaller than the review's own
wording suggests.** A charge the queue was about to ask about anyway costs a
caller nothing — that is the common case, because it is where every charge sits
immediately after a poll. But the rungs grow (20 s, 30 s, 45 s …) while the
floor stays at ten, so a charge parked further out is still brought forward by
every callback — which is what the route is *for* — and a caller repeating
against one live charge can still hold it at roughly one status query per
worker claim. **There is no rate limit**, per charge or per source. What stands
between the route and rail traffic is that the caller must know a v4
`provider_reference_id` for a live charge on this deployment, that each
accepted POST buys one authenticated status query which settles the charge the
rail names or nothing at all, that the body is bounded at 16 KiB, and that
nothing here writes charge or intent state. The module doc, the route's doc
comment and `docs/reference/vpay-api.md` all say this now, and all three also
say the route runs **two statements** (`enqueue_in_tx` + `pull_forward_in_tx`),
not "exactly one write".

**The cost, stated where it is paid:** a rail's callback arriving while the
charge sits on the ladder's *first* rung no longer settles it early — it
settles at that rung, up to ten seconds later than before. That is a real
reduction in what lane C shipped, and it is asserted rather than implied
(`a_callback_does_not_accelerate_a_poll_that_is_already_about_to_run`: 202,
`run_at` unchanged, nothing claimable). Lane C's headline case now parks its
job at a later rung — the state a callback is actually for — because the state
it used to stage is exactly the one that is now deliberately refused.

Guard proof, in `vpay-db`'s
`pull_forward_moves_a_job_past_the_floor_and_leaves_near_leased_parked_and_due_alone`:
a job 5 s out is refused **and its `run_at` is unchanged**, a job 30 s out
moves, and the same 5 s job moves when the floor is `Duration::ZERO` — so the
refusal is the floor's doing and not the row's.

---

## 4. F6 — four prefixes the classifier let through

`192.88.99.0/24` (6to4 relay anycast, deprecated and unroutable by RFC 7526 —
the IPv4 half of a mechanism whose IPv6 half `2002::/16` was already refused),
and `2001:1::/32` (IETF protocol assignments), `2001:2::/48` (benchmarking),
`2001:20::/28` (ORCHIDv2) — all three inside global unicast, so the `2000::/3`
test does not reach them, and all three beside `2001::/32` (Teredo) and
`2001:db8::/32`, which the classifier already named.

Each has a row in `every_refused_range_is_classified_in_both_families` — a row
rather than a test of its own, following that case's own reasoning that a
missing range should read as a missing row — including `2001:2f:ffff::1` for
the far end of the `/28`. `192.88.98.255` and `192.88.100.1` went into the
*deliverable* table, so the `/24` is pinned from both sides.

**Left to the maintainer:** these three IPv6 prefixes sit inside `2001::/23`,
the block RFC 2928 gave IANA for IETF protocol assignments as a whole.
Refusing the entire `/23` would be the broader and arguably more correct fix —
nothing in it is a place a merchant's receiver lives — but it is a wider call
than the review asked for, so it was not taken. It is why no address inside
`2001::/23` appears in the "ordinary public addresses" table: asserting one is
deliverable would be a claim about IANA's unassigned space that this lane is
not in a position to make.

---

## 5. Recorded, not fixed

### F5 — an SSRF-refused delivery is destroyed on its first attempt

An egress refusal is a **permanent** failure: `webhook_deliveries.state =
'exhausted'` on attempt 1, no next attempt, and replay is unbuilt. A
transiently poisoned DNS answer — or a receiver behind a resolver that briefly
returns a private address — therefore destroys the event permanently, and
there is no way to re-drive it. Lane B did exactly what the plan asked, and the
plan asked for fail-closed; the gap is that "fail closed" and "destroy the
event" are the same thing while replay does not exist.

Not fixed here: the remedy is a replay path (or an `ssrf_blocked` state that is
retryable a bounded number of times), which is a design decision about a
merchant-visible delivery state machine and belongs with whoever owns
`docs/flows/webhooks.md`. **Recommendation:** treat the *resolution* half the
way an unresolvable host is already treated — an ordinary failed attempt on
`delivery_delay` — and keep the permanent refusal for an address that
classifies as private on every attempt of the ladder. That distinction is
already made once in this code (`a_host_that_resolves_to_a_private_address_is_refused_and_an_unresolvable_one_retries`),
which is why it is worth naming rather than inventing.

### F7 — the 202s are not indistinguishable in *time*

`POST /provider/{code}/callback` answers `202` for a reference it has a charge
for and `202` for one it does not, deliberately, so the route is not an oracle.
The two are not the same *duration*: the unknown-reference path returns after
one indexed `SELECT`, and the known-reference path additionally opens a
transaction and runs two statements. That is a timing oracle for "does this
deployment hold a charge with this rail reference".

Not fixed, and not obviously worth fixing: it only matters to someone who has
already guessed a v4 UUID, which is the same thing they would need to exploit
the answer. Recorded so that nobody discovers it later and assumes the uniform
status code was believed to be a complete answer.

### Other cross-clock arithmetic left alone (in scope only by adjacency)

Mutation 2 of §1a made the young-push case take 184 s instead of 27 s, and the
reason is worth writing down: `handlers.rs` still reads the **host** clock in
three places, all of them scheduling rather than deciding.

- Two `enqueue_in_tx(..., OffsetDateTime::now_utc())` calls write `jobs.run_at`
  from the host clock. A worker running fast schedules its own next job late by
  the skew — which is what the 184 s was — but no decision is made from it, and
  `vpay-api`'s confirm path does the same thing on the same column.
- `scan_live_charges` computes `cutoff = now_utc() - 10 min` and compares it
  against `charges.updated_at`, which Postgres wrote. This *is* the same
  cross-clock defect as F1, in the mildest possible direction: a fast worker
  considers a charge unattended sooner, and every re-enqueue it produces is
  `ON CONFLICT DO NOTHING`. It was left alone because it is outside the four
  findings and because fixing it properly means the scan's query taking the
  cutoff as an interval rather than an instant — a `vpay-db` signature change
  with its own test, which is a change the reviewer did not ask for.

---

## 6. `docs/status.md` — amendments for lane E (verbatim)

### 6a. The confirm/worker race row (`docs/status.md:1210`)

**Replace** the sentence

> **The fix is a minimum charge age**: `recovery_step` answers `RecoveryAction::Wait` — reschedule on the ladder's first rung, write nothing, ask nothing — for any `submitting` charge younger than `RecoveryPolicy::not_found_window` (60 s, three times the 20 s rail request timeout), measured from `charges.created_at` because the `SubmitAttempt::Never` branch has no `provider_requests` row to measure from. One predicate in the pure function (`vpay-worker/src/recovery.rs:325`), reached by both callers through the `recovery_action` helper they already shared.

with

> **The fix is a minimum charge age**: `recovery_step` answers `RecoveryAction::Wait` — write nothing, ask nothing, come back once when the charge is old enough — for any `submitting` charge younger than `RecoveryPolicy::not_found_window` (60 s, three times the 20 s rail request timeout), measured from `charges.created_at` because the `SubmitAttempt::Never` branch has no `provider_requests` row to measure from. One predicate in the pure function, reached by both callers through the `recovery_action` helper they already shared. **Two defects in that first shape were found by Step 8's own correctness review and fixed the same day (lane H).** The age was `OffsetDateTime::now_utc() - charges.created_at` — the worker *host's* clock minus Postgres' — so a worker sixty seconds fast measured every charge as a minute older than it was and the guard became a silent no-op, exactly on the deployment whose fleet clocks had drifted; injecting `+61 s` at the old site failed `a_young_push_charge_is_not_advanced_until_it_is_older_than_the_window` with the identical line deleting the guard produces (`left: Finished  right: Rescheduled(10s)`). The age now comes from `Charges::get_by_id_as_of`, which selects `now()` on the same statement that reads the row, and `recovery_step` takes durations rather than instants so no caller can supply the wrong clock; with the fix, +61 s of host skew injected into every remaining host-clock read in `handlers.rs` leaves that case passing. `past_the_horizon` took the same subtraction and now takes the same age. And `Wait` rescheduled at `poll_delay(0)`: every reschedule is re-claimed, `Jobs::claim` increments `attempts`, and `poll_delay` is indexed by it, so a genuinely crashed charge burned six rungs waiting the window out and started its real recovery at `poll_delay(6)`. `Wait` now carries `window - age` (clamped into `[0, window]`) and reschedules **once**, so the wait costs one claim and the first real rung after a crash is `poll_delay(1)`, twenty seconds.

**And replace** the cost sentence

> **The cost is stated:** a charge orphaned by a genuine crash waits up to a minute for its first recovery pass; it stays live and queued throughout, and 60 s is not the 24-hour horizon.

with

> **The cost is stated:** a charge orphaned by a genuine crash waits up to a minute for its first recovery pass, and that wait costs it one rung of the poll ladder rather than six; it stays live and queued throughout, and 60 s is not the 24-hour horizon.

### 6b. The rail callback route row (`docs/status.md:1208`)

**Replace**

> and performs exactly one write — `TxRepositories::pull_forward_in_tx`, an `UPDATE jobs SET run_at = now()` on that charge's existing `poll:<charge id>` job, refusing a leased, already-due or dead-lettered one.

with

> and runs **two statements in one transaction** — `enqueue_in_tx` (`ON CONFLICT DO NOTHING`, so nothing in the ordinary case) and `TxRepositories::pull_forward_in_tx`, an `UPDATE jobs SET run_at = now()` on that charge's existing `poll:<charge id>` job, refusing a leased, dead-lettered, already-due **or already-due-within-ten-seconds** one.

**Replace** the headline-proof sentence

> including the headline one: the worker's first poll parks the job ten seconds out, `run_once` then finds **nothing runnable**, the rail's documented body is POSTed to the URL read back off the rail's own WireMock journal, and the next `run_once` settles the charge.

with

> 11 since Step 8's review, including the headline one: the poll job is parked at a later rung, `run_once` then finds **nothing runnable**, the rail's documented body is POSTed to the URL read back off the rail's own WireMock journal, and the next `run_once` settles the charge — and its sibling `a_callback_does_not_accelerate_a_poll_that_is_already_about_to_run`, which is the new bound: a poll already due within `PULL_FORWARD_FLOOR` (ten seconds, the ladder's fastest rung) is left exactly where the ladder put it, and the rail is still answered `202`.

**Replace** the whole final passage beginning "**What it also does, and nothing
said until 2026-09-04 (Step 8 review, finding 4)**" — up to the end of the row
— with

> **What it also does, and nothing said until 2026-09-04 (Step 8 review, finding 4): it is the first unauthenticated route this repo publishes to a network** — `compose.demo.yml` maps the whole `vpay-server` port to the host (`${VPAY_DEMO_PORT:-8080}:8080`, bound on `0.0.0.0`), so anyone on the same LAN as a demo or e2e stack can post a rail notification at it without a credential. **What that buys them was overstated and is now measured.** The module used to claim it was "bounded by what the ladder was going to do anyway"; it was not, and since 2026-09-04 (lane H) the pull-forward refuses a job due within the ladder's own fastest rung, so a POST about a charge the queue was about to ask about anyway changes no row and causes no rail request at all. Past that rung it is **not** bounded: the ladder's rungs grow (20 s, 30 s, 45 s …) while the floor stays at ten, so a caller repeating against one live charge can hold it at roughly one authenticated `query_status` per worker claim. **There is no rate limit, per charge or per source.** What is left standing is that the caller must know a v4 `provider_reference_id` for a live charge *on this deployment*, that each accepted POST buys exactly one authenticated status query — which settles the charge the rail actually names or nothing at all — that the body is bounded at 16 KiB, and that no charge or intent state is ever written ([flows/provider-port.md](../../flows/provider-port.md): the route is a hint that never moves state). It remains unauthenticated write access to a job's `run_at`, and **a real deployment must front this path** (rate limit, IP allowlist, or a reverse proxy) rather than publish it as the demo does. The floor also has a cost, stated: a rail calling back while the charge sits on the ladder's first rung no longer settles it early — it settles at that rung, up to ten seconds later than before

### 6c. The egress-guard row (`docs/status.md:1229`)

**Replace** the two occurrences of the phrase

> the IANA special-purpose IPv4 blocks, every IPv6 address outside global unicast, the 6to4/Teredo/documentation prefixes inside it

with

> the IANA special-purpose IPv4 blocks (including `192.88.99.0/24`, the 6to4 relay anycast RFC 7526 deprecated), every IPv6 address outside global unicast, and the special-purpose prefixes inside it — 6to4 `2002::/16`, Teredo `2001::/32`, IETF protocol assignments `2001:1::/32`, benchmarking `2001:2::/48`, ORCHIDv2 `2001:20::/28` and documentation `2001:db8::/32`

and change "Proven by 9 unit cases over every range in both families" to
"Proven by 9 unit cases over every range in both families (the table grew six
rows on 2026-09-04 for the four prefixes Step 8's review found missing —
finding 6)".

---

## 7. `docs/flows/*.md` sentences for lane E

### `docs/flows/crash-safety.md`

Lane G's §7 asked for a table row reading

> \| Charge younger than 60 s | A confirm may still be running | **Wait.** Reschedule on the poll ladder's first rung and touch nothing \|

Whether or not it has landed yet, the right text is now

> \| Charge younger than 60 s | A confirm may still be running | **Wait.** Reschedule once, for the rest of the window, and touch nothing \|

and the paragraph beneath it gains a sentence:

> The wait costs the charge one rung of the ladder and not six: `RecoveryAction::Wait` carries `not_found_window - age` and the poll comes back once, when the age guard will pass, so a genuinely crashed charge starts its real recovery at `poll_delay(1)` — twenty seconds — rather than at `poll_delay(6)`. The age itself is measured by Postgres at both ends (`Charges::get_by_id_as_of` selects `now()` beside the row), because a window computed from the worker host's clock is a window a fast host does not have.

### `docs/flows/reconciler.md`

Lane C's §5 asked for a "What is built" bullet reading

> - **The callback endpoint exists.** … It never changes state: it enqueues the charge's `poll:<charge id>` job if it is missing and brings it forward to `now()` if it is parked at a rung, refusing a leased or dead-lettered one. …

The accurate form is

> - **The callback endpoint exists.** `POST /provider/{code}/callback` (`vpay_api::provider_callback`) is the route the section above describes, built 2026-09-04. It never changes state: it enqueues the charge's `poll:<charge id>` job if it is missing and brings it forward to `now()` if it is parked **further out than the ladder's first rung**, refusing a leased or dead-lettered one, and refusing one already due within that rung so that an unauthenticated caller cannot spend a rail request on a poll the queue was about to make anyway. The `dedupe_key` really is what stops duplicate callbacks becoming a job storm, and it is now that on a live path rather than in a design. What it does **not** do is bound a caller who repeats against a charge parked further out; there is no rate limit, and [reference/vpay-api.md](../../reference/vpay-api.md) states the true bound.

### `docs/flows/webhooks.md` (line 248)

**Retire:**

> global unicast `2000::/3`, the 6to4/Teredo/documentation prefixes inside it,

**Replace with:**

> global unicast `2000::/3`, the special-purpose prefixes inside it — 6to4
> `2002::/16`, Teredo `2001::/32`, IETF protocol assignments `2001:1::/32`,
> benchmarking `2001:2::/48`, ORCHIDv2 `2001:20::/28` and documentation
> `2001:db8::/32` —

and, in the IPv4 list on the line above, `240.0.0.0/4`, the IANA
special-purpose IPv4 blocks → `240.0.0.0/4`, the IANA special-purpose IPv4
blocks (including the 6to4 relay anycast `192.88.99.0/24`).

---

## 8. Verification

Run in `/home/selast/dev/vpay/.claude/worktrees/step8-review-r1` with
`DOCKER_HOST=unix:///run/user/1000/docker.sock` and `CARGO_BUILD_JOBS=4`.

| Command | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo nextest run -p vpay-worker -p vpay-db -p vpay-api` | **371 tests run: 371 passed, 0 skipped** (was 368: +1 `vpay-worker` age case, +1 `vpay-worker` wait case, +1 `vpay-db` clock case) |
| `cargo nextest run -p vpay-tests-integration -E 'binary(worker_recovery) \| binary(worker_kill9) \| binary(provider_callback) \| binary(confirm_rails)'` | see §8a |
| `just verify` | see §8a |
| `just verify-ignored` | see §8a |
| `just test-doc` | see §8a |

### 8a. Counts, from the final run on this branch

- `cargo fmt --all --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo nextest run -p vpay-worker -p vpay-db -p vpay-api --no-fail-fast
  --retries 2` — **371 tests run: 371 passed, 0 skipped, 0 ignored** (1 slow,
  **no flakes**). `vpay-worker` 75, `vpay-db` 82, `vpay-api` 214.
- `cargo nextest run -p vpay-tests-integration -E 'binary(worker_recovery) |
  binary(worker_kill9) | binary(provider_callback) | binary(confirm_rails)'
  --no-fail-fast --retries 2 -j 1` — **43 tests run: 43 passed, 0 skipped**
  (5 slow, **no flakes**, 810 s). `worker_recovery` 23, `provider_callback`
  11, `confirm_rails` 7, `worker_kill9` 2.
- `just verify` — `verify-no-mocks` ok; `verify-status` ok (1 unimplemented
  item, `mtn_momo::refund`, unchanged); `verify-errors` ok (15 error types,
  all classified — this lane added none); `verify-sdk-parity` ok (267 proving
  tests, 24 dated gaps). `verify-docs` is advisory: `poll_charge` is on its
  long-function list at 115 lines, which is three lines longer than before
  (the `ChargeAsOf` destructuring and the comment that says why the clock is
  Postgres'), and `vpay_worker::recovery` is on the prose-ratio list at 292.9%.
- `just verify-ignored` — **0 ignored (expected 0), 41 test binaries
  (expected 41), 1059 total (minimum 1000)**, up from 1054. No new binary, so
  neither counter in the `justfile` moves; the comment above them records the
  new measurement and why, in this lane's commit.
- `just test-doc` — **77 doctests passed, 0 failed, 1 ignored** (the ignored
  one is `sdks/rust`'s README, pre-existing). `recovery_step`'s own doctest
  moved to durations with the function and still runs.

**Flakes seen while getting there, all the same one.** Three earlier runs of
individual cases hit `postgres:16-alpine container starts … failed to create a
container: Timeout error` on TRY 1 and passed on TRY 2 — the infrastructure
flake lanes A, C and G all recorded on this host, which had 30-odd containers
up from sibling lanes throughout. The final gate runs above consumed **no**
retries.

---

## 9. What this lane did **not** do

- **`docs/status.md`, `docs/roadmap.md` and `docs/flows/*.md` are untouched.**
  §6 and §7 are lane E's input; another agent holds those files on the gate
  branch. `docs/reference/vpay-worker.md`, `vpay-db.md` and `vpay-api.md` **are**
  edited here, since they are the reference pages of the crates this lane
  changed.
- **F5 and F7 are not fixed** — §5 says what each is and what the fix would
  have to decide.
- **No rate limit was added** to the callback route, and the floor is not one.
  §3 says exactly what it does and does not bound.
- **`2001::/23` as a whole is still deliverable** — §4 says why that was left
  to the maintainer.
- **The remaining host-clock reads in `handlers.rs` are unchanged**, including
  `scan_live_charges`' cutoff, which is the same defect class as F1 in its
  mildest direction (§5).
- **No real rail has called anything**, and no `just demo` was run here.
