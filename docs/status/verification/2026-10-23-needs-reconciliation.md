# 2026-10-23 — an answer only a person can act on escalates at once (#313, option 1)

Branch `claude/happy-hopper-r3h6rz`, from `master` at `8504536f` (#340).
It builds [vaam-apps/vpay#313](https://github.com/vaam-apps/vpay/issues/313)
option 1. **It is proven against WireMock's Notch Pay and a real Postgres.
Nothing was sent to Notch Pay.**

## Whose decision

#313 offered three options for a rail's "an operator must reconcile this".
The maintainer chose: "Go with option 1 on #313, build it". The ticket's
implementation plan was written for shape 1a, a new `ProviderError` variant,
and that is what was built. The implementer made the calls below, and the pull
request names each one so the maintainer can reverse it:

- Two more answers became the new variant besides the two #313 named (the
  short `amounts.total` and the `409` with no reference):
  - a `complete` for another amount or currency;
  - a `complete` with no integer `amounts.total`.
- `notchpay`'s missing `trx.…` moved from `Config` to the new variant, and so
  did `orange_money`'s missing `pay_token`. One meaning, one answer.
- The API's confirm does not escalate the charge itself. Its poll job does.
- A read refused with the new variant is recorded as answered (`0`), with one
  documented over-claim.

## What was built

| Piece                                                                                                                                                                                                                                                                           | Where                                                                    |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `ProviderError::NeedsReconciliation { context }`, `ProviderError::needs_reconciliation`: `Rail`, code `charge_needs_reconciliation`, `Severity::Error`, `Rail`'s `AfterBackoff`, a public message with no rail text; a row in the port's error table (`submit`, `query_status`) | `vpay-provider/src/lib.rs`                                               |
| The short `amounts.total`, a `complete` for another amount or currency, a `complete` with no integer `amounts.total`                                                                                                                                                            | `vpay-adapter-notchpay/src/mapping.rs` (`require_the_charges_amount`)    |
| The initialize `409` with no held reference; the read of a charge with no `trx.…` (was `Config`)                                                                                                                                                                                | `vpay-adapter-notchpay/src/lib.rs`                                       |
| The read of a charge with no `pay_token` (was `Config`)                                                                                                                                                                                                                         | `vpay-adapter-orange-money/src/lib.rs`                                   |
| `poll_charge` escalates on it at any age; `resubmit_charge` escalates on it from `submitting`; both through `needs_an_operator` → `escalate_to_unresolved`                                                                                                                      | `vpay-worker/src/handlers.rs` (`rail_did_not_answer`, `resubmit_charge`) |
| A resubmit's row stays `NULL` (`record_unresolved_submit`); a read's says `0` (`read_failure_status`)                                                                                                                                                                           | `vpay-worker/src/handlers.rs`                                            |
| An explicit confirm arm: the charge stays `submitting`, the row unanswered, `502` with code `charge_needs_reconciliation`, a `WARN`                                                                                                                                             | `vpay-api/src/v1/payment_intents.rs` (`finish_confirm`)                  |
| The refund path's cautious arm and its `error_kind`; the callback pass's log arm                                                                                                                                                                                                | `vpay-api/src/v1/refunds.rs`, `vpay-worker/src/callbacks.rs`             |
| ADR-0011's amendment                                                                                                                                                                                                                                                            | `docs/adr/0011-error-modelling.md`                                       |

Unchanged: `Malformed`'s meaning, an answer the adapter could not read; every
existing code and status; `JobError::decision`; the poll ladder; the horizon;
`escalate_to_unresolved` itself.

## How it is proven

**Three new cases in `backends/tests/integration/tests/worker_recovery.rs`.**
Each runs against WireMock's Notch Pay, mounted from the conformance tree, and
its own Postgres, with the real `notchpay` adapter. All three use the
production `RecoveryPolicy::default()` with its 24-hour horizon.

- `a_short_collection_is_unresolved_on_the_first_poll_and_an_unreadable_answer_is_not`.
  Two `pending` charges are polled once each.
  - The short collection (`trx.test_…0f10`, `complete` with
    `amounts.total` 4980 of 5000) gives `Rescheduled(3600 s)` with
    `alert = true`. The charge is `unresolved` with no `failure_code`. Its read
    row is `0` + `charge_needs_reconciliation`.
  - The 300 KB body (`…0b16`, `Malformed`) gives a ladder rung under 60 s and
    no alert. The charge stays `pending`.
  - A second hourly poll re-alerts without rewriting the charge
    (`updated_at` unchanged) and asks Notch Pay again. The poll job is never
    parked.
- `a_short_collection_the_worker_escalated_still_settles_on_a_later_answer`.
  After the escalation, a priority-0 mapping is added through WireMock's
  admin API, so the same payment now reads `complete` for 5000. The next poll
  gives `Finished`: charge and intent `succeeded`, `provider_txn_id` the
  `trx.…`, `amount_received` 5000.
- `a_duplicate_no_submit_can_resolve_is_unresolved_by_its_resubmit_and_never_resent`.
  A never-sent `submitting` charge on `…00dd` is resubmitted. Initialize
  answers `409`, and the resubmit gives `Rescheduled(3600 s)` with an alert.
  The charge is `unresolved` and its submit row stays `NULL` +
  `charge_needs_reconciliation`. On the next round the resubmit's re-run is
  `Finished` without calling the rail, and the poll is hourly with an alert.
  Exactly one submit row exists and one reference was ever used. The poll's
  read row is `0`: that is the documented over-claim, because no request left.

**Mutations, each run by hand on 2026-10-23 and reverted before anything
else ran:**

| Mutation                                                                                  | What failed                                                                                                                                                                    |
| ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Both worker arms disabled (`… if false`)                                                  | all three cases: `left: Rescheduled(10s)`, `right: Rescheduled(3600s)`                                                                                                         |
| The resubmit arm alone disabled, and the short `amounts.total` mapped back to `Malformed` | the `409` case at the resubmit's assertion; both short-collection cases at the first poll; `mapping::tests::a_complete_that_collected_less_than_the_charge_is_never_a_success` |
| `read_failure_status` writing `NULL` for the new variant                                  | `handlers::tests::a_refused_read_is_recorded_as_answered_only_when_the_rail_spoke`                                                                                             |

**Unit, doc and conformance tests:**

- **Adapter and conformance:**
  - the three amount refusals in `mapping.rs`;
  - `a_duplicate_with_no_held_reference_needs_reconciliation_and_sends_nothing_more`;
  - `a_read_without_notch_pays_reference_needs_reconciliation_before_any_request`;
  - Orange's new `a_read_without_a_pay_token_needs_reconciliation_before_any_request`,
    which points at a closed loopback port;
  - the two Notch Pay conformance cases (`…_fails_closed`, the short
    collection).
- **Port and API:**
  - the constructor's doctest (category, code, severity, public message);
  - `vpay-api`'s error table: `502`, `api_error`,
    `charge_needs_reconciliation`;
  - `vpay-api`'s retry table: `Stripe-Should-Retry: true`, as for a
    `Malformed`.

## Gates

They were run on this branch from a Claude Code cloud session. Postgres 16
and WireMock 3.9.2 ran in Docker. `cargo nextest` and `just` are not
installed here, so each recipe's commands ran directly, and `cargo test`
replaced nextest.

| Check                                                                        | Result                                                                                                                                          |
| ---------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo fmt --all --check`                                                    | clean                                                                                                                                           |
| `cargo clippy --workspace --all-targets -- -D warnings`                      | clean                                                                                                                                           |
| `cargo test --workspace --lib --bins --tests` (integration, conformance out) | **1891 passed, 0 failed, 0 ignored** (was 1890: one new Orange case)                                                                            |
| `cargo test --doc --workspace`                                               | **132 passed, 0 failed, 1 ignored** (was 131: the constructor's doctest; the ignored one is `sdks/rust/src/lib.rs` `ReadmeDoctests`, untouched) |
| `cargo test -p vpay-tests-conformance`                                       | **97 passed, 0 failed, 0 ignored**                                                                                                              |
| `cargo test -p vpay-tests-integration`                                       | **396 passed, 2 failed, 0 ignored** (398 cases: the 395 of 2026-10-22, and the three new ones). Both failures pass alone (below)                |
| the fifteen gates `verify` names, and `verify-citations`                     | all ok: `verify-errors` 20 error types, `verify-links` 2680 links, `verify-doc-counts` 13 counts, `verify-migrations` 51 files, `schema OK`     |
| `verify-ignored`'s count                                                     | unchanged: no test binary was added, so `expected_suites` stays 54                                                                              |
| `prettier --check` over the changed Markdown and JSON                        | clean                                                                                                                                           |

`cargo xtask verify-docs` is a report and never fails; it was run and not
compared line by line against `master`.

**The two integration failures, and why neither is this change's.**
`webhooks.rs`'s `the_ladder_walks_delivery_delay_and_then_succeeds` and
`worker_e2e.rs`'s `a_confirmed_payment_is_driven_to_succeeded_and_the_merchant_sees_it`
assert exact counts on the process-wide Prometheus recorder. `cargo test` runs a
binary's cases as threads of one process, so each scrape held other cases'
counts (`vpay_webhook_deliveries_total{outcome="retry"} 8` where three were
asked for). nextest, which CI runs, gives each case its own process. Run alone
with `--exact`, both pass. They are two of the four cases the 2026-10-22 page
names for the same reason; the other two passed in this run's order.
`cargo test` also stops at the first failing binary, so the run went in two
parts: every binary through `webhooks.rs`, then `--no-fail-fast` over the
four `worker_*` binaries after it. `postgres_smoke.rs`'s drift test passed
with the `cratestack` CLI on `PATH` and `RUST_BACKTRACE=0`.

## Not done

- **A real Notch Pay short collection reaching `unresolved`.** It needs
  another sandbox run, on the maintainer's approval in that session.
- **The confirm arm, end to end.** A confirm mints its own reference, and the
  stub answers `409` to one fixed reference, so no test drives the arm.
  `vpay-api`'s error tables prove its envelope.
- **`vpay_charge_transitions_total{to="unresolved"}` is not asserted on this
  path.** `worker_recovery.rs` installs no metrics recorder. The transition is
  `set_live_state`'s own `record_transition`, the same call the horizon's
  escalation makes.
- **A way for an operator to mark an `unresolved` charge reconciled** (#313
  § 8). An escalated charge whose answer can never change re-alerts hourly
  for as long as it lives.
- **Garbled answers on MTN and Orange**: no new case. The existing
  `a_status_read_the_adapter_refused_is_recorded_as_answered_and_a_503_is_not`
  (MTN, `Malformed`) still passes unchanged, and neither adapter raises the
  new variant on a read except Orange's missing `pay_token`.
- **No `docs/status.md` change**: no route, gate or `NotImplemented` token
  moved.
