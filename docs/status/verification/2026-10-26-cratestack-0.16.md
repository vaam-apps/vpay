# 2026-10-26 — CrateStack 0.15.0 → 0.16.0

Branch `claude/dazzling-curie-g8sxn2`, from `master` at `172a13a`. It moves
every CrateStack pin to 0.16.0 and declares no MCP. It also carries ADR-0033
and the MCP threat model, which are docs only. The area record is
[../cratestack.md](../cratestack.md) § "CrateStack 0.15.0 → 0.16.0".

## Environment

- **Machine:** a cloud container with **a Docker daemon**, started by hand
  (`dockerd`). So the testcontainers suites ran here, Postgres and WireMock
  alike, unlike the 0.12.0 → 0.15.0 bump's session.
- **CrateStack CLI:** the prebuilt `x86_64-unknown-linux-gnu` binaries from
  the v0.15.0 and v0.16.0 releases, each checked with `sha256sum -c` against
  its sidecar. The v0.16.0 sidecar digest is
  `65dd507d158b853f76b9422145c58ff4342ab33d87b7c10fef4978aaf5686be0`.
- **Build settings:** `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`
  and `CARGO_INCREMENTAL=0`. These keep `target/` inside the session's disk
  allowance; with debug info a full test build filled it.
  `CYPRESS_INSTALL_BINARY=0`, which CLAUDE.md documents. No recipe was
  changed.

## Gates, and how many

`just ci` ran in three passes on this tree. Each pass retried only from the
step the previous one stopped at; neither stop was a defect in the change.

1. **Pass 1** passed `fmt-check`, `clippy` (`--workspace --all-targets`,
   warnings denied) and `just verify` ("the fifteen gates above passed"). It
   then stopped in nextest on
   `vpay-testkit containers::tests::a_wiremock_container_serves_the_mappings_it_was_pointed_at`.
   Docker Hub answered `429 Too Many Requests` to the pull of
   `wiremock/wiremock:3.9.2`. The image was pulled by hand and the run was
   resumed.
2. **Pass 2** stopped at 2333 of 2489, on
   `postgres_smoke the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`.
   The cause was this shell's `RUST_BACKTRACE=1`; see
   [../cratestack.md](../cratestack.md) for why and for the fix.
3. **Pass 3** ran with `RUST_BACKTRACE` unset:
   - `just test-rust`: **2489 tests run: 2489 passed, 0 skipped**.
   - `just test-doc`: **138 passed, 0 failed, 1 ignored**. The ignored one
     is in `sdks/rust/src/model.rs` and predates this change.
   - `just verify-ignored`: "0 ignored (expected 0), 54 test binaries
     (expected 54), 2489 total".
   - `lint-web` and `test-web`: every package passed. checkout is 556 passed
     and 1 skipped, dashboard 456 passed and 1 skipped; both skips predate
     this change.
   - `audit-web`: no known vulnerabilities.
   - `deny`: "advisories ok, bans ok, licenses ok, sources ok".

After the test fix, the drift case was rerun **with** `RUST_BACKTRACE=1` set:
1 passed. Clippy on `vpay-tests-integration` (`--all-targets`, warnings
denied) and `cargo fmt --all -- --check` are clean.

## Schema gate

- `just check-schema` with the 0.16.0 CLI: "schemas/vpay.cstack type-checks
  under cratestack 0.16.0", with 30 model/enum declarations.
- A deliberately broken schema, a model field written `id @id`, exits 1 under
  both 0.15.0 and 0.16.0 with the same diagnostic, "invalid type reference:
  @id". So the gate still fails on a broken schema.

## What was not done

- **No MCP code.** No `mcp` feature, no route, no surface value. ADR-0033's
  steps 2 onwards are separate changes.
- **No vpay-skills change.** ADR-0033 adds no flow page, route or gate yet.
  The CrateStack pin is a toolchain-shaped pin, so `vpay-tooling` may want
  its version string moved. That is not checked here: the skills repository
  is not in this session.
