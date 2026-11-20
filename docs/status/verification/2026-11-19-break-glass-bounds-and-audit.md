# Issue #152 / `GAP-152-BREAK-GLASS`: break-glass console keys are bounded and journalled

_Written 2026-11-19 on `chore/152-break-glass-and-rotation`, branched from
`origin/master` (`acafba2c`, v0.17.0). What changed is in
[backend.md](../backend.md) (the `staff pair-key` and console-keys rows) and
[ADR-0038](../../adr/0038-break-glass-console-keys-are-bounded-and-audited.md);
this page is what was run. Amended 2026-11-20 after an adversarial review of PR
#411: ten findings fixed, each new test shown to fail under a mutation (below)._

## What changed

- Migration `0063_break-glass-bounds-and-audit.sql`: replaces the console-key
  life CHECK `console_keys_lives_exactly_thirty_days` with
  `console_keys_lives_at_most_thirty_days` (a row must satisfy both
  `paired_at < expires_at` and `expires_at <= paired_at + 720 hours`), and
  widens the `audit_events` vocabulary (`actor_kind` `operator`, `action`
  `console_key:pair` / `console_key:revoke`, `outcome` `paired` / `revoked`,
  `target_class` `console_key`).
- `vpay_db::NewConsoleKey` carries the `lifetime` the caller wants;
  `BREAK_GLASS_KEY_LIFETIME` is 24 hours. `TxRepositories` gains
  `revoke_console_key_in_tx`.
- `vpay_api::console_keys::{pair_from_operator, revoke_from_operator}` write the
  key and the `audit_events` row in **one transaction**, with
  `clean_operator` / `check_lifetime` refusals. The vaam paths store through
  `store_vaam_key`, which takes **no lifetime** (exactly 720 hours).
- `vpay-server staff pair-key` gains `--ttl-hours` (default 24) and
  `--operator` (required); `staff revoke-key` gains `--operator` (required).
- What it does **not** do (residual risk, `GAP-152-DUAL-CONTROL`): `--operator`
  is self-declared text, not an authenticated identity; refused attempts are not
  journalled; direct SQL can pair a key with no journal row; no second approver.

## Review findings and the mutation that proves each new test

Each mutation was applied by hand to a copy-backed file, the named test run and
seen to fail, and the file restored (`diff` against the saved good copy).

| #   | Finding                                                   | Test                                                                                                                                                                                                             | Mutation, and the failure it produced                                                                                                                                                                                                                                          |
| --- | --------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1   | Pair/revoke and their journal row commit together         | `console_pairing.rs` `an_operator_pair_and_revoke_commit_with_their_journal_row_or_not_at_all` (a trigger raises on an `audit_events` insert for a marker operator; control calls with another operator succeed) | Pair and journal in two separate transactions: fails at `console_pairing.rs:473` ("no console_keys row survives a pairing whose journal row failed"). Revoke and journal separately: fails at `:508` (`revoked_at` is `Some(..)`, expected `None`).                            |
| 2   | Validation precedes the database                          | `cli.rs` `staff_break_glass_commands_refuse_bad_input_before_the_database_is_opened` (`DATABASE_URL=postgres://vpay@127.0.0.1:1/none`, exit `1`, message names the flag, no "Postgres")                          | `--ttl-hours` check moved after `open_migrated_database`: exit `69` "connecting to Postgres" on the `ttl 0` case. CLI `clean_operator` calls removed: exit `69` on the empty-operator case; with only the `revoke-key` call removed, fails on `revoke-key, an empty operator`. |
| 3   | 720 h upper bound pinned in SQL                           | `repositories.rs` `a_console_key_lives_at_most_thirty_days_and_the_database_says_so` (720 h and 696 h accepted; 720 h + 1 us, 744 h, 0, -1 h refused)                                                            | `'720 hours'` to `'721 hours'` in `0063`: fails ("a life above 720 hours or at or below zero is refused by the table: PgQueryResult { rows_affected: 1 }"). Migration restored; `verify-migrations` ok.                                                                        |
| 4   | Vaam pairings are exactly 720 h                           | `console_pairing.rs` `a_linked_members_second_vaam_pairing_lives_exactly_seven_hundred_twenty_hours`; `store_vaam_key` has no lifetime parameter                                                                 | A lifetime argument can no longer be passed from the linked branch (the type system refuses it). Mutating the constant inside `store_vaam_key` to `BREAK_GLASS_KEY_LIFETIME`: fails (`86400 s` vs `2592000 s`).                                                                |
| 8   | `clean_operator` refuses control and invisible characters | `console_keys.rs` unit test `an_operator_is_counted_in_characters_and_carries_nothing_invisible`                                                                                                                 | `chars().count()` to `len()`: fails on 128 x `é`. Removing the `is_invisible_or_control` check: fails on the first refusal case.                                                                                                                                               |
| 9   | Repeat `pair-key` logs the real expiry                    | `cli.rs` `staff_pair_key_and_revoke_key_journal_the_operator` (asserts `created:false` and the key's `expires_at` in the repeat's log line, no `ttl_hours`)                                                      | Log `ttl_hours` instead of `expires_at`: fails ("a repeat pairing logs the live key's real expiry").                                                                                                                                                                           |
| 10  | Sub-microsecond lifetimes refused                         | `console_keys.rs` unit test `a_lifetime_of_zero_or_over_thirty_days_is_refused`                                                                                                                                  | Microsecond check disabled: fails on `nanoseconds(1_500)`.                                                                                                                                                                                                                     |

