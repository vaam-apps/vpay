# Issue #96 item 1 verification: a rail-accepted payment on a canceled intent is parked in `requires_review`

_Written 2026-11-17 on `impl/96-requires-review` over `master` at `ea469b4`.
Every number below was measured on the tree this page was committed with,
against real Postgres 16 containers (testcontainers, `postgres:16-alpine`).
Nothing below was skipped for want of a database._

What was built is in [docs/status/backend.md](../backend.md) (the row "A
rail-accepted payment on a canceled intent → `requires_review`"), in
[payment-lifecycle.md](../../flows/payment-lifecycle.md) § Status, and in the
new runbook [charges-requires-review.md](../../runbooks/charges-requires-review.md).
This page records what was run, what was broken on purpose, and what was not
done.

## The race, reproduced first: red on `master`

The new case `a_settlement_onto_a_canceled_intent_parks_the_charge_for_review_once`
(`backends/crates/vpay-db/tests/repositories.rs`) stages the real interleaving.
A cancel's transaction holds the intent's row, a confirm opens a charge on
another connection (it waits on the row since `0054`), the cancel commits, and
the charge commits onto the `canceled` intent. Then the rail's `succeeded` is
settled twice. Built and run against `master`'s `vpay-db` with only the test
added:

```text
running 1 test
test a_settlement_onto_a_canceled_intent_parks_the_charge_for_review_once ... FAILED

---- a_settlement_onto_a_canceled_intent_parks_the_charge_for_review_once stdout ----
Error: the settlement must commit the parking, not roll back whole

Caused by:
    no row in payment_intents matched pi_review, or it was no longer in the required state

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 138 filtered out; finished in 6.18s
```

That is the issue's sentence as a test: `WriteMatchedNoRow` on
`payment_intents`, and the whole settlement rolled back.

## Green after the change

| Command                                                                                                                                                                                              | Result                                                                                                                                         |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo test -p vpay-db --test repositories`                                                                                                                                                          | **140 passed, 0 failed, 0 ignored** (89 s, real Postgres; 139 before review added the `apply_failed` case below)                               |
| `cargo test -p vpay-db --lib` (includes `sql_audit`'s exact `AssertSqlSafe` count, 75 → 77)                                                                                                          | **150 passed, 0 failed, 0 ignored**                                                                                                            |
| `cargo test -p vpay-core` (lib + doctests)                                                                                                                                                           | lib **95 passed**, doctests **69 passed**, 0 failed, 0 ignored                                                                                 |
| `cargo test -p vpay-worker --lib`                                                                                                                                                                    | **82 passed, 0 failed, 0 ignored**                                                                                                             |
| `cargo test -p vpay-tests-integration --test postgres_smoke -- schema_migrates_cleanly_on_an_empty_database every_enum_check_0037 the_cstack_schema_drifts_from_the_migrations migration_0037_keeps` | **4 passed, 0 failed, 0 ignored**, 51 filtered out. The drift case ran `cratestack` 0.16.0 and measured **221** changes, unmoved by the change |
| `cargo clippy -p vpay-core -p vpay-db -p vpay-worker -p vpay-tests-integration --all-targets -- -D warnings`                                                                                         | clean                                                                                                                                          |
| `just check-schema`                                                                                                                                                                                  | ok: schema type-checks, and `frontends/packages/dash-client` matches it after `just gen-dash-client`                                           |
| `just test-alert-rules` (helm 3.16.1, promtool 3.13.5, the CI pins)                                                                                                                                  | **7 rules**, each with a firing and a silent case, `SUCCESS`; `VpayChargeRequiresReview` has three cases since review (below)                  |
| `just verify`                                                                                                                                                                                        | **the seventeen gates passed** (output below)                                                                                                  |

The full workspace suite and `just ci` were **not** run here (shared build
machine); CI runs them.

`a_cancel_racing_a_settlement_leaves_one_terminal_state_and_one_event` now
asserts the parked charge where it asserted the rollback.
`apply_succeeded_refuses_a_canceled_intent_and_commits_nothing` asserted the
old behaviour on a `canceled` intent. It called that pairing "genuinely
unreachable", and it is reachable. It became
`apply_succeeded_refuses_a_succeeded_intent_and_commits_nothing`: the one
status that is still a broken invariant must still roll back and must not be
parked. It failed (1 of 139) in the first full run after the change, before
it was rewritten.

## Mutations: each was applied, run, and reverted

| Mutation                                                                                                                          | What failed                                                                                                                                                                                                                                          |
| --------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| the change itself absent (master's `vpay-db`)                                                                                     | the new case: `WriteMatchedNoRow` on `payment_intents` (above)                                                                                                                                                                                       |
| `'requires_review'` added to `LIVE_CHARGE_STATES` (a parked charge stays settleable)                                              | the new case: the second delivery returned `Some(..)` and re-parked the charge, overwriting `provider_txn_id` with the duplicate's `MTN-TXN-OTHER`. That is the double-park and double-alert case                                                    |
| `record_transition` removed from `park_for_review`                                                                                | the new case: `the alert's input counts the parking exactly once across two deliveries` (no `to="requires_review"` series)                                                                                                                           |
| `VpayChargeRequiresReview`'s selector widened to `to=~"requires_review\|unresolved"`                                              | `just test-alert-rules`: the firing case at 12m got a second alert, from the `unresolved` series                                                                                                                                                     |
| review: the rule as first committed, `increase(...[1h]) > 0` alone, against a series that first appears already at 1 (`_x9 1x80`) | `just test-alert-rules`: the new case at 12m expected one alert and got none. Nothing registers the series at 0, so a pod's first parked charge was silent. Fixed with a second half that fires on a series which exists now and did not an hour ago |
| the `increase` half removed from the fixed rule                                                                                   | `just test-alert-rules`: the case for a second parking on an hour-old series (`1x69 2x30`) expected one alert at 72m and got none                                                                                                                    |

## `just verify`

```text
verify-no-mocks: ok — no test double reachable from a shipping binary
verify-status: ok — 2 unimplemented item(s), all declared in docs/status.md and all still in shipping code
verify-errors: ok — 25 error type(s), all classified; 19 `#[from]` variant(s) delegate every `Classify` method they match on; anyhow confined to binaries
verify-sdk-parity: ok — 737 proving test(s) named in docs/sdks/parity.md all exist, 42 dated gap(s), 35 SDK method(s) enumerated across 43 row(s)
verify-links: ok — 3228 repository link(s) in 536 tracked markdown file(s) resolve to a tracked path (anchors and http(s) URLs are not checked)
verify-npm-scope: ok — 3 publishable package(s) under sdks/ (@vaam-apps/vpay-sdk (sdks/nodejs/package.json), @vaam-apps/vpay-stripe-js (sdks/stripe-js/package.json), @vaam-apps/vpay-tauri-checkout (sdks/tauri/tauri-plugin-vpay-checkout/package.json)), 1 private one(s) declaring no publishConfig, and no retired package name outside docs/plans, docs/adr, docs/status.md and docs/status/
check-schema: ok — schemas/vpay.cstack type-checks under cratestack 0.16.0
check-schema: ok — frontends/packages/dash-client matches schemas/vpay.cstack
verify-serde: ok — 141 serialisable type(s) spell the workspace's wire convention, 17 exempted with a reason in docs/adr/0016-engineering-standards.md
verify-repositories: ok — 4 concrete implementation(s) in backends/crates/vpay-db (Handles, PendingTransaction, PgRepositories, SqlClientAssertionStore), named by none of the 119 source file(s) outside it, and no generated schema module is exported
verify-toolchain: ok — rust-toolchain.toml pins 1.98.0 and all 2 `FROM rust:` instruction(s) in 2 Dockerfile(s) name it (backends/Dockerfile: rust:1.98.0-alpine3.22, examples/shop/Dockerfile: rust:1.98.0-alpine3.22)
verify-migrations: ok — 59 migration file(s) in backends/migrations/ all match their entries in backends/migrations/MANIFEST.sha256
verify-versions: ok — 27 version references all say 0.16.0
verify-privacy-inventory: ok — 336 database column(s) classified in schemas/privacy-inventory.yaml across 27 elements (18 personal-data, 18 necessary), checked in both directions against 336 column(s) derived from backends/migrations; 11 non-database surface(s) registered (7 not yet statically enumerable, 7 with an unmet note, 5 element(s) name a recipient); 66 debug_protection(s) registered and checked in both directions
verify-doc-counts: ok — 9 documented count(s) in 9 of 311 markdown file(s) agree with what this tree measures
verify-security-inventory: ok — 5 service(s), 5 state store(s), 35 table(s), 11 secret class(es), 15 material control(s) and 7 tracked gap(s) verified in schemas/security-inventory.yaml
verify-mcp-parity: ok — 37 /v1 route(s), each with one row in docs/api/mcp-parity.md: 9 served by a tool, 28 dated gap(s), 0 excluded; all 9 declared tool(s) named
verify: ok — the seventeen gates above passed; the verify-docs report is advisory
```

## A trap met on the way, for whoever shares a target directory next

Every agent on the authoring machine built into one `CARGO_TARGET_DIR` from a
different worktree. Cargo's artifact names do not include the worktree's path,
and its freshness check compares modification times. So a crate another
worktree built after this one's last edit was taken as fresh, and its code was
used. Two symptoms were seen: `cargo clippy` failing on `ChargeState::RequiresReview`
not existing, and `just verify` reporting **58** migrations, because the
`xtask` binary had been built in another checkout and `repo_root()` is
`env!("CARGO_MANIFEST_DIR")`. Touching the edited sources forced a rebuild
from this tree. Every result above is from a run that compiled this worktree's
crates (the `Compiling … (/…/agent-a3670f9a914881f56/…)` lines were checked).

## What was not done

- **The operator action.** Neither "refund through the rail" nor "mark
  reconciled" exists, so a parked charge stays parked. This is #96's open
  remainder.
- **A dashboard listing.** `/dash/v1` has no charges list and no
  charge-state filter, so there was nothing to add `requires_review` to.
  Recorded in [dashboard/status-built-and-not-built.md](../../flows/dashboard/status-built-and-not-built.md).
- **A merchant event.** None is emitted. No documented type fits, and the
  events vocabulary was not reopened.
- **`apply_failed` on a canceled intent.** A rail decline for a charge on a
  canceled intent still rolls back with `WriteMatchedNoRow` and dead-letters.
  No money moved there, so it is outside #96's scope.
  Pinned since review by `apply_failed_onto_a_canceled_intent_still_rolls_back_and_pages`
  (`WriteMatchedNoRow` on `payment_intents`, `Internal`/`Page`/`Retry::Never`,
  the charge still `submitted`, one `payment_intent.canceled`), so changing it
  has to be deliberate.
