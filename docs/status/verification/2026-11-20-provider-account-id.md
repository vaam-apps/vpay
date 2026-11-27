# Issue #290 Phase A2: `charges.provider_account_id` and per-charge account resolution

_Written 2026-11-20 on `feat/290-a2-provider-account-id`, stacked on
`feat/290-a1-legal-entity` (PR #422). Implements RFC-0009 §§ 3, 5 (Delivery step
A2)._

## What changed

- Added migration `0064_charges-provider-account-id.sql` (backfills existing
  rows to `'default'`, then drops the column `DEFAULT` so every new `INSERT` must
  write `provider_account_id` explicitly), updated `MANIFEST.sha256`, and
  declared `provider_account_id String` in `schemas/vpay.cstack`.
- Moved `DEFAULT_PROVIDER_ACCOUNT_ID = "default"` into `vpay-core`
  (`vpay_worker::callbacks::DEFAULT_ACCOUNT` re-exports it).
- Added `provider_account_id` to `NewCharge`, `ChargeRow`, and `insert_for_intent`
  in `backends/crates/vpay-db/src/charges.rs`; updated `open_charge` in
  `vpay-api` and test seeds to write it explicitly.
- Added `vpay_provider::resolve_account_config` and
  `RailConfig::provider_config_for_account` and wired them into `submit`,
  `query_status` (worker), and `refund`, returning `ProviderError::Config` on
  any unconfigured `provider_account_id` without fallback.
- Classified `charges.provider_account_id` under `sys_config` in
  `schemas/privacy-inventory.yaml`.
- Added Phase A container test
  `two_merchants_of_one_legal_entity_record_default_provider_account_id` in
  `backends/tests/integration/tests/confirm_rails.rs`.
- Updated `docs/status/backend.md` and `docs/flows/payment-lifecycle.md`.

## Verification

- `cargo test -p vpay-provider` (includes `resolve_account_config_refuses_unknown_account_without_fallback`)
- `cargo test -p vpay-tests-integration --test confirm_rails two_merchants_of_one_legal_entity_record_default_provider_account_id`
- `just verify-migrations`, `just verify-links`, `just verify-doc-counts`, `just verify-status`

## Not done here

- `just check-schema` (CrateStack CLI not installed locally; CI runs it).
- Phase B (`B0`–`B7`, multi-company accounts, sealed DB secrets, `/v1/accounts`).
- Companion `vpay-skills` PR for A2 (`vpay-data-layer` / `vpay-payments`).

## Review of 2026-11-27

A full `cargo nextest run --workspace` on the merged head found two failures
the evidence above did not run, both fixed as new commits:

- `postgres_smoke::schema_migrates_cleanly_on_an_empty_database` still
  expected 63 applied migrations; it now expects 64 and names 0064.
- `repositories::migration_0062_backfills_the_payer_mask_and_leaves_the_rest_null`
  wrote `provider_account_id` into the **pre-0062** schema, where the column
  does not exist yet; that insert is back to its original shape.

0064's claims (backfill to `default`, no column `DEFAULT` afterwards) had no
test of their own. `migration_0064_backfills_existing_charges_to_default_and_leaves_no_column_default`
applies migrations 1 to 63, writes a charge, applies 0064 alone, reads
`default` back, and requires an `INSERT` that omits the column to fail with
SQLSTATE 23502. Broken on purpose: with `DROP DEFAULT` replaced by
`SET DEFAULT 'default'` that test fails; the migration was restored and
`verify-migrations` agrees with the manifest.

`resolve_account_config_refuses_unknown_account_without_fallback` was also
broken on purpose (the account comparison disabled): it fails.