Findings 5, 6 and 7 are documentation: the privacy inventory now classifies
`audit_events.actor_id` as staff data (`audit_actor_identity`, no retention or
erasure handler recorded as such); the operator name is described everywhere as
self-declared and unauthenticated; `A.8.2` is `partial` and
`GAP-152-DUAL-CONTROL` carries the residual risk; ADR-0038 no longer claims an
owner ruling (see its Deciders line).

## Evidence

Run on this branch on 2026-11-20 (macOS):

| Check                                                                                                                                           | Result                                                                                    |
| ----------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `cargo clippy --workspace --all-targets -- -D warnings`                                                                                         | **PASS**, 0 warnings                                                                      |
| `cargo fmt --all -- --check`; prettier 3.9.6 over the changed `.md`/`.yaml`                                                                     | **PASS**                                                                                  |
| `verify-links`, `verify-migrations` (63 files), `verify-serde`, `verify-repositories`, `verify-toolchain`                                       | **PASS**                                                                                  |
| `verify-privacy-inventory`                                                                                                                      | **PASS** (347 columns, 30 elements, both directions)                                      |
| `verify-doc-counts`                                                                                                                             | **PASS** (9 counts)                                                                       |
| `verify-security-inventory`                                                                                                                     | **PASS** (15 controls, 9 gaps: `GAP-152-BREAK-GLASS` closed, `GAP-152-DUAL-CONTROL` open) |
| `verify-mcp-parity`, `verify-adr-numbers`, `verify-status`, `verify-errors`, `verify-sdk-parity`, `verify-ui`, `verify-versions`, `verify-docs` | **PASS**                                                                                  |
| `just verify-ignored`                                                                                                                           | **PASS** (0 ignored, 59 test binaries, 2807 tests)                                        |

Tests (Postgres-backed ones use testcontainers, real Postgres 16):

| Suite                                                  | Result                                                                                                                                                |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `vpay-api` lib `console_keys` (11) and doctests        | **PASS**                                                                                                                                              |
| `vpay-tests-integration` `console_pairing` (13)        | **PASS**                                                                                                                                              |
| `vpay-tests-integration` `dashboard_read_surface` (20) | **PASS**                                                                                                                                              |
| `vpay-server` `cli` `staff_*` (14)                     | **PASS**                                                                                                                                              |
| `vpay-db` `repositories` (145)                         | 144 pass; `pending_due_returns_the_deliveries_nothing_is_driving` failed once under full-suite load and **passes alone** (not touched by this change) |
| `vpay-tests-integration` `postgres_smoke` (55)         | 54 pass; `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` fails because the `cratestack` CLI is not installed                      |

## What was NOT run

- **`check-schema` and the CrateStack drift test** need the external
  `cratestack` CLI (0.20.4), which is not installed here, so `just verify` as a
  single command stops at `check-schema`; every other gate in its list was run
  individually (above). The migration adds no table, column or index, so
  `EXPECTED_DRIFT_CHANGES` is unchanged. CI, which installs the pin, is the gate
  of record.
- **`just ci` end to end**, **`just fmt`'s pnpm step** (no `node_modules` here;
  prettier 3.9.6 was run through `npx` over the changed files instead).
- **The four `staff_sign_in` tests** (they bind 127.0.0.2+, impossible on macOS).
- **No device, no browser, no cluster.**
- **No dual-control approval, authenticated operator identity or journalling of
  refused attempts**: open as `GAP-152-DUAL-CONTROL`.
