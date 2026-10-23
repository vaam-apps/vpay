# 2026-10-23 — a rail-issued reference is committed before the prompt (ADR-0032)

Branch `wip/two-step-submit`, from `0fb04ec5`, the head of
[vaam-apps/vpay#341](https://github.com/vaam-apps/vpay/pull/341) (which merged
to `master` as `c672fffc`). It closes the Notch Pay crash window RFC-0012 § 10
item 19 left open. **It is proven against WireMock's Notch Pay and a real
Postgres. Nothing was sent to Notch Pay.**

## Whose decision

The maintainer listed Notch Pay's crash window as item 13 and asked "#13: how
do we do then?". The agent answered with the design below, and the maintainer
replied "Fan out the work via sub-agents". That is an instruction to build
it, not a review of the design, so
[ADR-0032](../../adr/0032-a-rail-issued-reference-is-committed-before-the-prompt.md)
is **Proposed**, and every decision in it is the implementing agent's:

- the port gains an optional method and a capability, rather than the adapter
  persisting anything (D1);
- no `submit` row is the proof nobody was prompted, rather than a new column
  or marker (D3);
- a `409` with no `submit` row behind it fails the charge as
  `provider_unavailable` through the declined path (D4);
- the `prepare` attempt row carries the number of the `submit` it precedes;
- a lost process answer is resolved by the read and is not re-processed.

## What was built

| Piece                                                                                                                                                                                                                                                                           | Where                                                       |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| `SubmitSteps { One, PrepareThenSubmit }`, `Capabilities::submit_steps`, `PriorSubmits { None, Some }`; `ProviderAdapter::prepare`, defaulting to `Unsupported`; a `prepare` column in the port's error table                                                                    | `vpay-provider/src/lib.rs`                                  |
| `Measured` forwards and counts `prepare` (operation `prepare`)                                                                                                                                                                                                                  | `vpay-provider/src/measured.rs`, `vpay-core/src/metrics.rs` |
| `Charges::record_prepared`: `provider_ref_extra` merged on a `submitting` charge, `WriteMatchedNoRow` otherwise                                                                                                                                                                 | `vpay-db/src/charges.rs` (`sql_audit` assert sites 74 → 75) |
| `provider_requests.operation` admits `prepare`                                                                                                                                                                                                                                  | `backends/migrations/0052_provider-requests-prepare.sql`    |
| the confirm path's step 3b: `prepare_on_rail`, then `open_attempt` (the `submit` row), then `submit_to_rail`; `refuse_confirm` shared by both refusals                                                                                                                          | `vpay-api/src/v1/payment_intents.rs`                        |
| the resubmit: `next_submit_attempt` returns `PriorSubmits`; `needs_prepare`; `prepare_again`                                                                                                                                                                                    | `vpay-worker/src/handlers.rs`                               |
| `notchpay`: `PrepareThenSubmit`; `prepare` is initialize and prompts nobody; `submit` is process by the held `trx.…`, `NeedsReconciliation` without one; initialize `409` is `Rejected { provider_unavailable }` under `PriorSubmits::None`, `NeedsReconciliation` under `Some` | `vpay-adapter-notchpay/src/lib.rs`                          |

`mtn_momo` and `orange_money` declare `One`. Their confirm and resubmit write
the same rows and make the same calls as before.

## How it is proven

**Unit.** `vpay-adapter-notchpay` (54 cases): `prepare` returns Notch Pay's
reference and sends no process; `submit` is addressed by the held reference
and never initializes; `prepare` of a charge holding the reference sends
nothing; the `409` under each `PriorSubmits`; a repeated process is read, not
re-prompted. `vpay-provider`: the default `prepare` is `Unsupported` for both
`PriorSubmits`; `SubmitSteps` spells itself in snake case; `Measured` counts
`prepare`.

**Conformance** (`adapter_conformance.rs`). Every submit case goes through
`Rail::submit_as_the_core`, which calls `prepare` first on a two-step rail.
The census `prepare_is_unsupported_exactly_on_a_one_step_rail` holds the
method to the capability on all three rails. Notch Pay's duplicate case
became two: `PriorSubmits::Some` is `NeedsReconciliation`, and `None` is
`Rejected { provider_unavailable }` with exactly one request in the stub's
journal.

**Integration, real Postgres and WireMock's Notch Pay.** Every case asserts
the stub's request journal through its admin API, so "never prompted" is
measured as "no process call", not inferred.

