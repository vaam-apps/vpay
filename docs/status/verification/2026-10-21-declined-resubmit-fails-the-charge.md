# 2026-10-21 — a declined resubmit fails the charge

Branch `claude/fail-declined-resubmit`, **stacked on #314**
(`claude/epic-bose-09916e` at `179ffe06`, itself on `master` at `e28292d3`), where
the measurement below was taken. It is the follow-up #314 named and did not make:
the worker's resubmit now settles a charge the rail declined, the way `vpay-api`'s
first submit does. **No route, migration, gate or `NotImplemented` token moved,
and no WireMock mapping was added** — the two stubs it needs were already in the
tree.

## What was wrong, measured

#314 found that `ProviderError::Rejected` is `Retry::NewAttempt`, so the loop
finishes a declined `resubmit_charge` job and nothing writes the charge, and said
the rest was "read from the code and not run". This is the run. One charge, a
crash fixture that left it `submitting` with nothing ever sent, against the MTN
WireMock tree:

- reference `…0404` — `requesttopay-status.json` answers `404 RESOURCE_NOT_FOUND`
  to **every** read, so the poll's `NotFound` streak is real and not staged;
- payer `237670000400` — `requesttopay.json` answers `400 PAYER_NOT_FOUND` to
  **every** submit for that MSISDN, whatever the reference. Together they are what
  a real decline looks like from the poll's side: the rail refuses the charge and
  then has no record of it.

`RecoveryPolicy { not_found_streak: 3, not_found_window: 50 ms }` — the documented
streak, a shorter window, the same struct `main` builds — and sixteen steps, each
`make_every_job_runnable`, a 60 ms pause and one `run_once`. **Before the change:**

| Step  | Job               | Disposition         | `submit` rows | status reads | Charge       | Intent                    |
| ----- | ----------------- | ------------------- | ------------- | ------------ | ------------ | ------------------------- |
| 1     | `poll_charge`     | `Rescheduled(10s)`  | 0             | 0            | `submitting` | `requires_payment_method` |
| 2     | `resubmit_charge` | `Finished`          | **1**         | 0            | `submitting` | `requires_payment_method` |
| 3     | `poll_charge`     | `Rescheduled(20s)`  | 1             | 1            | `submitting` | `requires_payment_method` |
| 4     | `poll_charge`     | `Rescheduled(30s)`  | 1             | 2            | `submitting` | `requires_payment_method` |
| 5     | `poll_charge`     | `Rescheduled(45s)`  | 1             | 3            | `submitting` | `requires_payment_method` |
| 6     | `resubmit_charge` | `Finished`          | **2**         | 3            | `submitting` | `requires_payment_method` |
| 7–9   | `poll_charge` × 3 | `Rescheduled(…)`    | 2             | 4 → 6        | `submitting` | `requires_payment_method` |
| 10    | `resubmit_charge` | `Finished`          | **3**         | 6            | `submitting` | `requires_payment_method` |
| 11–13 | `poll_charge` × 3 | `Rescheduled(120s)` | 3             | 7 → 9        | `submitting` | `requires_payment_method` |
| 14    | `resubmit_charge` | `Finished`          | **4**         | 9            | `submitting` | `requires_payment_method` |
| 15–16 | `poll_charge` × 2 | `Rescheduled(120s)` | 4             | 10 → 11      | `submitting` | `requires_payment_method` |

So the loop is real: a rail that has declined a charge is sent it again after
every third poll, for as long as the charge lives — the 24-hour horizon is the only
end to it, and it ends in `unresolved` and an alert about a charge the rail refused
on the first try. The intent never learns why: it stays `requires_payment_method`
with no `last_payment_error` and no `payment_intent.payment_failed` event. At the
documented 60 s window the loop is slower, not absent; that was not measured.

## What a declined resubmit does now

`submit_again` (`handlers.rs`), on `ProviderError::Rejected`:

