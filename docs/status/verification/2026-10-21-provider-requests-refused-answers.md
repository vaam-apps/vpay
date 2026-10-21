# 2026-10-21 — a refused status read is an answered one; a refused submit stays unresolved

Branch `claude/epic-bose-09916e`, from `master` at `4ca1fed0` (#308), where the
baseline below was run. `master` was then merged in at `e28292d3` (#310, the
RFC-0012 step 5 page this change answers), which conflicted in the two status
indexes alone (`docs/status/README.md`, `docs/status/backend.md`); both sides
were kept, and the index now counts 86 pages. The change: `vpay-worker`'s `handlers.rs` (one function
split in two, one pure function, one unit test), documentation in `vpay-db`'s
`provider_requests.rs`, one `vpay-db` test, three Postgres-and-WireMock cases in
`worker_recovery.rs`, and the docs. **No route, migration, gate or
`NotImplemented` token moved, and no WireMock mapping was added.**

## What was wrong

RFC-0012 step 5 ran three test-mode payments through vpay against Notch Pay's
real sandbox ([#310](https://github.com/vaam-apps/vpay/pull/310), merged as `e28292d3` while this change was in review). One
payment's reads came back `complete` with an `amounts.total` that was not its
amount, and the adapter refused each as `ProviderError::Malformed`. The worker
wrote every one of those reads to `provider_requests` as

```text
status_code = NULL, responded_at = NULL, error_kind = 'provider_error'
```

— rows 9 to 19 of #310's page, eleven reads — which is the encoding
`vpay_db::provider_requests` defines as "the only durable evidence that a rail
may have been asked to move money and never answered". Notch Pay had answered
each of them, and the adapter had parsed its `complete` and its `amounts`.

`vpay_worker::handlers::record_failure`, whose doc said "Records an attempt that
got no answer at all", was called for **every** `ProviderError` on both paths
that write a row after a call: `submit_again` (the resubmit) and `query_status`.

## What the code says, which is the evidence for the decision

**Who reads a row, and what they do with `NULL`.** Checked by grepping every
`FROM provider_requests` and every `latest_submit_attempt`:

| Reader                                                                                              | Operation it reads | What it does with the status                                                               |
| --------------------------------------------------------------------------------------------------- | ------------------ | ------------------------------------------------------------------------------------------ |
| `Settlement::latest_submit_attempt` → `submit_evidence` → `recovery_step`                           | `submit` only      | `NULL` is `Unanswered` (poll; resubmit after a `NotFound` streak); any status is `Advance` |
| The refund-cancel guards (`NOT EXISTS`/`EXISTS … operation = 'refund'` in `vpay-db`'s `refunds.rs`) | `refund` only      | none — row existence only                                                                  |
| nothing                                                                                             | `query_status`     | —                                                                                          |

So `NULL` on a `query_status` row decides nothing, and an **answered** status on
a `submit` row is `RecoveryAction::Advance`: "the rail accepted the submit and
only our bookkeeping is behind".

**What each `ProviderError` variant means**, from its definition and every
construction site in the three adapters and `vpay-provider`:

- `Malformed`: "the rail answered something this adapter cannot act on". True of
  every response-parsing site on the `submit` and `query_status` paths
  (`status_outcome`, an unknown status, an oversize body, a `3xx`). **Not true of
  two documented uses**: `vpay_provider::http::path_segment` ("`Malformed` for a
  request that never left the process", for a dots-only `trx.…` reference) and
  `parse_destination` (a refund's destination, which neither path reaches). So
  the premise "by construction the rail did answer" is right for what happened in
  step 5 and not exact.
- `Rejected`: the rail decided. Answered, by definition.
- `Transport`: a union. `ProviderError::transport`'s own doc calls the no-source
  form "a rail that answered, badly", and `vpay-api`'s `account_holders.rs` uses
  the same constructor for `Answer::Unreachable`. The worker cannot split it.
- `Config`: a union too — raised before any request (Notch Pay's missing
  credential, channel or currency) **and** after the rail answered (Orange's
  `404`/`4xx` on a read, MTN's `500` carrying a configuration code).
- `Unsupported`, `NotImplemented`: raised before a socket opens, which is the
  `NULL` encoding's own example in `record_response`'s doc.

**Fail-closed on a submit is deliberate, in the adapters and in the API.**
`vpay-adapter-notchpay` returns `Malformed` for an initialize that answered `409`
when the charge holds no Notch Pay reference, "so its fate is unknown and an
operator must reconcile it"; `vpay-adapter-mtn-momo` returns it for an unfollowed
`3xx`, "never a decline: the charge's fate is unknown". `vpay-api`'s
`finish_confirm` records `Malformed` on the first submit with `status_code NULL`
and says why in a comment ("we do not know what the rail did with the request"),
and the refund path does the same for `Transport` and `Malformed`. The worker's
resubmit was the one writer that did not distinguish.

## The decision

| `ProviderError`                                        | `submit` row | `query_status` row      |
| ------------------------------------------------------ | ------------ | ----------------------- |
| `Malformed`                                            | `NULL`       | `0` + `provider_error`  |
| `Rejected`                                             | `NULL`       | `0` + `charge_declined` |
| `Transport`, `Config`, `Unsupported`, `NotImplemented` | `NULL`       | `NULL`                  |

Answered on a **read** because a read moves no money and nothing reads the row.
Unresolved on a **submit** because recovery would read an answered row as
acceptance. A `Config` that fires before any request leaves a row regardless —
`insert_pending` runs before the call — and deleting it would erase the evidence
the write-before-network ordering exists to keep. The diagrams, the residuals and
the open gap are in
[vpay-worker.md § "What a refused answer is recorded as"](../../reference/vpay-worker.md#what-a-refused-answer-is-recorded-as).

## Tests, and what each mutation did to them

Five tests are new. Each was written, run green, and then run against a named
mutation of the code it covers, to see it fail:

| New test                                                                         | Where                                          | What it runs                                                                                                                                                     |
| -------------------------------------------------------------------------------- | ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `a_refused_read_is_recorded_as_answered_only_when_the_rail_spoke`                | `vpay-worker`, unit                            | `read_failure_status` over all six `ProviderError` variants                                                                                                      |
| `an_answer_the_caller_refused_is_stored_as_answered_and_keeps_its_reason`        | `vpay-db/tests/repositories.rs`, real Postgres | the sentinel **and** an `error_kind` in one row, read back with a plain `SELECT` (the combination the other test of this table did not cover)                    |
| `a_status_read_the_adapter_refused_is_recorded_as_answered_and_a_503_is_not`     | `worker_recovery.rs`, Postgres + WireMock      | two charges polled once each: `…0b16` (a 300 KB body, `Malformed`) leaves `0`/answered/`provider_error`; `…05aa` (a `503`, `Transport`) leaves `NULL`/unanswered |
| `a_resubmit_the_adapter_refuses_as_malformed_stays_unanswered_for_recovery`      | `worker_recovery.rs`, Postgres + WireMock      | a never-sent charge, resubmitted into `…0302` (`307`, `Malformed`): row `NULL`, `latest_submit_attempt` reads `NULL`, charge still `submitting`                  |
| `a_resubmit_the_rail_declines_stays_unanswered_because_nothing_fails_the_charge` | `worker_recovery.rs`, Postgres + WireMock      | the same into `…0400` (`400 PAYER_NOT_FOUND`, `Rejected`): job `Finished`, row `NULL` + `charge_declined`, charge still `submitting`                             |

**Mutations**, each a one-line edit of `handlers.rs`, restored byte-for-byte
afterwards (`diff` against a copy taken first):

| Mutation                                                                          | Unit | Read, 503 | Submit, `Malformed` | Submit, `Rejected` |
| --------------------------------------------------------------------------------- | ---- | --------- | ------------------- | ------------------ |
| M1 — `Malformed` treated as unanswered on a read: the defect step 5 hit           | FAIL | FAIL      | pass                | pass               |
| M2 — `record_submit_failure` uses the read rule: the tempting one-line fix        | pass | pass      | **FAIL**            | **FAIL**           |
| M3 — `Transport` recorded as answered on a read                                   | FAIL | FAIL      | pass                | pass               |
| M4 — the write restored to the pre-fix `None`, `read_failure_status` left correct | pass | **FAIL**  | pass                | pass               |

M4 is why the Postgres case exists next to the unit one: the pure function can be
right while the row is wrong, and only the second one sees that. M2 is the
change this PR exists to refuse, and both submit cases catch it.

**Counts** (`passed` and `ignored`; nextest reports ignored tests as skipped):

| Suite                                                  | Result                                                                                         |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| `cargo nextest run -p vpay-worker`                     | 76 passed, 0 ignored (75 before: +1)                                                           |
| `vpay-db` — 140 unit, 4 `postgres`, 122 `repositories` | 266 passed, 0 ignored (`repositories` was 121 before: +1), from the workspace run below        |
| `worker_recovery` + `worker_e2e`                       | 29 passed, 0 ignored (`worker_recovery` 23 before: +3; `worker_e2e` 3)                         |
| `worker_kill9`                                         | 6 passed (1 slow), 0 ignored — the shipping binaries and a real `SIGKILL`                      |
| `cargo xtask` `doc_count_tests`                        | 23 passed, 0 ignored                                                                           |
| `just test-doc` (`cargo test --doc --workspace`)       | 130 passed, 0 failed, **1 ignored** — the same one `verify-docs` reports; no doctest was added |
| `just verify-ignored`                                  | 0 ignored (expected 0), 53 test binaries (expected 53), 2339 total (minimum 1080)              |

**After the merge of `master` at `e28292d3`** (which changed the Notch Pay adapter,
`vpay-server`'s `main.rs`, the conformance suite and `compose.yml`, none of them
on this change's paths), these were run again on the merged tree and agree:
`vpay-worker` 76 passed, 0 ignored; the two `vpay-db` provider-request cases 2
passed; `worker_recovery` + `worker_e2e` 29 passed, 0 ignored; `doc_count_tests`
23 passed; `just verify` exit 0 with all fifteen gates (`verify-links` 2498 links
in 467 files); `rustfmt` and `clippy --workspace --all-targets -D warnings`
clean. `worker_kill9`, the doctests, `verify-ignored` and the interrupted
workspace run below are **pre-merge** numbers and were not repeated.

**The workspace run was not completed, and that is on me.** `cargo nextest run
--workspace` was stopped at **1817 of 2339** (212 s of test time) when I read the
slow end of it as stalled; it had only reached the container-booting
`vpay-server::cli` tests. 1815 passed, 0 ignored, and two did not: `xtask`
`doc_count_tests::the_repository_itself_passes`, which was a real failure at that
moment — `docs/status/README.md` already said 85 pages and this page did not yet
exist — and passes in the re-run above, and one `vpay-server::cli` case that my
own `SIGTERM` killed. **The 522 not run**: 25 `vpay-server::cli`, 6 `vpay-server`
binary, 11 `vpay-testkit`, 94 `adapter_conformance`, and every
`vpay-tests-integration` binary — of which only `worker_recovery`, `worker_e2e`
and `worker_kill9` were run afterwards. The rest of the integration suites
(payment intents, refunds, checkout sessions, customers, invoices, webhooks, the
staff sign-in cases among them) were **not run**. A `grep` for `provider_requests`
over them finds assertions on the API's own rows (`confirm_rails`,
`payment_intents`, `refunds`) and none on a refused `query_status` or resubmit
row, and `vpay-api` is untouched; that is a reading and not a measurement.

## Gates

| Gate                                                                                         | Before (`master` at `4ca1fed0`, clean tree) | After (this branch)                                                   |
| -------------------------------------------------------------------------------------------- | ------------------------------------------- | --------------------------------------------------------------------- |
| `just verify` — the fifteen gates                                                            | exit 0                                      | exit 0                                                                |
| `verify-links`                                                                               | 2467 links in 465 files                     | 2474 links in 466 files (this page and its links)                     |
| `verify-doc-counts`                                                                          | 13 counts in 11 of 294 files agree          | the same 13 agree; the marked count of verification pages is 85       |
| `cargo fmt --all -- --check`                                                                 | —                                           | clean (after `cargo fmt --all` rewrapped two test tuples)             |
| `cargo clippy --workspace --all-targets -- -D warnings`                                      | —                                           | clean                                                                 |
| prettier 3.9.6, the version `package.json` pins, on the seven markdown files this PR changes | —                                           | applied; `vpay-worker.md` and `provider-error-rate.md` were reflowed  |
| `markdownlint-cli2@0.20.0` with CI's config, on the six changed docs at `HEAD` and now       | 0 errors each                               | 0 errors each (the tool was shown to flag a known-bad file: 5 errors) |

`check-schema` printed its usual warning on both runs — `cratestack 0.15.1` is on
`PATH` and the repository pins `0.15.0` — which is not this change's.

## Found on the way

- **A stale claim, corrected.** `vpay_db::provider_requests` said "Nothing reads
  this table … the worker job loop is not started" and had since 2026-09-03.
  `latest_submit_attempt` has read it on every poll of a `submitting` charge
  since Step 4, the same day. [#310](https://github.com/vaam-apps/vpay/pull/310)'s
  page repeats the old sentence ("Today 'Nothing reads this table', … so the
  consequence is audit imprecision, not money"); it carries a dated correction now,
  added by this change. The consequence it names was right for a `query_status` row
  and was not right for a `submit` one.
- **A declined resubmit leaves the charge `submitting`.** `ProviderError::Rejected`
  is `Retry::NewAttempt`, so the loop finishes the resubmit job and nothing fails
  the charge — unlike `vpay-api`'s first submit, which records the decline and
  fails the charge in one handler. This is why the worker may not record it as
  answered (recovery would `Advance` a charge the rail refused).
  `a_resubmit_the_rail_declines_stays_unanswered_because_nothing_fails_the_charge`
  runs the first decline: job `Finished`, row unanswered, charge `submitting`.
  That the next `NotFound` streak then resubmits again, until the 24-hour horizon
  escalates, is read from the code and **not run**.

  **Measured and fixed later the same day**
  ([2026-10-21-declined-resubmit-fails-the-charge.md](2026-10-21-declined-resubmit-fails-the-charge.md)):
  the loop is real, a declined resubmit now fails the charge, and `Rejected` on a
  submit is recorded answered, after it. The decision table above is the state of
  this change as reviewed; its `Rejected` / `submit` cell is `0` +
  `charge_declined` now, and the test named above was replaced.

## What was not done, and not run

- **Not re-run against Notch Pay.** The eleven reads of step 5 were not
  reproduced; that needs a test key, and Notch Pay's short `amounts.total`. The
  `Malformed` read in the Postgres case is MTN's 300 KB-body stub, which is a
  real `Malformed` on a real status read through the shipping adapter and
  worker, and not Notch Pay's refusal. The `worker_recovery` harness has MTN and
  Orange stubs and no Notch Pay one.
- **`Rejected` and `Config` on a read have no end-to-end case.** MTN's status path
  does not produce either from a stub in the tree; both are covered by the unit
  table and by the reasoning above, not by a row read back from Postgres.
- **A declined resubmit's second pass.** That the next `NotFound` streak
  resubmits again, until the 24-hour horizon escalates, was read from the code and
  not run when this page was written; it was run, and fixed, later the same day
  (see above).
- **The workspace test run, the rest of it** — the 522 listed above, other than
  `worker_recovery`, `worker_e2e` and `worker_kill9`.
- **`just ci`'s web half.** `just fmt-check`'s whole-tree `prettier --check .`,
  `lint-web`, `test-web`, `audit-web`, `deny` and `test-storybook` were not run.
  This worktree has no `node_modules`, and `pnpm exec prettier` sat at 0% CPU for
  more than ten minutes without printing anything, so the pinned prettier was run
  on the changed files from outside the repository instead. No web code,
  dependency or lockfile moved.
- **`cargo xtask verify-citations`** (network and a token) was not run. This page
  and the docs cite #310, a merged PR that exists, and no CI run id.
- **#310's finding was corrected in place, nothing else on its page was touched.**
  It repeated "Nothing reads this table", which was true of `query_status` rows and
  not of `submit` ones; the correction is dated and points here.
- **`vpay-docs`** (the human documentation, pinned to a release tag) was not
  touched; AGENTS.md says its `release-parity` workflow reports at the next tag.
- **The `vpay-skills` companion** is a draft PR, to merge after this one. Its
  `CHANGELOG.md` entry names a merged vpay commit and waits for one.
- **No change to the port.** Making `Transport`/`Config` exact on a read would
  mean `ProviderError` carrying whether anything was sent, against ADR-0002's
  rule that the port does not expose the rail's status line. That is a decision
  for a maintainer, not made and not costed here.