| Case                                                                                       | Staged                                                            | Asserted                                                                                                                          |
| ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `worker_recovery.rs` `a_prepare_whose_answer_was_lost_fails_the_charge_and_never_prompts`  | a `prepare` row unanswered, no `trx.…`, no `submit` row           | charge `failed` `provider_unavailable`, intent back to the payer, one event, no `submit` row; journal 1 initialize, **0 process** |
| `…a_prepared_charge_whose_prompt_never_left_is_prompted_without_initializing_again`        | `trx.…` held, no `submit` row                                     | `submitted`; journal 0 initialize, 1 process by the held reference                                                                |
| `…a_prepared_charge_whose_prompt_lost_its_answer_is_read_by_the_reference_it_holds`        | `trx.…` held, an unanswered `submit` row                          | `succeeded` by the poll; journal 0 initialize, 0 process, 1 read                                                                  |
| `…a_never_sent_two_step_charge_is_prepared_committed_and_then_prompted`                    | no attempt row                                                    | `submitted`, `trx.…` committed, rows `prepare` then `submit`; journal 1 and 1                                                     |
| `…a_one_call_charge_whose_resubmit_lost_its_answer_is_escalated_and_never_prompted`        | a one-call `submit` row unanswered, no `trx.…`, a queued resubmit | `unresolved`, alert, the `prepare` row unanswered `charge_needs_reconciliation`; re-runs send nothing; journal 1, 0, 0            |
| `…a_one_call_charge_whose_submit_lost_its_answer_is_escalated_by_its_poll`                 | a one-call `submit` row unanswered, no `trx.…`                    | `unresolved` by the poll, no `prepare` row, no resubmit; journal 0, 0, 0                                                          |
| `confirm_rails.rs` `a_two_step_confirm_prepares_commits_and_then_prompts`                  | a confirm over HTTP                                               | `processing`, `trx.test_{reference}` held, rows `prepare` then `submit`, both answered; journal 1 initialize, 1 process           |
| `confirm_rails.rs` `a_two_step_confirm_whose_prompt_fails_still_holds_the_rails_reference` | process forced to `503`                                           | `502` `provider_unavailable`; charge `submitting` **holding** its `trx.…`; `submit` row unanswered                                |

The first `worker_recovery.rs` case replaces #313's
`a_duplicate_no_submit_can_resolve_is_unresolved_by_its_resubmit_and_never_resent`,
whose staging (no `submit` row, a `409`) now fails the charge cleanly instead
of escalating it. The escalation it pinned is the fifth case's, for a charge
a one-call build submitted.

**Mutations.** Each guard was broken in the committed source, its case run,
and the file restored with `git checkout`. All eight failed their case:

| Mutation                                                         | Case that failed                                                                   |
| ---------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| worker: a recorded `submit` read as `PriorSubmits::None`         | `a_one_call_charge_whose_resubmit_lost_its_answer_is_escalated_and_never_prompted` |
| worker: `needs_prepare` ignores a held reference                 | `a_prepared_charge_whose_prompt_never_left_is_prompted_without_initializing_again` |
| API: `prepare_on_rail` does not commit the reference             | `a_two_step_confirm_whose_prompt_fails_still_holds_the_rails_reference`            |
| worker: `prepare_again` does not commit the reference            | `a_never_sent_two_step_charge_is_prepared_committed_and_then_prompted`             |
| adapter: a `409` is `NeedsReconciliation` whatever was submitted | `a_prepare_whose_answer_was_lost_fails_the_charge_and_never_prompts`               |
| adapter: a `409` is a clean decline whatever was submitted       | `a_one_call_charge_whose_resubmit_lost_its_answer_is_escalated_and_never_prompted` |
| API: the `prepare` row written as a `submit` row                 | `a_two_step_confirm_prepares_commits_and_then_prompts`                             |
| worker: the `prepare` row written as a `submit` row              | `a_prepare_whose_answer_was_lost_fails_the_charge_and_never_prompts`               |

## Gates

Run on this branch from a Claude Code cloud session, on the code as committed
at `0d3717a2` (the docs commits after it change no Rust). Postgres 16, WireMock
and Keycloak ran in Docker, shared with three other sessions' suites.
`cargo test` replaced nextest for the runs; `just verify-ignored` used nextest
to list.

