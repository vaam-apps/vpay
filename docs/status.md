# STATUS

**What actually works today.** This page is the contract behind the repo's second
rule: *never advertise a feature as done when it clearly is not.*

It is machine-checked. `cargo xtask verify-status` scans the workspace for every
`ProviderError::NotImplemented("…")` token and fails the build if one is missing
from this file. You cannot quietly ship an unimplemented path.

Last verified: 2026-08-09, `cargo nextest run --workspace` (78 passed, 3
skipped), `pnpm -r test` (10 assertions), `cargo xtask verify-status`, `cargo
deny check`, and `just verify`, all run against the working tree of the
stabilization pass described below. The Rust count moved from 64 passed / 5
skipped to 71 passed / 3 skipped in the prior pass (database schema and
migrations 0001–0005), and from 71 to **78 passed / 3 skipped in this pass**:
three more migrations landed (`0006_create-authkestra-op-tables.sql`,
`0007_create-oauth-signing-keys.sql`, `0008_create-merchant-api-keys.sql` —
see the "Authkestra OP tables", "OAuth signing keys" and "Merchant API keys"
rows below), adding six new
per-constraint tests to `backends/tests/integration/tests/postgres_smoke.rs`
plus one new test file, `backends/tests/integration/tests/authkestra_op_smoke.rs`,
whose single test,
`sqlx_op_store_round_trips_a_client_and_enforces_single_use_codes`, drives the
real `authkestra_op::sqlx_store::SqlxOpStore<Postgres>` against migration
0006: it inserts a client, calls `find_client` (proving the JSONB columns
decode through the store's own type), `store_code`, then `consume_code`
twice and asserts the second call returns `None` — the store's single-use
enforcement actually firing against this schema, not merely SQL that parses.
The remaining 3 skipped are unrelated: the adapter conformance suite's
`#[ignore]`d cases in `backends/tests/conformance/tests/adapter_conformance.rs`,
gated on rail wire calls that are still `ProviderError::NotImplemented`.

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
| Database schema / migrations (core) | ✅ | Five migrations exist in `backends/migrations/` (`0001_create-currencies.sql` … `0005_create-ledger.sql`), applied via `sqlx::migrate!` to a real `postgres:16-alpine` (testcontainers) and asserted against in `backends/tests/integration/tests/postgres_smoke.rs`: a clean migration run on an empty database, the `one_charge_per_intent` unique index, two cross-column `CHECK` constraints firing (`partial_refunds_imply_refunds` on `providers`, `no_over_refund` on `payment_intents`), a plain `amount >= 0` check, an FK violation, and an out-of-range currency exponent. Marked ✅ and not 🟡 because the claim this row makes — "the schema and migrations exist, apply cleanly, and their constraints actually fire" — is fully implemented and tested; a broken migration or a dead constraint would fail a real test. **This is narrower than "the database works."** Nothing in the application consumes this schema yet: there is no connection pool, no query/repository layer, and `--database-url` is still accepted-but-unused CLI/env plumbing (see the row above). `vpay-server` still only serves `/healthz` — no route reads or writes a row. That gap is tracked by the rows around it (CLI/env configuration, HTTP surface, dashboard auth below), not by this one, the same way "Provider port trait" being ✅ above does not imply the adapters' wire calls work |
| Authkestra OP tables (`0006_create-authkestra-op-tables.sql`) | ✅ | `CREATE SCHEMA authkestra` plus `oauth_clients`, `oauth_codes`, `oauth_refresh_tokens`, `oauth_device_codes` — a byte-faithful transcription of the `CREATE TABLE` string literal hardcoded inside `authkestra-op` `=0.3.4`'s own `SqlxOpStore::migrate()` (not a vpay design; table/column names and types are not configurable — see the migration's header comment). Proven compatible, not just transcribed correctly by eye: `backends/tests/integration/tests/authkestra_op_smoke.rs`'s `sqlx_op_store_round_trips_a_client_and_enforces_single_use_codes` drives the real `SqlxOpStore<Postgres>` against this schema end to end — inserts a client, `find_client` (JSONB columns decode through the store's own type), `store_code`, `consume_code`, and asserts a second `consume_code` of the same code returns `None`, proving the crate's single-use `UPDATE … WHERE used = FALSE` actually fires here. A second test in `postgres_smoke.rs` proves the `oauth_codes → oauth_clients` FK fires. `oauth_device_codes` is created even though vpay's login flow (PKCE only) never uses the device grant, because `SqlxOpStore` implements `DeviceCodeStore` unconditionally. **Marked ✅ for what this row claims — the DDL exists, matches the pinned crate, and is proven compatible against a real store — not for dashboard auth working.** No shipping binary uses any of this: `authkestra-op`/`authkestra-engine` are dev-dependencies of `vpay-tests-integration` only (`backends/tests/integration/Cargo.toml`); `vpay-server` and `vpay-worker-bin` depend on neither. See "Dashboard auth" below. **Coupling risk:** this migration is pinned to `authkestra-op = "=0.3.4"` (root `Cargo.toml`) and must move in lockstep with it — the crate hand-builds SQL against these exact table/column names as string literals, so nothing type-checks a mismatch. Any future version bump of `authkestra-op` requires re-reading `sqlx_store.rs`'s `migrate()` block at the new version and re-diffing against this file before assuming compatibility still holds; the migration's own header comment says the same and this is not to be treated as a routine dependency bump |
| OAuth signing keys (`0007_create-oauth-signing-keys.sql`) | 🟡 | vpay-owned table (authkestra ships no signing-key type, store, or rotation logic at any published version — confirmed by grepping `authkestra-op-0.3.4` and `authkestra-engine-0.3.4` source for `struct SigningKey`, `trait KeyStore` and `fn rotate`, with no hits). Partial unique index `one_active_signing_key` (at most one active key), `active_key_has_no_expiry`, and `expiry_after_creation` are each proven to fire by a dedicated test in `postgres_smoke.rs`. **Marked 🟡, not ✅: the private key PEM is stored in plaintext (`private_key_pem TEXT`) — encryption at rest is not implemented anywhere in this repository.** Anyone who can `SELECT` the column reads the live signing key outright; the column comment says this plainly. There is also no key-generation or rotation code at all — the table only proves its own constraints, not that a key will ever be written to it correctly |
| Merchant API keys (`0008_create-merchant-api-keys.sql`) | 🟡 | The intended `/v1` credential store for Stripe-shaped `sk_live_`/`sk_test_` keys — deliberately not routed through Authkestra (no opaque-key primitive there; its `verify_secret()` is argon2, too slow for a hot path; `client_credentials` would break Stripe SDK drop-in compatibility). Unique SHA-256 `key_digest` (only the digest is ever stored — the plaintext key is unrecoverable after creation), `key_digest_is_sha256_hex` shape check, and `revoked_after_created` are each proven to fire by a dedicated test in `postgres_smoke.rs`. **Marked 🟡, not ✅: nothing generates, hashes, verifies, or revokes a key.** There is no `/v1` authentication middleware, no key-issuance endpoint, and no code anywhere that reads this table — it is schema only |
| Dashboard auth (`/dash/v1` as an Authkestra OP) | ⛔ | Decision recorded in [ADR-0009](adr/0009-dashboard-oidc-provider.md), design in [docs/flows/dashboard-auth.md](flows/dashboard-auth.md). **Still no dashboard-auth code and no `/dash/v1` route.** `authkestra-op`/`authkestra-engine`/`authkestra-axum`/`authkestra-resource` now appear in the root `Cargo.toml` as pinned workspace dependency versions, and `authkestra-op`/`authkestra-engine` are real dev-dependencies of `vpay-tests-integration` (used only by `tests/authkestra_op_smoke.rs`, above) — but **no shipping binary depends on any of it**: `vpay-server` and `vpay-worker-bin` do not list `authkestra*` in their `Cargo.toml`s. The three new migrations (rows above) give this a real, tested schema, but a reader must not conclude login works from that — no login has ever been performed, no token has ever been issued by this code, and no key has ever been rotated. The actual blocker is unchanged from before this pass: there is still no connection pool and no query/repository layer anywhere in the workspace, so `authkestra_op::sqlx_store::SqlxOpStore` cannot be constructed by any binary that would serve traffic |
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
| `@vpay/ui` production build (`next build`) | ✅ | Was broken: relative imports used a `.js` suffix (`'./cn.js'`); `moduleResolution: "bundler"` let `tsc`/Vitest resolve that back to the `.ts` source, so both passed while Next's webpack resolver took the suffix literally and failed with `Module not found`. Suffixes were dropped from `frontends/packages/ui/src/index.ts`, `status-badge.tsx` and `payer-sheet.tsx`; `pnpm -r build` now compiles all packages including the dashboard's `next build` |
| Storybook | 🟡 | Configured with a11y addon; **only `StatusBadge` has stories** |
| `@vpay/api-client` | 🟡 | `formatAmount` done + 4 tests. **Every network call throws `NotImplementedError`** |
| Dashboard app | 🟡 | Renders a scaffold notice and a design-system smoke test. **No data, no auth, no routes** |
| `pnpm -r test` sweep | ✅ | 10 assertions total (3 `@vpay/tokens` + 4 `@vpay/api-client` + 3 `@vpay/ui`), all passing. Previously broken: `@vpay/e2e`'s `test` script ran `cypress run`, so the recursive sweep tried to launch Cypress and failed with no binary installed — `just ci` and the CI `web` job could never pass. Fixed by renaming that package's script to `e2e` (`frontends/tests/e2e/package.json`), which `pnpm -r test` no longer touches |
| Cypress e2e | 🟡 | 3 specs written against the compose stack. **Still never executed here** — now purely because the Cypress binary itself isn't installed (its CDN is unreachable from this sandbox), not because of the script-wiring bug above. Run `pnpm exec cypress install` on a machine that can reach the CDN, then `pnpm --filter @vpay/e2e run e2e` |

