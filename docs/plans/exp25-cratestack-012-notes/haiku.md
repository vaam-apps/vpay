# exp25 (haiku): CrateStack 0.11.1 → 0.12.0

## Summary

Bumped CrateStack from 0.11.1 (2026-09-03) to 0.12.0 (latest), including the CLI, the `cratestack-pg` and all twelve `cratestack-*` workspace dependencies, and the CI action pins.

## Breaking change in 0.12.0

The key breaking change in 0.12.0 is:
- **feat(parser,cli)!: give SchemaError file identity so diagnostics can span files**

This means `SchemaError` diagnostics may now print file:line information. The justfile's `check-schema` recipe handles this transparently — no adjustment was needed.

## Changes made

1. **justfile**: `cratestack_version` bumped from `"0.11.1"` to `"0.12.0"`
2. **Cargo.toml**: `cratestack-pg` version bumped from `"=0.11.1"` to `"=0.12.0"`
3. **Cargo.lock**: all twelve `cratestack-*` crates updated to 0.12.0 via `cargo update -p cratestack-pg --precise 0.12.0`
4. **.github/workflows/ci.yml**: 
   - Action pins updated from commit `6b3053fa77924f5162915d594d457d3eda51afaa` (v0.11.1) to commit `0823bab382425e1fe4d04c42b9657b7e7bb7b286` (v0.12.0)
   - Both occurrences (lines 150 and 264) and their version comments updated

## Verification results

### Schema check (check-schema)
- Ran successfully under 0.12.0
- Schema file: `schemas/vpay.cstack`
- Result: `schema OK`
- Note: Declaration count increased from 12 to 15 (models added since the file was last checked)

### Drift test (postgres_smoke.rs)
- Test: `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`
- Result: **PASS** (no constants needed changing)
- `EXPECTED_DRIFT_CHANGES`: 101 (unchanged)
- `EXPECTED_DRIFTED_RELATIONS`: 16 (unchanged)
- The drift report is stable across the upgrade

### verify gates
- All ten gates passed

### deny check
- No new license violations
- No new duplicate versions
- Result: `advisories ok, bans ok, licenses ok, sources ok`

## Upstream gaps

No breaking changes observed in the four upstream capability gaps documented in docs/reference/vpay-db.md:
1. `@default` fields in create/upsert inputs — still absent at 0.12.0
2. `do_nothing()` authorising on the pool — still absent at 0.12.0
3. `from_plain_json` f64 demotion — still absent at 0.12.0
4. jsonb/int4 read-back — still absent at 0.12.0

The breaking change (file-identity in SchemaError) does not affect the schema itself or the `check-schema` gate behavior.

## Not done

- Full `just ci` end-to-end (mechanical bounds prevented in the experiment window; a review stage follows to verify the complete gate under CI's exact pins)
- Updates to docs/status.md referencing 0.11.1 as "current" — those are historical notes and intentionally not edited
- Updates to docs/reference/vpay-db.md CrateStack section — no upstream gaps closed, so no edit needed

## Commits

- `0f0aa45`: `bump(cratestack): 0.11.1 -> 0.12.0 everywhere`
