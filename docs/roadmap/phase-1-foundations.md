# Roadmap — Phase 1: Foundations

_Moved out of [docs/roadmap.md](../roadmap.md) on 2026-09-11 by exp57, which split a 1 504-line page into one page per phase. **The text below is the original, unedited** — every struck-through claim, every "Open —" maintainer question and every dated amendment is here as it was written, because on this page those are most of the content; only the relative links gained a `../` because the file moved one directory down._

## Phase 1 — Foundations

**Goal.** The binaries boot, self-check, connect to a real database, and
load reviewed configuration before serving anything.

**Status.** Complete. Seven commits on `master`: `61e404b` (scaffold) →
`237c716` (#1, CLI/env config) → `9e92d02` (#2, kebab-case rename) →
`879e2cb` (#3, schema/migrations/auth storage) → `286d75a` (#4,
signal-handler race fix) → `3d7635a` (#5, CrateStack re-verify) →
`932d8a4` (#6, YAML config + Postgres connectivity).

**Scope.**

- Workspace on edition 2024/resolver 3, lint policy enforced (`unwrap`/
  `expect`/`panic`/`todo`/float arithmetic denied, `unsafe` forbidden —
  [ADR-0007](../adr/0007-lint-policy.md)).
- `clap` CLI on both binaries, every option resolving from an env var with
  an explicit flag winning (`vpay-config::cli`).
- Graceful shutdown on SIGINT/SIGTERM, including a fix for a real startup
  race where a SIGTERM delivered before the first signal-future poll bypassed
  shutdown entirely (`vpay_config::signal::ShutdownSignals`).
- All 12 Postgres migrations (`backends/migrations/0001`–`0012`), applied
  and constraint-tested against a real database. _This is Phase 1's scope,
  not the repository total: there are **20** migrations as of 2026-09-03
  (`0013` with the authkestra upgrade, `0014`–`0018` with Step 2, of which
  `0017` and `0018` are schema only, and `0019`–`0020` with Step 3). See the
  Phase 3 addenda._ **Corrected 2026-09-03 (evening): that "20" was true when
  it was written and is now wrong — `ls backends/migrations | wc -l` answers 26.** `0021`–`0024` came with the worker and the webhook outbox (Steps 4 and
  5), `0025` with the Stripe-SDK retry advice (Step 5b) and `0026` with
  `client_secret` (Step 5c). Phase 1's own scope is still the first twelve.
- YAML configuration loading (`vpay_config::Config::load`: Figment layers +
  hand-rolled `${ENV}` resolution + `garde` validation), wired into both
  binaries as a hard startup requirement — [ADR-0003](../adr/0003-yaml-configuration.md).
- `vpay-db`'s connect/migrate/healthcheck, wired into both binaries.
- `schemas/vpay.cstack` re-verified against real CrateStack 0.7.10 grammar —
  a design sketch, excluded from the build graph, not part of this phase's
  runtime scope.

**Definition of done (met).** `cargo xtask verify-status` and
`cargo xtask verify-no-mocks` pass; both binaries exit non-zero, naming the
problem, on a missing or invalid `--config`/`--database-url`
(`a_missing_config_is_exit_78_naming_the_problem` and siblings in
each binary's `tests/cli.rs`); `/healthz` returns 200 or 503 based on a real
`SELECT 1`, not a static string; all migrations apply cleanly and
idempotently with their constraints proven to fire in
`backends/tests/integration/tests/postgres_smoke.rs` — 12 when this phase
closed, **26** as of 2026-09-03 (evening; this line said 20 until then —
`schema_migrates_cleanly_on_an_empty_database` applies whatever is in
`backends/migrations/`, so the count is not pinned here and a stale number in
this document never failed a build).

**Unblocks.** Everything below — no later phase can start without a durable
process and a database.

**Risks and open questions carried by this phase.**

- MSRV `1.88` (`Cargo.toml`) is derived only from dependency metadata
  (`cargo metadata`'s max declared `rust_version`), never actually compiled
  against — the only compiler that has ever built this workspace is whatever
  `rust-toolchain.toml` pinned at the time (1.95.0 until 2026-09-05, 1.98.0
  since). Re-derived on 2026-09-05 during that bump and unchanged at 1.88;
  **135 of 477** graph packages declare no `rust_version` at all (the numbers
  here read "63 of 317" until then, which was the graph as it stood on
  2026-09-02), so the true floor could be higher
  (`rust-toolchain.toml`'s own header comment).
- `.config/nextest.toml` sets `max-threads = 1` for every Postgres-backed
  integration test, tuned against a 4-vCPU Docker Desktop VM allocation on
  the authoring machine, not the host's own CPU count. Whether this should
  relax on a real CI runner with a larger Docker allocation is open — the
  file's own comment already flags the wall-clock trade-off.
- `ProviderHost::settings` and `::credentials` are both plain
  `BTreeMap<String, String>`; only `credentials` is redacted in `Debug`
  output. A value placed in the wrong map leaks in plaintext, and nothing —
  no test, no type — catches that misclassification (`docs/status.md`,
  "Secret redaction" row).

---
