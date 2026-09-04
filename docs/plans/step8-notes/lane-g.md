<!-- Per-lane notes for Step 8. Lane E (the orchestrator) edits docs/status.md, docs/roadmap.md and docs/flows/*.md from these, so the lanes never fight over one table. This file is history once Step 8 lands. -->

# Step 8, lane G — the confirm/worker recovery race

Branch `claude/step8-lane-g-race`, on `93c6a1c` (master `572a89f` + the Step 8
plan). This lane exists because **lane A's demo found a real defect in vpay**
and deliberately did not fix it: `docs/plans/step8-notes/lane-a.md` §3, four
failed walkthroughs in six on a loaded machine, every one a `500`
`write_matched_no_row` on a confirm.

## 1. The decision this lane implements

Recovery of a `submitting` charge is legal **only once the charge is older than
`RecoveryPolicy::not_found_window`** (60 s, against a rail request timeout of
20 s — `vpay_provider::DEFAULT_REQUEST_TIMEOUT`). Younger than that, every
branch of the recovery table returns a short reschedule and touches nothing.

This was the maintainer's choice between lane A's three candidates (a minimum
charge age, a delayed first rung, a confirm-held lease); this lane did not
reopen it. The other two are argued against in
`docs/reference/vpay-worker.md` §"Nothing younger than the window is
recovered".

## 2. What landed

| Change | Where |
|---|---|
| `RecoveryAction::Wait` — the only action that touches neither the charge nor the rail | `backends/crates/vpay-worker/src/recovery.rs:139-147` |
| The guard itself: one predicate, above the flow-shape branch, so `Answered`→`Advance`, `Redirect`→`FailDeadOrder`, `Never`→`Resubmit` and `Unanswered`→`Poll` are all gated by it | `recovery.rs:324-326` |
| `recovery_step` gained a `charge_created_at` parameter (7th) | `recovery.rs:306-316` |
| `window()`, one `Duration` conversion for the guard and the `NotFound` streak, so they cannot drift on saturation | `recovery.rs:364` |
| Both callers reach it through the **one** existing shared helper, which now passes `charges.created_at` | `handlers.rs:309-327` (`recovery_action`) |
| `Wait` at the top-of-poll block: reschedule at `poll_delay(0)`, no rail call, no write | `handlers.rs:407-417` (`act_on_recovery`) |
| `Wait` in `recover`, joined to the `Poll` arm and documented as unreachable (same `now`, same `created_at` as the block that already returned) | `handlers.rs:601` |

**The clock is `charges.created_at`, not the first `provider_requests.sent_at`,**
and the argument is in the `recovery_step` doc comment: the branch the guard
most has to protect is `SubmitAttempt::Never`, where there is *no* attempt row,
so a clock read from `provider_requests` is `None` exactly when it is needed.
`created_at` is written by Postgres' `now()` inside the confirm's first
transaction — before any network call by construction (`vpay_db::NewCharge`) —
so it dates the window from the moment the race opens, and it is the column
`past_the_horizon` already measures the 24-hour escalation from.

**The cost, stated:** a charge orphaned by a genuine crash waits up to a minute
for its first recovery pass. It stays live and queued throughout. The deferral
cannot swallow the 24-hour escalation, because a charge younger than 60 s is
not a day old.

## 3. Tests

**Unit** (`recovery.rs`, `cargo nextest run -p vpay-worker`: **64 tests run, 64
passed, 0 skipped**, up from 60):

- `nothing_younger_than_the_window_is_recovered` — the age table: five branches
  (`Never`→`Resubmit`, `Unanswered`→`Poll`, `Unanswered`+streak→`Resubmit`,
  `Answered`→`Advance`, `Redirect`→`FailDeadOrder`) × four ages (0 s, 59 s,
  **60 s**, 61 s). The boundary is `>=`, matching the streak window's own
  comparison.
- `a_young_charge_is_left_alone_on_every_shape_of_evidence` — every
  (flow × evidence × streak) combination at one second old is `Wait`. This is
  what a fix that guarded only one branch would fail.
- `a_tightened_window_shortens_the_age_guard_too` — the guard moves with the
  policy, which is what lets an integration test cross it without sleeping.
- `a_charge_created_in_the_future_is_not_recovered` — clock skew saturates
  toward doing nothing.
- The `recovery_step` doctest gained the two young cases (push and redirect).

**Integration** (`worker_recovery.rs`):

- `a_young_push_charge_is_not_advanced_until_it_is_older_than_the_window`
  (`:734`) — kill point 3's fixture, *unaged*: the job is
  `Rescheduled(poll_delay(0))`, the charge is still `submitting`, the intent
  still `requires_payment_method`, `status_queries` is 0, no event, no
  resubmit job. Then aged 90 s and re-run: advanced and settled `succeeded`.
- `a_young_redirect_charge_is_not_failed_as_a_dead_order_until_it_is_older`
  (`:842`) — the same shape on Orange, where the branch that must not fire is
  `FailDeadOrder`: young → untouched, `failure_code` NULL, no
  `payment_intent.payment_failed` event; aged → `failed`,
  `provider_unavailable`, exactly as before.
- `a_confirms_compare_and_swap_wins_against_the_worker_that_claimed_its_poll_job`
  (`:949`) — **the race, run.** The shipping `vpay_worker::run_loop`, one
  worker, the poll job at `run_at = now()` as `insert_charge` writes it; the
  test waits until the loop has claimed *and settled* that job
  (`wait_for_the_poll_job_to_run`, `:2297` — polls `jobs.attempts`/`locked_by`
  rather than sleeping, so the ordering is guaranteed, not hoped for), and only
  then performs the confirm's own compare-and-swap,
  `TxRepositories::mark_submitted` with a redirect confirm's arguments. The
  final state must be the confirm's.

  **No seam was needed**, and one thing is staged: the HTTP confirm *handler*
  is not in this suite (it has no router; `confirm_rails.rs` owns that half),
  so what runs is the compare-and-swap `persist_submitted` makes, at the instant
  the race occurs. Everything else — claim, recovery decision, reschedule — is
  the shipping loop.

## 4. Revert proof

Guard deleted (`recovery.rs:324-326` replaced by `let _ = charge_created_at;`),
nothing else changed, same containers:

```
FAIL [ 104.479s] a_young_push_charge_is_not_advanced_until_it_is_older_than_the_window
  assertion `left == right` failed: a charge too young to recover comes back on the ladder's first rung
    left: Finished          right: Rescheduled(10s)

FAIL [ 123.833s] a_young_redirect_charge_is_not_failed_as_a_dead_order_until_it_is_older
  assertion `left == right` failed: a redirect charge too young to recover comes back on the ladder's first rung
    left: Finished          right: Rescheduled(10s)

FAIL [   8.982s] a_confirms_compare_and_swap_wins_against_the_worker_that_claimed_its_poll_job
  Error: the confirm's `submitting` -> `submitted` compare-and-swap matched no row:
         the worker moved the charge first. ...
  Caused by: no row in charges matched ch_m8mfr8mgp957k4sta43ykv22, or it was no
             longer in the required state
```

That last line is lane A's `500` verbatim — the same sentence the demo's
merchant received (`lane-a.md` §3). Guard restored; all three pass again.

## 4b. Gate output, on `93125f0`

```
cargo fmt --all --check                                          clean
cargo clippy -p vpay-worker -p vpay-api --all-targets -D warnings clean
  (and -p vpay-tests-integration, which carries the new cases)   clean
cargo nextest run -p vpay-worker            64 tests run: 64 passed, 0 skipped
cargo test --doc -p vpay-worker              5 tests run:  5 passed, 0 ignored
just test-doc (workspace)                   all suites ok; 1 ignored doctest,
                                            pre-existing (sdks/rust README)
just verify                                 ok (3 gates; verify-docs advisory)
just verify-ignored                         0 ignored (expected 0), 39 binaries
                                            (expected 39), 1006 total (min 950)
cargo nextest run -p vpay-tests-integration \
  -E 'binary(worker_recovery) | binary(confirm_rails) | binary(worker_e2e)' \
  --no-fail-fast --retries 2                33 tests run: 33 passed, 0 skipped
                                            (183 s, no retries consumed)
```

The same three-binary gate was run three times in all, on a machine running
three sibling lanes (load average 9–16, 33 containers up). The other two runs
were **33/33 with one flaky** (`three_not_founds_over_the_window_resubmit_and_two_do_not`,
`TRY 1` = "the Orange stub container starts / failed to create a container:
Timeout error") and **31/33 with two failures**
(`confirm_rails::a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read`
and `a_push_confirm_the_rail_accepts_moves_the_intent_to_processing`, both at
120.007 s — testcontainers' container-create timeout, three tries each). Both
of those re-ran green in isolation immediately afterwards (11.3 s and 113.6 s),
and neither touches the worker. **It is the same infrastructure flake lane A
recorded** (`lane-a.md` §2, `Client(CreateContainer(RequestTimeoutError))`), not
a regression — but it is written down rather than left as "green on the third
try".

## 5. Existing tests: what changed and why

**No existing test was weakened, and none encoded immediate recovery of a fresh
`submitting` charge as a property.** What they encoded is the recovery *table*,
staged by `support::crashed_charge`, which inserts the charge milliseconds
before the assertion — and a charge that young is now indistinguishable from a
live confirm, which is the whole point of the guard. That is a **fixture**
problem, so the fixtures were aged:

- New: `support::age_the_crash(pool, charge_id, age)` and
  `support::RECOVERABLE_CRASH_AGE` (90 s) —
  `backends/tests/integration/tests/support/mod.rs:508-541`. It moves
  `charges.created_at`, the same lever `age_past_the_horizon` already pulls for
  the 24-hour escalation, on the same column, leaving `RecoveryPolicy` exactly
  as a deployment has it.
- Called immediately after every `crashed_charge` in `worker_recovery.rs` — 16
  call sites across these cases:
  `a_charge_whose_submit_never_left_is_resubmitted_under_the_same_reference`,
  `a_submit_whose_answer_was_lost_is_resolved_by_asking_the_rail`,
  `an_answered_submit_advances_the_bookkeeping_rather_than_submitting_again`,
  `three_not_founds_over_the_window_resubmit_and_two_do_not`,
  `a_charge_past_the_horizon_is_unresolved_polled_hourly_and_alerted_never_parked`,
  `a_redirect_charge_with_no_token_is_failed_without_ever_asking_the_rail`,
  `a_settlement_lands_on_the_intent_a_crashed_confirm_left_behind`,
  `a_rail_that_never_answers_is_still_escalated_at_the_horizon`,
  `a_late_success_past_the_horizon_still_settles`,
  `a_resubmit_past_the_horizon_still_escalates`,
  `a_never_submitted_charge_past_the_horizon_escalates_after_enqueuing_the_resubmit`,
  `a_poisoned_job_past_the_horizon_is_parked_rather_than_rescheduled_hourly`,
  `a_second_hourly_poll_of_an_unresolved_charge_re_alerts_without_writing_it_again`,
  `a_decline_past_the_horizon_settles_an_unresolved_charge_and_clears_the_alert`,
  `the_backstop_scan_re_enqueues_an_unattended_charge_and_leaves_attended_ones_alone`
  (both charges), plus the shared helper `charge_that_settles_in_one_poll`
  (which gained a `pool` parameter and serves the two lease-reaping cases).
- **That the ageing is load-bearing rather than cosmetic is measured, not
  argued:** `a_young_push_charge_is_not_advanced_until_it_is_older_than_the_window`
  stages `an_answered_submit_advances_…`'s fixture *without* the ageing and
  asserts the charge is **not** advanced.
- `worker_e2e.rs`'s single `crashed_charge` call is untouched: it moves the
  charge to `submitted` immediately, so the recovery block never applies to it.

## 6. Status row for lane E (verbatim)

**Replace** lane A's drafted ⛔ row (`lane-a.md` §6, "Confirm vs. the worker's
first poll") with:

> \| Confirm vs. the worker's first poll (`write_matched_no_row`) \| ✅ \| **Found 2026-09-04 by Step 8's demo, fixed the same day (lane G).** `insert_charge` commits the `submitting` charge and its `poll_charge` job in one transaction with `run_at = now()` (`vpay-api/src/v1/payment_intents.rs:1505`), and the worker may claim that job before the confirm finishes its own `submitting → submitted` compare-and-swap (`vpay-db/src/charges.rs:265`; `IDLE_SLEEP` is 1 s, `vpay-worker/src/run_loop.rs:52`). It then applied the **crash-recovery** table to a charge whose process had not crashed, and either branch moved the charge out from under the confirm — observed four times in six walkthrough runs on a loaded machine (confirm latency 3.7 s): on a push rail the merchant was told the confirm failed and was then delivered a `payment_intent.succeeded` webhook; on a redirect rail `FailDeadOrder` killed a live order as `provider_unavailable` while the confirm held the very redirect URL its `failure_raw` said the payer had never been given. **The fix is a minimum charge age**: `recovery_step` answers `RecoveryAction::Wait` — reschedule on the ladder's first rung, write nothing, ask nothing — for any `submitting` charge younger than `RecoveryPolicy::not_found_window` (60 s, three times the 20 s rail request timeout), measured from `charges.created_at` because the `SubmitAttempt::Never` branch has no `provider_requests` row to measure from. One predicate in the pure function (`vpay-worker/src/recovery.rs:324`), reached by both callers through the `recovery_action` helper they already shared. **The cost is stated:** a charge orphaned by a genuine crash waits up to a minute for its first recovery pass; it stays live and queued throughout, and 60 s is not the 24-hour horizon. Proven by a unit table over all five branches at four ages, and by three cases in `worker_recovery.rs` — the young push charge is not advanced and the aged one is, the young redirect charge is not dead-lettered and the aged one is, and `a_confirms_compare_and_swap_wins_against_the_worker_that_claimed_its_poll_job` runs the shipping `run_loop` against the confirm's own `mark_submitted` and asserts the confirm wins. Deleting the guard fails all three, the last with the merchant's own error text. What is **not** proven end-to-end: the HTTP confirm handler is not in that suite, so the racing write is `persist_submitted`'s compare-and-swap called directly; and `just demo` has not been re-run against the fix (lane A's harness, not this lane's) \|

**Also change** the "Local demo" row's sentence *"Still 🟡, and for a new
reason: `just demo` from nothing has never been observed green"* — the defect
behind those four failures is fixed, but **the demo has not been re-run here**,
so the row must not claim a green walkthrough. Suggested addition:

> **Updated 2026-09-04 (lane G): the defect behind those four failures is fixed** — see the confirm/worker race row. `just demo` has **not** been re-run since, so this row still records two greens in six and not a green from nothing.

## 7. `docs/flows/crash-safety.md` sentences for lane E

**Add** a row to the "Recovering a `submitting` charge" table (§ after line 48),
directly beneath it, plus the paragraph:

> \| Charge younger than 60 s | A confirm may still be running | **Wait.** Reschedule on the poll ladder's first rung and touch nothing \|
>
> **The table applies only to a charge that has been `submitting` for at least
> `not_found_window` (60 s).** That state is not only what a crash leaves: it
> is also the ordinary state of a confirm that is still inside its rail call,
> because the charge and its poll job are committed *before* the network call
> and the `submitting → submitted` compare-and-swap happens after it. Younger
> than the window, nothing on disk distinguishes the two, and every row above
> would move a charge out from under a live confirm — which is what Step 8's
> demo observed four times in six runs. The age is read from
> `charges.created_at`, because the first row of the table has no
> `provider_requests` row to read a time from at all.

**Amend** the bullet "**The table itself** is
`vpay_worker::recovery::recovery_step`, a pure function over (flow shape,
latest submit attempt, `NotFound` streak, window)" (line 157) to read "… over
(flow shape, latest submit attempt, `NotFound` streak, **charge age**,
window)".

**Amend** "**The flow shape decides first.**" (line 161) to "**The charge's age
decides first, then the flow shape.**", and append to that bullet:

> The redirect branch is unconditional *within* the table but no longer
> unconditional overall: a redirect charge younger than the window is left
> alone, because `FailDeadOrder` is correct only for a submit response that was
> genuinely lost and catastrophic for one that is about to arrive
> (`a_young_redirect_charge_is_not_failed_as_a_dead_order_until_it_is_older`).

**Amend** the Tests section: after "…then runs the real handler against a real
WireMock rail and asserts the recovery table resolves it", add:

> Each of those three states is written **ninety seconds in the past**
> (`support::age_the_crash`), which is not a convenience: a charge inserted a
> millisecond ago is indistinguishable from a confirm that is still running, and
> the recovery table now refuses it. The suite's fourth and fifth cases are that
> refusal — the same fixtures unaged, asserting nothing moves — and its sixth
> runs the shipping loop against the confirm's own compare-and-swap.

## 8. Not done

- **`just demo` was not re-run.** The fix addresses the mechanism lane A
  diagnosed and the reproduction fails without it, but nobody has watched six
  outcomes go green from nothing since. Lane A owns that harness.
- **`docs/status.md` and `docs/flows/*.md` are untouched by this lane**, per the
  plan's "edited only by the orchestrator (lane E) from per-lane notes". §6 and
  §7 are the verbatim text. `docs/reference/vpay-worker.md` **is** edited here,
  since it is this crate's own reference and no other lane holds it.
- **No API-side change.** `insert_charge` still enqueues at `run_at = now()`;
  the decision was to gate the recovery, not to delay the job. A confirm-held
  lease (lane A's option 3) remains the more correct fix and is not implemented.
- **The `Wait` arm in `handlers::recover` is unreachable** and is documented as
  such rather than tested: the same `now` and the same `created_at` are used by
  the block that already returned before the rail was asked.
