# STATUS

**What actually works today.** This page is the contract behind the repo's second
rule: *never advertise a feature as done when it clearly is not.*

It is machine-checked. `cargo xtask verify-status` scans the workspace for every
`ProviderError::NotImplemented("…")` token and fails the build if one is missing
from this file. You cannot quietly ship an unimplemented path.

Last verified: 2026-08-09, `cargo nextest run --workspace` (61 passed, 5
skipped), `pnpm -r test` (10 assertions), `cargo xtask verify-status`, `cargo
deny check`, and `just verify`, all run against the working tree of the
stabilization pass described below.

---

## Legend

| Marker | Meaning |
|---|---|
| ✅ **Done** | Implemented, tested, and the tests actually assert behaviour |
| 🟡 **Partial** | Some of it is real. The rest is listed explicitly below |
| ⛔ **Not started** | No implementation. Calls return `NotImplemented`; tests are `#[ignore]`d with a reason |

Nothing in this repo is ✅ unless a test would fail if it broke.

---

## Overall

> **vpay is a scaffold.** It compiles, lints clean, and its tests pass — but it
> cannot take a payment. No HTTP call to any rail has ever been made by this
> code. Do not deploy it.

---

## Backend

| Area | Status | Notes |
|---|---|---|
| Workspace, edition 2024, resolver 3 | ✅ | `cargo check --workspace --all-targets` clean |
| Lint policy (no `unwrap`/`expect`/`panic`/float in prod) | ✅ | `cargo clippy -- -D warnings` clean; tests exempted via `clippy.toml` |
| `Money` — integer minor units, XAF zero-decimal | ✅ | 6 tests incl. cross-currency and over-refund rejection |
| Canonical failure taxonomy | ✅ | 3 tests |
| Charge / intent state + `ProviderFlow` | ✅ | 3 tests incl. live-xor-terminal exhaustiveness |
| Ledger balancing invariant | 🟡 | Types and `validate()` done + 3 tests. **Persistence not started** |
| Config guard rails (stub host, literal secret) | 🟡 | Rules done + 5 tests. **YAML loading and DB reconciliation not started** |
| CLI / env configuration (`vpay-config::cli`) | 🟡 | `--version` reports `0.1.0`. Every option auto-resolves from an env var with an explicit flag winning, shared between both binaries via a flattened `CommonArgs`, covered by unit tests on the built `clap::Command` plus subprocess tests that set real env vars on a child process. **`--database-url`, `--config` and `--public-base-url` are accepted and parsed but not consumed by anything** — no DB connection is opened, no YAML is read, no redirect/webhook URL is built from them. This is CLI/env plumbing only, not the ADR-0003 config system (see the row below) |
| Provider port trait | ✅ | Interface defined; both adapters implement it |
| Process lifecycle (SIGINT/SIGTERM) | ✅ | `vpay-server` now shuts down via `axum::serve(...).with_graceful_shutdown(...)` on SIGINT or SIGTERM instead of requiring `docker compose down` to SIGKILL it. `vpay-worker-bin` no longer exits immediately on boot — it stays up, answers the same signals, and logs a startup WARN banner plus a 60-second WARN heartbeat stating the job loop is not implemented and no jobs are being processed. Both are exercised by subprocess tests that send a real `SIGTERM` and assert a clean exit (`backends/apps/vpay-server/tests/cli.rs`, `backends/apps/vpay-worker-bin/tests/cli.rs`) |
| `--shutdown-grace-seconds` bounded drain | 🟡 | On `vpay-server` this is now wired in: `serve_with_bounded_drain` in `backends/apps/vpay-server/src/main.rs` races the axum drain against a `shutdown_grace_seconds`-long clock and exits non-zero if the clock wins, logging that in-flight work was cut off. **No test exercises the timeout path itself** — the existing SIGTERM tests never have in-flight work to drain, so they would pass identically with the grace clock deleted; nothing here proves the bound actually holds under load. On `vpay-worker-bin` the flag is accepted and logged ("has no effect yet") but genuinely does nothing — there is no drain to bound because there is no job loop |
| Poll ladder | 🟡 | `poll_delay` done + 3 tests. **Job loop not started** |
| HTTP surface | 🟡 | Only `/healthz` and the Stripe-shaped 404. **No `/v1/*` route exists** |
| Database schema / migrations | ⛔ | Designed in the design doc; no SQL in this repo yet |
| Webhooks (signing, outbox, delivery) | ⛔ | |
| Idempotency | ⛔ | |
| Reconciler | ⛔ | |

### Unimplemented items tracked by `verify-status`

Every token below appears verbatim in the source. Removing an item here without
removing it from the code fails CI, and vice versa.