| Check                                                                        | Result                                                                                                                                                       |
| ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `cargo fmt --all --check`                                                    | clean                                                                                                                                                        |
| `cargo clippy --workspace --all-targets -- -D warnings`                      | clean                                                                                                                                                        |
| `cargo test --workspace --lib --bins --tests` (integration, conformance out) | **1897 passed, 0 failed, 0 ignored**. The needs-reconciliation page measured 1891; the six are `vpay-adapter-notchpay` +4, `vpay-provider` +1, `Measured` +1 |
| `cargo test --doc --workspace`                                               | **132 passed, 0 failed, 1 ignored**, unchanged (the ignored one is `sdks/rust/src/lib.rs`'s `ReadmeDoctests`)                                                |
| `cargo test -p vpay-tests-conformance`                                       | **101 passed, 0 failed, 0 ignored** (97 before: the census's three rail cases, and Notch Pay's duplicate case split in two)                                  |
| `cargo test -p vpay-tests-integration --no-fail-fast`                        | **401 passed, 4 failed, 0 ignored** of 405 cases (398 before: five net in `worker_recovery.rs`, two in `confirm_rails.rs`). All four pass alone (below)      |
| `just verify`                                                                | ok, all fifteen: `verify-links` 2720 links, `verify-doc-counts` 13 counts, `verify-migrations` 52 files, `verify-errors` 20 error types, `schema OK`         |
| `just verify-ignored`                                                        | 0 ignored, **54** test binaries (expected 54: no binary added), 2403 total                                                                                   |
| vpay-skills `node tools/verify-coverage.mjs` against this tree               | ok: 21 skills, 265 vpay paths, 48 feature pages                                                                                                              |
| `prettier@3.9.6 --check` over the changed Markdown, both repositories        | clean                                                                                                                                                        |

`verify-citations` was not run: it asks GitHub, and is not part of `just ci`.

**The four integration failures, and why none is this change's.** Three
assert exact counts on the process-wide Prometheus recorder, which `cargo
test` shares between a binary's cases and nextest, which CI runs, does not:
`webhooks.rs`'s `the_ladder_walks_delivery_delay_and_then_succeeds` and
`worker_e2e.rs`'s `a_confirmed_payment_is_driven_to_succeeded_and_the_merchant_sees_it`
and `a_decline_after_submission_returns_the_intent_to_requires_payment_method`.
They are three of the four the
[2026-10-22 page](2026-10-22-rfc-0013-callback-registration.md) names for that reason. Each
passed run alone with `--exact`. The fourth, `staff_sso_keycloak.rs`'s only
case, hit its 60 s container-start timeout while three other sessions'
suites shared the Docker daemon; run alone it passed in 47.8 s.

## Found in review: the declared codes omitted `provider_unavailable`

Review of [#345](https://github.com/vaam-apps/vpay/pull/345) found that
`prepare`'s `409` arm with `PriorSubmits::None` produces
`Rejected { provider_unavailable }`, while
`vpay_adapter_notchpay::PRODUCED_FAILURE_CODES` still declared four codes. No
gate failed: `the_declines_prove_every_code_each_rail_can_produce` compares
the decline table's rows against the declaration, and the `409` has no row,
so both sides were short by the same code. The declaration is five codes now;
the mapping's own unit test and the conformance gate each add
`provider_unavailable` by hand, as they already add
`provider_account_blocked`, naming the case that drives it
(`a_duplicate_no_submit_ever_prompted_for_fails_the_charge_cleanly` and
`notchpay_a_duplicate_no_submit_prompted_for_is_a_clean_decline`).
`docs/flows/failures.md` and `adapter-notchpay.md` list it.

Measured: `cargo test -p vpay-adapter-notchpay --lib`, **54 passed, 0 failed,
0 ignored**; the conformance gate and both `409` cases, **5 passed**. With
`ProviderUnavailable` removed from the declaration again, both
`the_declared_codes_are_exactly_what_the_mapping_can_produce` and
`the_declines_prove_every_code_each_rail_can_produce::case_3_notchpay` failed;
restored.

## Not done

- **The split has not met Notch Pay.** No sandbox run has sent `prepare` and
  `submit` as separate port calls with a commit between them. The wire is the
  same two requests step 5 and B3b sent, but whether Notch Pay keeps a
  never-processed payment `pending` until its expiry, and answers a lost
  initialize's retry with the `409` the stub answers, are not observed.
- **A lost process answer is not re-processed.** A charge holding its `trx.…`
  beside an unanswered `submit` row is read. If the process never reached
  Notch Pay, the payer is never prompted, and the charge waits for Notch
  Pay's expiry and fails `payer_timeout`. Re-processing from a poll stays
  refused (RFC-0012 § 3).
- **The orphan a lost initialize leaves is not cancelled.** It expires on
  Notch Pay. Until then the merchant's public key could process it, which
  is ADR-0031's accepted exposure.
- **The confirm path's `PriorSubmits::Some` arm is not reachable**: a first
  confirm has no `submit` row. Only the worker passes `Some`.
- **Numbers that may collide on landing.** ADR `0032` and migration `0052`
  were free on `master` at `be31abcd`; the sibling branch
  `wip/method-routing` also carries a migration `0052`. Whichever lands
  second renumbers. _(Resolved 2026-10-23: the method-type routing change
  renumbered its migration to `0053` and was rebased onto this one; see
  [its page](2026-10-23-method-type-routing.md).)_
- **No `docs/status.md` change**: no route, gate or `NotImplemented` token
  moved.