---

## Infrastructure

| Area | Status | Notes |
|---|---|---|
| `compose.yml` (Postgres + 2 WireMock rails) | 🟡 | Written; **still never started as a stack.** Docker itself works here — `backends/tests/integration` runs real `postgres:16-alpine` containers against it — but **Docker Hub is unreachable**, and `wiremock/wiremock:3.9.2` is not in the local image cache, so the two rail stubs cannot be pulled. Postgres is cached and does run |
| `compose.e2e.yml` (full stack) | 🟡 | Revised this pass; **still never run** — see below |
| `backends/Dockerfile` (musl → scratch) | 🟡 | Rewritten this pass; **still never built** — see below |
| `frontends/Dockerfile` | 🟡 | Rewritten this pass; **still never built** — see below |
| `deny.toml` | ✅ | `cargo deny check` passes clean: `advisories ok, bans ok, licenses ok, sources ok`. The three advisories that failed before were fixed by **upgrading dependencies, not by suppressing them** — see below. One advisory is explicitly ignored: **RUSTSEC-2023-0071** (Marvin Attack in `rsa`, no patched release, an unconditional dependency of `authkestra-engine` per [ADR-0009](adr/0009-dashboard-oidc-provider.md)), accepted deliberately with the reasoning recorded inline in `deny.toml`. **This entry was preemptive when added and now genuinely fires**: `authkestra-op`/`authkestra-engine` landed as dev-dependencies of `vpay-tests-integration` in this pass (see "Authkestra OP tables" above), and `cargo deny -L info check advisories` now reports `note[advisory-ignored]` against `rsa v0.9.10` via that path — independently re-run for this update, output confirmed. `cargo deny check` still passes with 0 errors because an `ignore`d advisory downgrades to a note, not a failure; the exposure itself is still narrower than "in production," since the only path to `rsa` is `vpay-tests-integration`'s dev-dependencies — no shipping binary pulls it in. Also bans `aws-lc-rs`/`aws-lc-sys` so a second rustls crypto provider cannot reappear |
| GitHub Actions | 🟡 | Workflow written; **never executed** |
| `schemas/*.cstack` | 🟡 | **Syntax now verified against real CrateStack 0.7.8**; content remains a design sketch, excluded from the build graph — see below. **The migrations are now the authoritative schema, and this file has diverged from them on two constraints**: raw SQL in `backends/migrations/0002_create-providers.sql` and `0003_create-payment-intents.sql` expresses two `CHECK` constraints (`partial_refunds_imply_refunds`, `no_over_refund`) that CrateStack's grammar cannot — no `@@check(expr)` exists in this version. The `.cstack` file's own `GAP` comments on those two models now point at the migrations that implement them |

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
- **Two constraints this grammar cannot express now exist in raw SQL, and the
  migrations are the authoritative schema.** The file's `GAP` comments on
  `Provider` and `PaymentIntent` explain that CrateStack's `@db_enforce` only
  promotes a single-field `@range`/`@length`/`@iso4217` validator to a
  column-level CHECK — there is no `@@check(expr)` or any other cross-column
  boolean constraint, so `supports_partial_refunds ⇒ supports_refunds` and
  the over-refund guard could never be expressed in this file. Raw SQL has no
  such limitation: `backends/migrations/0002_create-providers.sql` and
  `0003_create-payment-intents.sql` implement both as real `CHECK`
  constraints, each proven to fire by a test in
  `backends/tests/integration/tests/postgres_smoke.rs` against a real
  Postgres. `Capabilities::is_coherent` in
  `backends/crates/vpay-provider/src/lib.rs` (tested by
  `vpay-provider::tests::partial_refunds_imply_refunds`) still enforces the
  first of those in Rust too — belt and braces, not a replacement for the DB
  constraint. **This file has diverged from what it mirrors**: it is still
  syntax-verified against real CrateStack 0.7.8 and still excluded from the
  build graph (below), but on these two constraints specifically it is now a
  design sketch that the migrations have moved past, not the other way
  around — see `docs/flows/configuration.md` and `docs/flows/ledger.md` for
  the full corrections.
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