- `mtn_momo::submit`
- `mtn_momo::query_status`
- `mtn_momo::parse_callback`
- `mtn_momo::refund`
- `orange_money::submit`
- `orange_money::query_status`
- `orange_money::parse_callback`
- `orange_money::refund`

### Adapters

| Rail | Capabilities | Wire calls |
|---|---|---|
| `mtn_momo` (push) | ✅ declared and tested | ⛔ not started |
| `orange_money` (redirect) | ✅ declared and tested | ⛔ not started |

Capabilities being real matters more than it sounds: `orange_money` declares
`supports_refunds: false`, and that flag — not a rail-specific branch — is what
makes the core refuse a refund on that rail.

---

## Frontend

| Area | Status | Notes |
|---|---|---|
| pnpm workspace, TS strict | ✅ | `pnpm -r typecheck` clean |
| `@vpay/tokens` status tokens | ✅ | 3 tests incl. "success tone belongs to `succeeded` alone" |
| `@vpay/ui` `StatusBadge` (cva + daisyUI) | ✅ | 3 tests |
| `@vpay/ui` `PayerSheet` (vaul + framer-motion) | 🟡 | Renders; **no test** — needs interaction coverage |
| `@vpay/ui` production build (`next build`) | ✅ | Was broken: relative imports used a `.js` suffix (`'./cn.js'`); `moduleResolution: "bundler"` let `tsc`/Vitest resolve that back to the `.ts` source, so both passed while Next's webpack resolver took the suffix literally and failed with `Module not found`. Suffixes were dropped from `frontends/packages/ui/src/index.ts`, `StatusBadge.tsx` and `PayerSheet.tsx`; `pnpm -r build` now compiles all packages including the dashboard's `next build` |
| Storybook | 🟡 | Configured with a11y addon; **only `StatusBadge` has stories** |
| `@vpay/api-client` | 🟡 | `formatAmount` done + 4 tests. **Every network call throws `NotImplementedError`** |
| Dashboard app | 🟡 | Renders a scaffold notice and a design-system smoke test. **No data, no auth, no routes** |
| `pnpm -r test` sweep | ✅ | 10 assertions total (3 `@vpay/tokens` + 4 `@vpay/api-client` + 3 `@vpay/ui`), all passing. Previously broken: `@vpay/e2e`'s `test` script ran `cypress run`, so the recursive sweep tried to launch Cypress and failed with no binary installed — `just ci` and the CI `web` job could never pass. Fixed by renaming that package's script to `e2e` (`frontends/tests/e2e/package.json`), which `pnpm -r test` no longer touches |
| Cypress e2e | 🟡 | 3 specs written against the compose stack. **Still never executed here** — now purely because the Cypress binary itself isn't installed (its CDN is unreachable from this sandbox), not because of the script-wiring bug above. Run `pnpm exec cypress install` on a machine that can reach the CDN, then `pnpm --filter @vpay/e2e run e2e` |

---

## Infrastructure

| Area | Status | Notes |
|---|---|---|
| `compose.yml` (Postgres + 2 WireMock rails) | 🟡 | Written; **still not started in this sandbox — Docker unavailable**, unchanged by this pass |
| `compose.e2e.yml` (full stack) | 🟡 | Revised this pass; **still never run** — see below |
| `backends/Dockerfile` (musl → scratch) | 🟡 | Rewritten this pass; **still never built** — see below |
| `frontends/Dockerfile` | 🟡 | Rewritten this pass; **still never built** — see below |
| `deny.toml` | ✅ | `cargo deny check` now run and passes clean: `advisories ok, bans ok, licenses ok, sources ok`, with `ignore = []` (no advisory was suppressed). It failed before this pass; fixed by upgrading dependencies, not by adding exceptions — see below |
| GitHub Actions | 🟡 | Workflow written; **never executed** |
| `schemas/*.cstack` | 🟡 | **Syntax now verified against real CrateStack 0.7.8**; content remains a design sketch, excluded from the build graph — see below |

### Docker / compose — rewritten, still unverified

Both Dockerfiles and `compose.e2e.yml` were rewritten in this pass:

- `backends/Dockerfile` now builds the host's implicit musl target instead of
  hardcoding `x86_64-unknown-linux-musl`, which could never have succeeded on
  an arm64 host; it pins `rust:1.95.0-alpine3.22`.
- Both runtime stages now run as non-root UID 65532.
- A new `.dockerignore` was added.

**None of it was built in this pass.** Docker Hub is unreachable from this
sandbox: `docker pull alpine:3.22` did not complete in five minutes, and
`rust:1.95.0-alpine3.22`, `node:22-alpine`, `wiremock/wiremock:3.9.2` and
`docker/dockerfile:1` are all missing from the local image cache and
unpullable here. Only `postgres:16-alpine` is cached. The rows above stay 🟡
for exactly the same reason they were 🟡 before this pass — the content
changed, the never-built status did not.

