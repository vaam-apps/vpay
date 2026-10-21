# 2026-10-21 — a declined confirm fails the charge before it records the attempt

Branch `claude/confirm-decline-order`, from `master` at `6d9dfbf5` (#314, which
carried #316). The follow-up the review of #316 asked for and that change
deliberately did not make: `vpay-api`'s `finish_confirm` now settles a decline
**before** it completes the attempt row, the order the worker's resubmit already
uses. **No route, response, migration, gate or `NotImplemented` token moved, and
no WireMock mapping was added.**

## What was wrong, measured

`finish_confirm` called `record_response(Some(0), charge_declined)` and _then_
`persist_decline` — two commits. #316's review pointed out that a crash between
them leaves a `submitting` charge beside an answered `submit` row, which
`recovery_step` reads as `Advance` ("the rail accepted it"). It did not say
whether a crash is the only way in. It is not.

**1. Reachable without a crash.** A Postgres trigger refuses the one write
`persist_decline` makes to the charge (`BEFORE UPDATE ON charges … WHEN
(NEW.state = 'failed')`), on a confirm against the MTN stub's declined payer —
fault injection at the infrastructure layer, the posture
`payment_intents.rs`'s `a_failed_store_releases_the_key_so_the_retry_re_executes`
documents. **Before the change:**

- the confirm answered `503` `database_query_failed` (`Category::Storage`);
- the charge stayed `submitting`;
- the `submit` row read `(status_code, error_kind, answered) = (0, charge_declined, true)`
  — answered, beside a `submitting` charge. Not a crash: an ordinary refused write.

**2. What recovery then does with that pair.** Staged at kill point 3 against the
honest stub pair (`…0404` is `NotFound` on every read, MSISDN `237670000400` is
declined on every submit, so the production table does the work), fourteen steps,
before the change:

| Step | Job           | Disposition           | `submit` rows | status reads | Charge      |
| ---- | ------------- | --------------------- | ------------- | ------------ | ----------- |
| 1    | `poll_charge` | `Rescheduled(10s)`    | 1             | 1            | `submitted` |
| 2–6  | `poll_charge` | `Rescheduled(20…90s)` | 1             | 2 → 6        | `submitted` |
| 7–14 | `poll_charge` | `Rescheduled(120s)`   | 1             | 7 → 14       | `submitted` |

The first poll **advanced a charge the rail had declined** to `submitted`, and
every poll after it asked the rail again and got `NotFound`. It was never failed
and, being `submitted`, never resubmitted (`resubmit` is `submitting`-only): the
24-hour horizon is the only end, and it ends in `unresolved` and an alert about a
decline the rail made on the first try. (That staging is a probe; it is not
committed.)

**3. The other direction.** A trigger refusing the `UPDATE` on `provider_requests`
(`insert_pending` still runs) made the old order fail _before_ settling: `503`,
charge `submitting`. With the order reversed that write is audit-only, and the
question becomes what a refusal of it should do.

## The change

`finish_confirm`, on `ProviderError::Rejected`:

1. `persist_decline` — unchanged: charge `failed`, intent back to the payer with
   the error pair, one `payment_intent.payment_failed` event, one transaction.
2. `record_response(Some(0), charge_declined)` — only then.
3. **If step 2 fails, the request still answers the decline.** The charge is
   terminal and nothing reads a `submit` row for it, so the failure is logged
   (`tracing::error!`, with the intent, charge and attempt ids) and the row stays
   pending. A `5xx` for a decline that is committed and has been webhooked would
   tell the merchant vpay failed when it did not.

**A refused settlement now leaves `submitting` and unanswered** — exactly what a
crash before the answer leaves, which the recovery table already resolves. The
worker resubmits into the same decline and fails the charge itself (#316).

**The accepted arm keeps its order, and the doc now says why.** Row first, then
`persist_submitted`: there the pair `submitting` + answered is `Advance`'s own
input ("the rail took it and our bookkeeping lags", kill point 3), so recording
the answer first is what makes that pair true. The order is chosen by what
recovery should do with the pair, which is why it differs by outcome.

**What the merchant sees did not change** for a settled decline: `409`
`invalid_request_error` `charge_declined`, the same intent, the same event. The
two cases that changed are both failures of vpay's own writes: a refused
settlement is `503` before and after (`Category::Storage`), and a row that cannot
be completed after a settled decline used to be a `503` (with the charge still
`submitting`) and is now the `409`.

## Against issue #320

[#320](https://github.com/vaam-apps/vpay/issues/320) (filed by the maintainer from
the review of #316) asked for one of two fixes. **This implements Option 2 —
settle first, record second — and not Option 1, the preferred single
transaction.**

| #320 acceptance criterion                                                                  | Where it stands                                                                                                                                                                                                                                                               |
| ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| One transaction, or failed-first then answered-second                                      | Option 2: `finish_confirm`'s `Rejected` arm. Option 1 is **not** done (below)                                                                                                                                                                                                 |
| A test that drives the failure between the two writes                                      | `a_decline_the_database_refuses_to_settle_leaves_the_attempt_unanswered` injects the refusal in Postgres; `a_declined_submit_whose_settlement_was_lost_is_failed_by_the_worker` runs the production table over the resulting rows and fails if the charge is ever `submitted` |
| `crash-safety.md` parenthesis replaced by a dated note naming the ticket and PR            | `docs/flows/crash-safety.md`, "Amended 2026-10-21 (#320, #322)"                                                                                                                                                                                                               |
| `vpay-api.md` § "the confirm path" describes the new order                                 | `docs/reference/vpay-api/confirm-path.md` (the reference lives under `vpay-api/` since the split), "Which write comes first differs by outcome, on purpose"                                                                                                                   |
| Status pages, a dated verification page, a skills companion if a skill describes the crash | `docs/status/backend.md`, this page, the index; the skills companion is **not yet opened**, by design (below)                                                                                                                                                                 |
| Option 2's required sentence in `provider_requests.rs`                                     | the module header: "**A `NULL` `submit` row beside a terminal charge is not evidence.**"                                                                                                                                                                                      |

**Why Option 2.** `vpay-db` has no in-transaction write to `provider_requests`:
Option 1 means a new `TxRepositories` method, its `PendingTransaction`
implementation and the call inside `persist_decline`'s closure — a change to the
repository seam in a money-path transaction, behind `verify-repositories`. The
order alone removes the harm #320 measured (a `submitting` charge beside an
answered row, advanced), and what it leaves is a state the production table
already resolves. **What Option 1 would add** and Option 2 does not: no crash
window at all, no unanswered row beside a terminal charge (the residue the new
sentence documents), and no log-and-continue — a row that could not be written
would roll the decline back and the worker would redo it. That is a real
advantage, and a maintainer who prefers it is asking for a second, small change
on top of this one; nothing here has to be undone for it.

**A redirect rail is not helped by either option, and #320's summary says it
would be.** If the settlement is lost on a redirect rail the charge stays
`submitting`, and `recovery_step` answers `FailDeadOrder` before it reads the
row ("it holds whatever the attempt row says"), so it is failed as
`provider_unavailable` and not with the rail's code. The row's order is
irrelevant there, and so is Option 1: what was lost is the settlement itself. The
reorder removes the push-rail hole (a hidden decline waiting 24 hours) and leaves
the redirect-rail one — a lost decline code — as it was. Not changed, and not run
against Orange (the harness case uses the push rail).

## Tests, and what each mutation did to them

Three tests are new. The first two run the real server and Postgres in
`confirm_rails.rs`; the third is the other half of the argument.

| Test                                                                                         | What it asserts                                                                                                                                                                                                                         |
| -------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `a_decline_the_database_refuses_to_settle_leaves_the_attempt_unanswered`                     | `503` (`Category::Storage`); charge `submitting`; the row `(None, None, false)`; `latest_submit_attempt` reads `NULL` (what `recovery_step` sees); intent unchanged, no error stamp, no event                                           |
| `an_attempt_row_that_cannot_be_completed_after_a_settled_decline_is_still_the_decline`       | `409` `charge_declined`; charge `failed` with `invalid_payer`; exactly one `payment_intent.payment_failed`; the row pending                                                                                                             |
| `a_declined_submit_whose_settlement_was_lost_is_failed_by_the_worker` (`worker_recovery.rs`) | from the staged pending row against the honest stubs: charge `failed` with `invalid_payer`, one event, **three** status reads, **two** submit rows (the staged pending one and the answered `charge_declined` one), nothing left queued |

**Both confirm cases were run against the unfixed code first and failed**, with
exactly the measurements above (`(Some(0), Some("charge_declined"), true)`; and
`(503, "api_error", Some("database_query_failed"))` against the expected `409`).

**Mutations**, each an edit of the shipping source, restored byte-for-byte
afterwards (`diff` against copies taken first):

| Mutation                                                                   | Refused settlement | Refused row | Worker self-heal | Normal decline |
| -------------------------------------------------------------------------- | ------------------ | ----------- | ---------------- | -------------- |
| P1 — the old order (row, then `persist_decline`)                           | **FAIL**           | **FAIL**    | pass             | pass           |
| P2 — new order, but the row's failure propagates (`?`)                     | pass               | **FAIL**    | pass             | pass           |
| P3 — `submit_again` no longer settles (undoes #316's `settle_failed_with`) | pass               | pass        | **FAIL**         | pass           |

P3 shows the third test depends on #316 and not on the confirm path: the
reordering is safe because the worker resolves what it leaves.

**Counts** (`passed` and `ignored`; nextest reports ignored tests as skipped), on
this branch:

| Suite                                                                                 | Result                                                                           |
| ------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `cargo nextest run -p vpay-api`                                                       | 429 passed, 0 ignored                                                            |
| `cargo nextest run -p vpay-worker`                                                    | 76 passed, 0 ignored                                                             |
| `confirm_rails`                                                                       | 15 passed, 0 ignored (13 before: +2)                                             |
| `worker_recovery` + `worker_e2e`                                                      | 31 passed, 0 ignored (`worker_recovery` 28: +1)                                  |
| `worker_kill9`                                                                        | 6 passed, 0 ignored — the shipping binaries and a real `SIGKILL`                 |
| `payment_intents` + `refunds` + `webhooks` (API paths that share the settlement code) | 69 passed, 0 ignored                                                             |
| `cargo test --doc -p vpay-api -p vpay-db -p vpay-worker`                              | 32 passed (19 + 8 + 5), 0 failed, 0 ignored                                      |
| `just verify-ignored`                                                                 | 0 ignored (expected 0), 53 test binaries (expected 53), 2343 total (2340 before) |

| Gate                                                              | Result                                                                                                                  |
| ----------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `cargo fmt --all -- --check`                                      | clean                                                                                                                   |
| `cargo clippy --workspace --all-targets -- -D warnings`           | clean                                                                                                                   |
| `just verify` — the fifteen gates                                 | exit 0, all fifteen gates (`verify-links` 2529 links in 473 files; `verify-doc-counts` agrees at 92 verification pages) |
| prettier 3.9.6 and `markdownlint-cli2@0.20.0` on the changed docs | prettier applied; markdownlint 0 errors on the seven changed docs                                                       |

**After merging `master` at `e3931169`** (#307, #315, #317: the Node SDK's local
refusals, `tests/cli.rs` diagnostics and the release branch — none on this
change's paths), which conflicted in `docs/status/README.md` alone and kept both
sides (the index now counts 94 pages), these were re-run on the merged tree and
agree: `just verify` exit 0, all fifteen gates (`verify-links` 2537 links in 475
files); rustfmt and clippy clean; `confirm_rails` + `worker_recovery` 43 passed, 0
ignored. **`master` moved again at `0706cd57`** (#319: rail-callback retries — code in
`provider_callback.rs`, both `jobs.rs` and their tests, so on the neighbouring
paths this time), conflicting in the status index alone again (it now counts 95
pages). Re-run on that tree: `just verify` exit 0, all fifteen gates
(`verify-links` 2557 links in 476 files); rustfmt and clippy clean; `vpay-api` +
`vpay-worker` 505 passed, 0 ignored; `confirm_rails` + `worker_recovery` +
`provider_callback` 54 passed, 0 ignored. The other suites in the table are
pre-merge numbers.

**A third time, at `63c395df`** (#323: the dashboard's theme script and re-sign-in
route — `frontends/` only, plus its two flow docs), conflicting in the status index
alone (it now counts 96 pages, 73 listed, the same 23 apart). Re-run on that tree:
`just verify` exit 0, all fifteen gates (`verify-links` 2563 links in 477 files);
rustfmt and clippy clean. **No Rust changed in that merge, so the Rust suites were
not re-run.** #323 also edits `dashboard.cy.ts`, the spec file whose 17 failures
made `e2e (compose)` red on `master` and on this PR before it, so this head is the
first to carry that change; whether the check turns green is for its own run to
say, not for this page.

The full-workspace `cargo nextest run` was **not** run for this change; the suites
above are the ones its one code change (`finish_confirm`) can reach.

## Not done, and not run

- **No crash is injected.** A refused write is the reachable form of the window;
  a `kill -9` between the two commits would leave the same rows, and is not
  staged. `worker_kill9.rs` kills the shipping binaries at the worker's kill
  points, not inside `finish_confirm`.
- **The `Transport`/`Malformed` arm of `finish_confirm`** records its row with `?`
  and no state write follows it; a refused row there is still a `503` instead of
  the rail's `502`. Unchanged and out of scope: there is no pair to misread.
- **A refund's `Rejected`** also records an answered row before releasing its
  reservation (`refunds.rs`), but `recovery_step` reads only `submit` rows and the
  cancel guards read a `refund` row's existence, so the order is immaterial there.
  Not changed, and checked by reading, not by a test.
- **The probe of the old order's consequence** (the fourteen-step table) was run
  once against the unfixed model and is not committed; the committed tests assert
  the pair's cause and the new order's cure, not the old order's consequence.
- **Real rails.** Every rail here is a WireMock host.
- **The workspace-wide `cargo nextest run` and the web half of `just ci`** were not
  run; the counts above are what was.
- **`vpay-skills`**: its `vpay-reconciler` paragraph (merged in #52) says the API's
  confirm records the row first; that is true until this change merges and false
  after. A companion PR corrects it and adds the changelog entry, and is to be
  opened **after** this merges, so that it never describes behaviour `master` does
  not have.