1. `settle_failed_with` — the poll path's own decline settlement, unchanged:
   `apply_failed` in one transaction. Charge `failed` with the rail's failure code
   and words, intent back to `requires_payment_method` carrying the error pair,
   one `payment_intent.payment_failed` event. It works on a `submitting` charge
   because `LIVE_CHARGE_STATES` includes it, and on an intent a crashed confirm
   left at `requires_payment_method` because `SETTLEABLE_STATUSES` does.
2. `record_declined_submit` — only then is the attempt row marked answered: the
   `0` sentinel and `charge_declined`, the row `vpay-api`'s confirm writes for the
   same answer.
3. The error is raised unchanged. `log_failure` still counts it in
   `vpay_error_events_total`, a `ProviderAccountBlocked` still pages, and
   `Classify::retry` still derives `Decision::Terminal`.

**Why the row comes second.** `recovery_step` reads a `submit` row for a
`submitting` charge and turns an answered one into `Advance` ("the rail accepted
it"). Row first, then a crash or a failed settlement, leaves exactly that pair and
the charge would be advanced to `submitted` and polled to the horizon. Settlement
first, then a crash, leaves a `failed` charge beside an unanswered row, which
nothing reads. A settlement that itself fails is retried by the loop (`Db` is
transient); the retry makes a new attempt row and the first stays unanswered for it
to supersede, as `latest_submit_attempt` already orders by `sent_at DESC, id DESC`.

**Why a decline of a resubmit can be trusted — and what it assumes.** A resubmit
happens only when the table says nothing was ever sent, or after three consecutive
`NotFound` reads over the window, so the rail has no record of the reference; and
the port's contract is that a duplicate is reported as `Submitted`, never as an
error (all three shipped adapters comply: MTN's `409`, Notch Pay's `AlreadyExists`,
Orange never resubmitted). So a `Rejected` is the rail declining _this_ request.
**The assumption is the same one `vpay-api` makes for a first submit, and so is its
weakness:** a rail that declined a duplicate instead of reporting `Submitted` would
break the contract, and a terminal charge is not polled, so vpay would not see a
payment that rail then took. This is a money-path decision a maintainer should
read, and it is the first thing the PR asks to be reviewed.

**What did not change.** A `Rejected` on a status _read_ does not fail the charge —
Notch Pay raises it "only for our own refused keys", which is a fact about our
credentials. `Malformed` on a resubmit still leaves the fate unknown and the row
unanswered (`N3` below holds that line). `Transport`, `Config`, `Unsupported` and
`NotImplemented` are as #314 left them.

## Tests, and what each mutation did to them

Two cases changed and one was added, all in `worker_recovery.rs` against real
Postgres and WireMock. `a_resubmit_the_rail_declines_stays_unanswered_because_nothing_fails_the_charge`,
which #314 added to **pin the gap**, was replaced as its own doc said it should be.

| Test                                                                        | What it asserts                                                                                                                                                                                                                                        |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `a_resubmit_the_rail_declines_fails_the_charge_and_is_recorded_as_answered` | job `Finished`; charge `failed` with `invalid_payer`; intent `requires_payment_method` with `last_payment_error_code` `invalid_payer`; exactly one `payment_intent.payment_failed`; the `submit` row `0` / answered / `charge_declined`; one reference |
| `a_declined_resubmit_is_not_resent_by_the_poll_that_follows`                | the measured loop, turned round: charge `failed`; the leftover poll job `Finished`; **no job left in the queue**; one `submit` row; **zero** status reads                                                                                              |
| `a_resubmit_the_adapter_refuses_as_malformed_stays_unanswered_for_recovery` | unchanged from #314 — the fail-closed line                                                                                                                                                                                                             |

**Mutations**, each an edit of `handlers.rs`, restored byte-for-byte afterwards
(`diff` against a copy taken first):

| Mutation                                                                        | Declined → fails | Declined → not resent | `Malformed` stays unanswered |
| ------------------------------------------------------------------------------- | ---------------- | --------------------- | ---------------------------- |
| N1 — no `settle_failed_with`, row still marked answered: the **dangerous pair** | **FAIL**         | **FAIL**              | pass                         |
| N2 — settlement kept, row left unanswered                                       | **FAIL**         | pass                  | pass                         |
| N3 — `Malformed` also fails the charge                                          | pass             | pass                  | **FAIL**                     |
| N4 — the pre-change behaviour (no settlement, row unanswered)                   | **FAIL**         | **FAIL**              | pass                         |

N4 is the "fails before the fix, passes after" check; N1 is the one the order
argument is about, and both declined-resubmit cases catch it. N2 shows the two
assertions are separate. N3 is why the `Malformed` case from #314 stays.

**Counts** (`passed` and `ignored`; nextest reports ignored tests as skipped), on
this branch:

| Suite                                        | Result                                                                                      |
| -------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `cargo nextest run -p vpay-worker`           | 76 passed, 0 ignored (unchanged: the pure function was not touched)                         |
| `vpay-db`'s two provider-request cases       | 2 passed, 0 ignored (the module's docs changed, its code did not)                           |
| `worker_recovery` + `worker_e2e`             | 30 passed, 0 ignored (`worker_recovery` 27: #314's 26 with one case replaced and one added) |
| `worker_kill9`                               | 6 passed, 0 ignored — the shipping binaries and a real `SIGKILL`                            |
| `cargo test --doc -p vpay-worker -p vpay-db` | 13 passed (8 + 5), 0 failed, 0 ignored                                                      |
| `just verify-ignored`                        | 0 ignored (expected 0), 53 test binaries (expected 53), 2340 total (2339 on #314's branch)  |

| Gate                                                              | Result                                                                                                                  |
| ----------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `cargo fmt --all -- --check`                                      | clean                                                                                                                   |
| `cargo clippy --workspace --all-targets -- -D warnings`           | clean                                                                                                                   |
| `just verify` — the fifteen gates                                 | exit 0, all fifteen gates (`verify-links` 2502 links in 468 files; `verify-doc-counts` agrees at 87 verification pages) |
| prettier 3.9.6 and `markdownlint-cli2@0.20.0` on the changed docs | prettier applied; markdownlint 0 errors on the seven changed docs (the tool was shown on #314 to flag a known-bad file) |

`check-schema` passed under `cratestack 0.12.0` this time, where #314's runs had
`0.15.1` on `PATH` — a difference in this machine's shell, not in the change.

The full-workspace `cargo nextest run` was **not** run for this change. #314's page
records the one attempt at it and why it stopped; the suites above are the ones
this change can reach (`handlers.rs` is the only Rust file with a code change, and
it is the worker's).

## Not done, and not run

- **`vpay-api`'s confirm has the window this change closes for the worker.**
  `finish_confirm` records the answered row, then calls `persist_decline`, two
  commits, so a crash between them leaves a `submitting` charge beside an answered
  row, which `recovery_step` reads as `Advance` ("the rail accepted it") for a
  charge the rail declined. It predates this change and is not changed by it. It
  was pointed out in review of #316 (`lightbridge-assistant`), and the doc
  sentences that implied otherwise — the `provider_requests.rs` module header, the
  unresolved-charges runbook and the crash-safety note — are corrected here.
  Reordering it is its own change, not made.
- **The order argument is a reading, not a measurement.** "Row first would leave a
  `submitting` charge beside an answered row" is the consequence of
  `recovery_step`'s table, which has its own unit and integration cases; no test
  injects a crash between the settlement and the row write.
- **A settlement that fails is not injected either.** The retry behaviour above is
  the loop's existing one for `Db`, not a case written here.
- **No real rail.** Both stubs are WireMock; no declined resubmit has met MTN's or
  Notch Pay's real sandbox.
- **Orange.** A redirect rail is never resubmitted (`FailDeadOrder`), so this path
  does not exist for it.
- **A duplicate that a rail declines** — the assumption above — is argued from the
  three adapters' source, not run against any of them.
- **The workspace-wide `cargo nextest run` and the web half of `just ci`** were not
  run; see the counts above for what was.
- **`vpay-skills#52`** is updated for this change and is still a draft that must
  merge after both vpay PRs.