### `cargo deny` — fixed properly, not suppressed

`cargo deny check` previously failed and is now clean, without adding a single
`ignore` entry to `deny.toml` (`ignore = []`, confirmed by reading the file).
Two real dependency upgrades did the work:

- `time` 0.3.45 → 0.3.47, a production dependency (reachable from
  `vpay-core` via `sqlx`'s `time` feature, not only from `dev-dependencies`),
  addressing a `time`-crate advisory reported as RUSTSEC-2026-0009.
- `testcontainers` 0.23 → 0.27 and `testcontainers-modules` 0.11 → 0.15 (a
  test-only dependency), which moved onto `bollard` 0.20. That drops
  `rustls-pemfile` entirely (RUSTSEC-2025-0134) and replaces the unmaintained
  `tokio-tar` with the maintained `astral-tokio-tar` fork (RUSTSEC-2025-0111).
  Both advisory IDs are cited in `Cargo.toml`'s own comment next to the
  `testcontainers` pin.

`rust-version` moved `1.85` → `1.88` in the same pass, computed as the max
`rust_version` declared anywhere in the resolved dependency graph (`cargo
metadata`, including dev-dependencies). **Read the comment block at the top of
`rust-toolchain.toml` before trusting that number**: it states plainly that
1.88 has **not** been verified by actually compiling with a 1.88 toolchain —
only stable 1.95.0 was available here — and that 63 of 317 packages in the
graph declare no `rust_version` at all, so the true floor could in principle
be higher.

### CrateStack

`schemas/vpay.cstack` was rewritten in this pass against CrateStack's real
grammar (previously it was an invented, Prisma-like guess, because
`cratestack.dev/docs` 404s publicly and no authoritative reference was found).
The new file was cross-checked field by field against `cratestack-parser`'s
own test suite, and:

```
$ cratestack check --schema schemas/vpay.cstack
schema OK: schemas/vpay.cstack
```

Independently re-run for this pass (`cratestack 0.7.8`, the same version the
header cites) — output confirmed verbatim.

What this does and does not prove:

- **Syntax is verified.** Every scalar, attribute, relation and enum in the
  file parses and type-checks against the real CrateStack 0.7.8 grammar.
- **It does not prove a working migration or a running server.** The file is
  still **excluded from the build graph** — no crate depends on it, no macro
  consumes it, and `cratestack migrate diff` has never been run against a real
  vpay Postgres.
- **Content is a design sketch, not full coverage.** It models only entities
  with a real, tested Rust type to mirror: `Currency`, `Provider`,
  `PaymentIntent`, `Charge`, and the new `LedgerTransaction`/`LedgerEntry`
  pair. It deliberately omits `provider_requests`, webhooks/outbox, the job
  queue, idempotency keys and `Merchant` — none of those has a backing Rust
  struct yet, and the file's own `GAP` comments say so rather than inventing a
  plausible shape.
- **A previously-implied DB constraint does not exist in this grammar.** The
  file's `GAP` comment on `Provider` explains that CrateStack's `@db_enforce`
  only promotes a single-field `@range`/`@length`/`@iso4217` validator to a
  column-level CHECK; there is no `@@check(expr)` or any other cross-column
  boolean constraint. `supports_partial_refunds ⇒ supports_refunds` is
  therefore enforced only in Rust today, by
  `Capabilities::is_coherent` in `backends/crates/vpay-provider/src/lib.rs`
  (tested by `vpay-provider::tests::partial_refunds_imply_refunds`) — see the
  correction below and in `docs/flows/configuration.md`.
- **A structural gap surfaced by the rewrite:** `LedgerEntry.account` mirrors
  `vpay_ledger::AccountKind`, which has exactly three variants
  (`MerchantPayable`, `PayerClearing`, `PlatformFeeRevenue`) with no
  per-merchant dimension. `docs/flows/ledger.md`'s invariant 2 — "per
  merchant: `balance(merchant_payable) = Σ captures − Σ fees − Σ refunds`" —
  cannot be computed from the modelled data, because nothing says *which*
  merchant a `merchant_payable` posting belongs to. That is a real gap in the
  Rust type this schema mirrors, not something to paper over in the schema.

---

## What would have to be true to call this "an MVP"

1. Database schema + migrations, with the `one_charge_per_intent` unique index.
2. Both adapters making real HTTP calls, passing the shared conformance suite
   with the `#[ignore]`s removed.
3. The worker's job loop, poll ladder and reconciler, with crash tests.
4. `/v1/payment_intents` create + confirm, form-encoded, with idempotency.
5. Signed webhooks with the two-step outbox.
6. `just test-e2e` green against the compose stack.

Until every one of those is ✅, this README's own claim is: **it does not take payments.**
