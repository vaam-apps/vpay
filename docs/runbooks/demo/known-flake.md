# The demo — §9. The known flake: a real defect the demo found

_Moved out of [docs/runbooks/demo.md](../demo.md) on 2026-09-11 by exp57, which split a 1 426-line runbook into the procedure and its steps. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a `§` cross-reference pointed at a section that is now on another page, the page it moved to._

## 9. The known flake: a real defect the demo found

**`just demo` from nothing did not go green on the authoring machine.** Six
walkthrough attempts on 2026-09-03/04: two green (six outcomes for six, exit
`0` — one is pasted in §4), four failed, always the same way:

```console
✘ orange_money · the payer completes the hosted page — confirm: confirming the payment intent: vpay API error (500): api_error — An internal error occurred. Contact support with the request id.
```

```json
{"level":"ERROR","fields":{"message":"api error","alert":true,"category":"Internal","code":"write_matched_no_row","error":"no row in charges matched ch_pk69syzy2x16s9f0wmpvx8gg, or it was no longer in the required state"}}
```

**This is not the demo's bug.** It is a race between `vpay-api`'s confirm and
`vpay-worker`'s poll job, and the demo is what exposed it:

1. `insert_charge` commits the charge in `submitting` **and** its `poll_charge`
   job in one transaction, with `run_at = OffsetDateTime::now_utc()` —
   immediately runnable (`backends/crates/vpay-api/src/v1/payment_intents.rs:1368`).
2. The confirm then calls the rail and finally CASes the charge
   `submitting` → `submitted` (`charges::mark_submitted`, `vpay-db/src/charges.rs:463`,
   `WHERE id = $1 AND state = 'submitting'`).
3. The worker is entitled to claim that job at once — `IDLE_SLEEP` is **1 s**
   (`vpay-worker/src/run_loop.rs:69`), and zero if it is already busy. It finds
   a charge in `submitting` and applies the crash-recovery table, whose
   precondition is "the process died". Nothing distinguishes _that_ from a
   confirm still in flight.
4. Whichever branch it takes moves the charge, so the confirm's CAS matches no
   row and the merchant gets a `500` — with `alert: true`, so it pages.

The window is the confirm's rail call plus two commits. Normally tens of
milliseconds; **measured at 3.7 s** on a loaded machine, which is where four of
six runs lost it.

Two distinct bad outcomes were observed in the database, and the second is the
serious one:

| Rail              | Branch                                                                                                                                         | What the merchant got | What the database holds                                                                                                                                                                                                                                              |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| MTN (push)        | `RecoveryAction::Advance` — "the rail answered and the state update was lost"                                                                  | `500`                 | intent `succeeded`, and a `payment_intent.succeeded` webhook **was delivered**                                                                                                                                                                                       |
| Orange (redirect) | `RecoveryAction::FailDeadOrder` (`vpay-worker/src/recovery.rs:179`, taken **unconditionally** for `ProviderFlow::Redirect`, with no age check) | `500`                 | charge `failed`, `failure_code = provider_unavailable`, `failure_raw` = _"the rail's submit response was lost before its token could be committed; the payer was never handed a redirect URL…"_ — **while the confirm was in flight and holding exactly that token** |

So on a push rail a merchant is told the confirm failed and is then sent a
`succeeded` webhook; on a redirect rail a **live order is killed** and
mis-labelled `provider_unavailable`, having never been unreachable.

The `Never` branch of that same recovery table already guards against exactly
this class of mistake, with a 60-second `not_found_window` whose comment says a
count alone "would look identical to [a rail] that never got it". The
`Answered` and `Redirect` branches have no equivalent minimum age.

~~**Not fixed in this step.**~~ **Fixed 2026-09-04, later the same day (Step 8,
lane G).** The three candidates were a minimum charge age before the recovery
table applies, a first-rung delay on the poll job, and a `submitting` lease the
confirm holds; the first was taken. `recovery_step` now answers
`RecoveryAction::Wait` — reschedule on the ladder's first rung, write nothing,
ask nothing — for any `submitting` charge younger than
`RecoveryPolicy::not_found_window` (60 s), measured from `charges.created_at`.
Deleting that one predicate reproduces the failure above, including the
merchant's own error text. ~~**After the fix, `just demo` from nothing ran
green: six outcomes for six, zero `write_matched_no_row`** — measured on lane
A's rebased branch, which carried the fix.~~ **Corrected 2026-09-04: no demo
run after the fix is recorded.** One green run from nothing exists (lane A's
rebased branch, 2026-09-04, **without** lane G — it was rebased onto `068d8b7`,
master plus lanes B and D, and lane G merged later as `53f7a7e`; the race is
timing-dependent and did not fire), lane A's own earlier count was two greens
in six attempts and zero for three from nothing, lane G did not re-run the
demo. **Run on the merged branch, 2026-09-04, in the `vpay-ci` VM (code as of `4b5a9d7`, lanes G and H in):** `just demo` from nothing **six times, four green** (six outcomes for six each, exit 0; the first green is the paste in `docs/runbooks/demo.md` §4). The two failures were not the race: in both, the VM's Postgres answered single statements in 14–36 s while the host's I/O pressure was above 50 % (a second VM and two reviewer builds), and the worker's log shows the settlement and the webhook landing _after_ the demo's 120 s / 30 s budgets — a `DELETE FROM jobs` at 18 s and a `COMMIT` at 14.6 s in one, `INSERT`s at 5 s each in the other. `write_matched_no_row` appeared in no run's server or worker log. The plan's bar of three from nothing is met in count, not consecutively, which is why the row stays 🟡 and this sentence says both. **What proves the fix is lane G's
tests, not the demo** (`docs/status.md`'s confirm/worker race row).

**Two things this section must still be read as saying.** The line references in
the account above (`payment_intents.rs:1368`, `charges.rs:463`,
`run_loop.rs:69`, `recovery.rs:179`) are the ones the defect was found at and
have since moved; the current ones are in `docs/status.md`'s confirm/worker race
row. And **`just demo` has not been run on the merged Step 8 gate branch** — the
one green run from nothing is lane A's, and it predates lane G's fix — so if you
see a `500` with `write_matched_no_row` here, report it: it would be the first
observation of the defect on a tree that carries the fix.
