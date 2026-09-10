# exp42 (haiku) boot coherence implementation

## Task
Boot should refuse an incoherent adapter (one where partial_refunds=true but refunds=false) as a configuration error (exit 78) before the CHECK constraint fires.

## Key files
- `backends/crates/vpay-config/src/lib.rs` - add ConfigError variant
- `backends/crates/vpay-api/src/v1/boot.rs` - add is_coherent check in boot_seeds
- `backends/apps/vpay-server/tests/cli.rs` - add boot test for incoherent capabilities
- `docs/flows/configuration.md` - add row to boot-guard table
- `docs/status.md` - update date

## Approach
1. Add `IncoherentCapabilities` variant to ConfigError enum
2. Call `Capabilities::is_coherent()` in boot_seeds after getting adapter but before seeding
3. Write cli.rs test with incoherent YAML fixture
4. Update docs

## Status
Starting implementation.
