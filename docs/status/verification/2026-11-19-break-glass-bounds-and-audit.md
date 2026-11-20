# Issue #152 / `GAP-152-BREAK-GLASS`: break-glass console keys are bounded and audited

_Written 2026-11-19 on `chore/152-break-glass-and-rotation`, branched from
`origin/master` (`acafba2c`, v0.17.0). What changed is in
[backend.md](../backend.md) (the `staff pair-key` and console-keys rows) and
[ADR-0038](../../adr/0038-break-glass-console-keys-are-bounded-and-audited.md);
this page is what was run._

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
  `clean_operator` / `check_lifetime` refusals.
- `vpay-server staff pair-key` gains `--ttl-hours` (default 24) and
  `--operator` (required); `staff revoke-key` gains `--operator` (required).

## Evidence

Backend and gate checks, run on `acafba2c` + this branch:

| Check                                                                                      | Result                                          |
| ------------------------------------------------------------------------------------------ | ----------------------------------------------- |
| `cargo clippy --workspace --all-targets -- -D warnings`                                    | **PASS**, 0 warnings                            |
| `just fmt` / `cargo fmt --check`                                                           | **PASS**                                        |
| `cargo xtask verify-links`                                                                 | **PASS** (3364 links in 552 tracked files)      |
| `cargo xtask verify-migrations`                                                            | **PASS** (63 files match `MANIFEST.sha256`)     |
| `cargo xtask verify-serde` / `verify-repositories` / `verify-toolchain`                    | **PASS**                                        |
| `cargo xtask verify-privacy-inventory`                                                     | **PASS** (347 columns both directions)          |
| `cargo xtask verify-doc-counts`                                                            | **PASS** (migration README corrected to 63)     |
| `cargo xtask verify-security-inventory`                                                    | **PASS** (8 gaps; `GAP-152-BREAK-GLASS` closed) |
| `cargo xtask verify-mcp-parity` / `verify-adr-numbers` / `verify-status` / `verify-errors` | **PASS**                                        |

Postgres-backed tests (testcontainers, real Postgres 16), all **PASS**:

| Test                                                                                                                   | Suite                                                                                                                            |
| ---------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `a_console_key_lives_at_most_thirty_days_and_the_database_says_so`                                                     | `vpay-db` `repositories.rs` — ordinary pairing 720 h, a 24 h break-glass row stored, 744 h/0 h/-1 h refused by the new CHECK     |
| `a_console_key_is_live_until_it_expires_or_is_revoked`                                                                 | `vpay-db` `repositories.rs`                                                                                                      |
| `staff_pair_key_and_revoke_key_journal_the_operator`                                                                   | `vpay-server` `cli.rs` — 2-hour `--ttl-hours` key, `console_key:pair` and `console_key:revoke` rows with `actor_kind = operator` |
| `staff_pair_key_attaches_a_key_prints_one_line_and_is_idempotent`                                                      | `vpay-server` `cli.rs` — now asserts the **24-hour** default                                                                     |
| `staff_revoke_key_revokes_an_administrators_key_prints_one_line_and_is_idempotent` and the rest of the `staff_*` cases | `vpay-server` `cli.rs` — 13/13                                                                                                   |
| `a_key_expires_thirty_days_after_it_was_paired` (vaam pairing unchanged)                                               | `vpay-tests-integration` `dashboard_read_surface.rs`                                                                             |
| `console_pairing.rs` (11 tests)                                                                                        | `vpay-tests-integration`                                                                                                         |
| `schema_migrates_cleanly_on_an_empty_database` (now pins 63 migrations)                                                | `vpay-tests-integration` `postgres_smoke.rs`                                                                                     |

## What was NOT run

- **`check-schema` and the CrateStack drift test** need the external
  `cratestack` CLI, which is not installed in this environment (`mise` reports
  no version set). The migration adds no table, column or index — it swaps one
  two-column CHECK for another and four single-column CHECKs for their widened
  twins — so `EXPECTED_DRIFT_CHANGES` is unchanged; the multi-column list in
  `postgres_smoke.rs` carries the one renamed entry. CI, which installs the pin
  exactly, is the gate of record.
- **`just ci` end to end** (its `ci` recipe needs the network for `audit-web`
  and the `cratestack` CLI for `check-schema`). Every other `verify` gate and
  the targeted tests above were run individually.
- **No device, no browser, no cluster.** This is a command-line and database
  change; nothing binds a listener.
- **No dual-control approval.** The gap also named "no second person"; that
  organisational control is left open (ADR-0038 § What this does not do).