1. ~~Database schema + migrations, with the `one_charge_per_intent` unique
   index.~~ **Done.** `backends/migrations/0004_create-charges.sql:73`
   creates `one_charge_per_intent` as a plain unique index on
   `charges (payment_intent_id)`, proven to reject a second charge by
   `one_charge_per_intent_is_enforced_by_the_database` in
   `backends/tests/integration/tests/postgres_smoke.rs`. This item is about
   the schema existing and its constraints holding, not about the
   application using it — see the "Database schema / migrations (core)" row
   above for that distinction; the remaining items below are unaffected.
2. Both adapters making real HTTP calls, passing the shared conformance suite
   with the `#[ignore]`s removed.
3. The worker's job loop, poll ladder and reconciler, with crash tests.
4. `/v1/payment_intents` create + confirm, form-encoded, with idempotency.
   **The credential store this needs now exists as schema** (`merchant_api_keys`,
   migration `0008` — see the row above), but nothing generates, hashes,
   verifies, or checks a key against it yet; there is no `/v1` auth
   middleware. Schema existing does not move this item to done.
5. Signed webhooks with the two-step outbox.
6. `just test-e2e` green against the compose stack.
7. `/dash/v1` login working end to end against a real database — issuing an
   access token, verifying it on a subsequent call, and rotating a signing
   key at least once. The schema this needs now exists (migrations `0006` and
   `0007`, rows above) and is proven compatible with `SqlxOpStore`, but
   nothing in this repository has ever performed a login, issued a token, or
   rotated a key — see "Dashboard auth" above. Not part of "does this take
   payments," included here because it is the other place this pass's
   migrations land, and the same "schema ≠ working feature" caution applies.

Until every one of those is ✅, this README's own claim is: **it does not take payments.**
