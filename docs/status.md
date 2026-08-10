# STATUS

**What actually works today.** This page is the contract behind the repo's second
rule: *never advertise a feature as done when it clearly is not.*

It is machine-checked. `cargo xtask verify-status` scans the workspace for every
`ProviderError::NotImplemented("…")` token and fails the build if one is missing
from this file. You cannot quietly ship an unimplemented path.

Last verified: 2026-08-10, `cargo nextest run --workspace` (105 passed, 3
skipped), `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo xtask verify-status`, `just verify`, and
`cargo deny check` (`advisories ok, bans ok, licenses ok, sources ok`), all
run against the working tree of four things landing together: (1) a real YAML
config loader (`vpay_config::Config::load`, Figment + hand-rolled `${ENV}`
resolution + the existing guard rules), a library with tests but **not wired
into either binary**; (2) a new `vpay-db` crate — `connect`, `run_migrations`,
`check_connection` — that both binaries now require at boot, with `/healthz`
performing a real `SELECT 1`; (3) migrations `0009`-`0012`, a schema cutover
that drops `merchant_api_keys`, reshapes `oauth_signing_keys` to hold no
private key material, and adds `oauth_client_assertion_jtis` and
`disabled_clients`; (4) [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md),
moving `/v1` merchant auth from Stripe-shaped API keys to `client_credentials`
+ `private_key_jwt`, with README/API docs/examples corrected to say a Stripe
SDK cannot authenticate against vpay. **None of this makes authentication
work.** The router is still `/healthz` plus the Stripe-shaped 404: no `/v1/*`
route, no `/dash/v1` route, no client store, no `ClientAssertionStore`, no
kill-switch check, no signing-key generation or rotation exist anywhere in
this repository. The schema and the config loader are real and tested; the
auth is not built. See the rows below for exactly what each new piece proves
and does not prove. The Rust count moved from 80 passed / 3 skipped to **105
passed / 3 skipped in this pass**: 31 new tests, split roughly across
`vpay-config` (5 → 36, covering `Config::load`'s YAML layering, `${ENV}`
resolution, and validation), `vpay-db` (a new crate: pool construction,
migration idempotency, and both the success and failure path of the
healthcheck query, each against a real `postgres:16-alpine` via
testcontainers), `vpay-api` (router wiring), and
`backends/tests/integration/tests/postgres_smoke.rs` (six new tests proving
migrations `0009`-`0012`'s constraints — see the schema rows below). The
previous pass's own note is unchanged below, describing the state before this
one: the Rust count moved from 78 passed / 3 skipped to **80 passed / 3
skipped in this pass**: two new regression tests,
one per binary (`sigterm_immediately_after_startup_still_triggers_graceful_shutdown`
in `backends/apps/vpay-server/tests/cli.rs` and
`backends/apps/vpay-worker-bin/tests/cli.rs`), covering a real startup race
where a SIGTERM delivered immediately after process start bypassed graceful
shutdown entirely — see the "Process lifecycle" row for the mechanism, the
fix, and this pass's own honest accounting of the regression test's
statistical (not deterministic) nature and its reduced sensitivity when run
as part of the full workspace suite versus scoped/alone. Both new tests were
verified to fail against the pre-fix code before this pass landed. The prior
pass's own note is unchanged below: the Rust count moved from 64 passed / 5
skipped to 71 passed / 3 skipped (database schema and migrations 0001–0005),
then to 78 passed / 3 skipped (three more migrations —
`0006_create-authkestra-op-tables.sql`, `0007_create-oauth-signing-keys.sql`,
`0008_create-merchant-api-keys.sql` — see the "Authkestra OP tables", "OAuth
signing keys" and "Merchant API keys" rows below), adding six new
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
| Config guard rails (stub host, literal secret) | 🟡 | The two rules (`validate_host`, `validate_secret`) are unchanged and still directly unit-tested (the original 5 tests). They are now also exercised through real YAML loading: `a_livemode_config_with_an_http_host_is_rejected` and `a_livemode_config_with_a_literal_secret_is_rejected` in `vpay-config`'s `config.rs` drive them through `Config::load_with_env` against fixture files, not just as bare function calls. **DB reconciliation (boot-sequence step 4) still not started** — see "YAML config loading" below for what changed and what did not |
| YAML config loading (`vpay-config::Config::load`) | 🟡 | New this pass. Figment layers `application.yml` with an optional `application-{profile}.yml` overlay (same directory, `<stem>-<profile>.<ext>`); `${VAR}` placeholders are resolved by hand-rolled string scanning (figment's own `Env` provider does not interpolate inside YAML scalars) before typed deserialization, so an unresolved placeholder is a named, fatal error, never an empty string; validation runs `garde`'s structural derive, then the existing `validate_host`/`validate_secret` guard rules over every provider, then a currency-exponent-vs-canonical-table check and duplicate-code checks. 16 dedicated tests in `vpay-config/src/config.rs` cover: a real load against the example `config/application.yml`, the profile overlay actually overriding one field while leaving the rest untouched, an unresolved placeholder naming the missing variable, a malformed-YAML file producing a typed error rather than a panic, a missing base path, a missing base file, both `livemode` guard rules (above), a currency-exponent mismatch, a duplicate provider/currency code, and (see "Secret redaction" below) that neither `ProviderHost`'s nor the whole `Config`'s `Debug` output ever contains a credential value. **Marked 🟡, not ✅, for two reasons stated plainly in the module's own doc comment:** (1) **not wired into either binary** — `--config`/`VPAY_CONFIG` is still parsed and unused (see the CLI/env row below); this is a library API with tests, nothing more; (2) **merchant and dashboard OAuth clients are deliberately not modelled** — the auth ADR was still in flight when this was written — so two boot-guard rules from [docs/flows/configuration.md](flows/configuration.md)'s table remain unimplemented on purpose: "every merchant's rail host is in the allowlist" and "every referenced provider exists and is enabled." Boot-sequence step 4 (reconciling into the database in one transaction) is also out of scope here |
| Secret redaction (`ProviderHost`/`CommonArgs` hand-written `Debug`) | ✅ | `ProviderHost` (rail credentials) and `CommonArgs` (`--database-url`, which routinely embeds a plaintext password) both hand-write `fmt::Debug` to redact secret values while keeping every other field, and credential *keys*, visible. `Config`, `ServerArgs` and `WorkerArgs` keep `#[derive(Debug)]` — safe because a derive formats each field via *that field's own* `Debug` impl, so the redaction composes upward without a second hand-written impl at every level. Six dedicated tests prove this holds, not just for the leaf types but through the composition: `provider_host_debug_output_never_contains_a_credential_value`, `a_whole_config_debug_output_never_contains_a_credential_value` (`vpay-config/src/config.rs`), `common_args_debug_output_never_contains_the_database_password`, `server_and_worker_args_debug_output_never_contains_the_database_password` (`vpay-config/src/cli.rs`), plus two more asserting the non-secret fields (rail code, host, `[redacted]` marker itself, `database_url: None` when unset) stay visible so the redaction does not silently swallow useful debugging signal. Marked ✅ because a re-derived `Debug` on either type — the exact regression these tests exist to catch — fails the build. **Residual risk stated, not hidden:** `ProviderHost::settings` and `::credentials` are both plain `BTreeMap<String, String>` and only `credentials` is redacted; a value accidentally placed in `settings` instead would leak in plaintext, and no test (and no type) can catch a value merely misclassified between the two maps — the boundary is enforced by convention, not by the type system |
| CLI / env configuration (`vpay-config::cli`) | 🟡 | `--version` reports `0.1.0`. Every option auto-resolves from an env var with an explicit flag winning, shared between both binaries via a flattened `CommonArgs`, covered by unit tests on the built `clap::Command` plus subprocess tests that set real env vars on a child process. **`--database-url` is no longer inert** — both binaries now treat it as required at runtime and use it to open a real connection pool and run migrations before serving (see "Database connectivity" below); it stays `Option<String>` at the clap type level, so the CLI itself does not enforce presence, only the two binaries' own startup logic does. **`--config` and `--public-base-url` are still accepted and parsed but consumed by nothing** — no YAML is read from `--config` by either binary (`Config::load` exists as a library, see the row above, but nothing calls it from `main.rs`), and no redirect/webhook URL is built from `--public-base-url` |
| Database connectivity (`vpay-db`: pool, migrations, healthcheck) | 🟡 | New crate this pass: `connect()` (a `PgPoolOptions` pool, max 10 connections, 5s acquire/connect timeout, eager — it does not return until at least one connection succeeds or the timeout elapses), `run_migrations()` (`sqlx::migrate!` against `backends/migrations`, idempotent by construction), and `check_connection()` (`SELECT 1`). All three are tested against a real `postgres:16-alpine` via testcontainers in `vpay-db/tests/postgres.rs`: `run_migrations_applies_cleanly_and_is_idempotent`, `check_connection_succeeds_against_a_live_database`, and `check_connection_fails_against_a_dead_database` (the container is stopped mid-test to prove the failure path, not just asserted by reading the code). Both `vpay-server` and `vpay-worker-bin` now call `connect()` then `run_migrations()` before doing anything else observable, and this happy path is proven end-to-end, not just at the crate level: `backends/apps/vpay-server/tests/cli.rs` spawns the real binary against a real testcontainers Postgres and polls `GET /healthz` until it returns **200** (`bind_and_log_format_env_vars_are_actually_applied` and others); `vpay-worker-bin`'s equivalent tests prove the same connect-then-migrate sequence via its startup log lines. **Marked 🟡, not ✅, because two specific claims this pass makes are implemented but not proven by any test:** (1) **"a missing `--database-url` is a hard startup failure"** — true by reading `main.rs` in both binaries (`args.common.database_url.as_deref().context(...)?`), but every subprocess test in both `tests/cli.rs` files always supplies `DATABASE_URL`; no test spawns either binary without it and asserts a non-zero exit. (2) **"`/healthz` returns 503 when the database is unreachable"** — true by reading `vpay-api/src/lib.rs`'s `healthz` handler, which maps a `check_connection` error to `StatusCode::SERVICE_UNAVAILABLE`, and `check_connection`'s own failure path is unit-tested in `vpay-db` (above) — but nothing kills the database mid-request and polls the real HTTP endpoint to observe a 503; the handler's status-code mapping itself is unexercised by any test |
| Provider port trait | ✅ | Interface defined; both adapters implement it |
| Process lifecycle (SIGINT/SIGTERM) | ✅ | `vpay-server` shuts down via `axum::serve(...).with_graceful_shutdown(...)` on SIGINT or SIGTERM instead of requiring `docker compose down` to SIGKILL it. `vpay-worker-bin` no longer exits immediately on boot — it stays up, answers the same signals, and logs a startup WARN banner plus a 60-second WARN heartbeat stating the job loop is not implemented and no jobs are being processed. **Startup race fixed this pass:** both binaries used to construct their shutdown-signal future late (inside `with_graceful_shutdown`'s argument, or just before the worker's select loop) — `tokio::signal::unix::signal(..)` and `tokio::signal::ctrl_c()` both install their OS-level handler on first *poll*, not at construction, so a SIGTERM delivered before that first poll (CLI parsing, tracing init, adapter-registry logging, `TcpListener::bind` all had to complete first) kept its default disposition and killed the process outright, skipping graceful shutdown and dropping any in-flight request. Confirmed by reproduction (`kill -TERM` sent tens of milliseconds after spawn reliably produced exit 143 with no shutdown log line) and by reading `tokio`'s own source (`signal_hook_registry::register` runs synchronously inside `tokio::signal::unix::signal`'s function body, not inside the future it returns). Fixed by `vpay_config::signal::ShutdownSignals`, a new type in `backends/crates/vpay-config/src/signal.rs` shared by both binaries (precedented by `CommonArgs` living in the same crate): `ShutdownSignals::install()` is now the first thing either binary's `main` does, before tracing init, registering SIGTERM/SIGINT handlers before any slower startup work can run. On Unix, SIGINT is now handled via `signal(SignalKind::interrupt())` rather than `tokio::signal::ctrl_c()` specifically because `ctrl_c()` is an `async fn` and would reintroduce the same late-installation race; non-Unix platforms still fall back to `ctrl_c()` inside `ShutdownSignals::wait()`, unchanged from before. A failure to install a handler is now a **hard startup failure** (`main` returns `Err`), not a logged warning that lets the process run its whole life with no graceful-shutdown path — deliberately stricter than before, since silently continuing would reintroduce the exact bug for the entire process lifetime rather than a brief window. Both binaries are exercised by subprocess tests that send a real `SIGTERM` and assert a clean exit (`backends/apps/vpay-server/tests/cli.rs`, `backends/apps/vpay-worker-bin/tests/cli.rs`), including a new regression test per binary (`sigterm_immediately_after_startup_still_triggers_graceful_shutdown`) that sends SIGTERM almost immediately after spawn and asserts both exit 0 and the graceful-shutdown log line. **That regression test's own limits, stated plainly:** it is a statistical majority-vote test (`ATTEMPTS`/`MIN_SUCCESSES` spawn-signal-wait trials), not a deterministic one, because the actual race window on modern hardware is on the order of a millisecond once other confounds (binary cold-start, CPU frequency ramp-up) are controlled for — verified in isolation to reliably fail against the pre-fix code and pass against the fix (repeated hundreds of times across macOS and a Linux container). But `cargo nextest run --workspace`'s real contention from ~20 concurrently running test binaries widens that window for *both* fixed and unfixed code enough that no single delay was both safe against the fixed binary and sensitive to the bug under full-suite load; the delay actually shipped (`DELAY = 50ms`) was chosen to never fail the full suite on correctly fixed code, at the cost of not reliably catching the bug when run as part of the full suite — its demonstrated sensitivity is strongest when run scoped/alone. This is disclosed in the test's own doc comment, not hidden. |
| `--shutdown-grace-seconds` bounded drain | 🟡 | On `vpay-server` this is now wired in: `serve_with_bounded_drain` in `backends/apps/vpay-server/src/main.rs` races the axum drain against a `shutdown_grace_seconds`-long clock and exits non-zero if the clock wins, logging that in-flight work was cut off. **No test exercises the timeout path itself** — the existing SIGTERM tests never have in-flight work to drain, so they would pass identically with the grace clock deleted; nothing here proves the bound actually holds under load. On `vpay-worker-bin` the flag is accepted and logged ("has no effect yet") but genuinely does nothing — there is no drain to bound because there is no job loop |
| Poll ladder | 🟡 | `poll_delay` done + 3 tests. **Job loop not started** |
| HTTP surface | 🟡 | Still only `/healthz` and the Stripe-shaped 404 — **no `/v1/*` route exists**. What changed this pass: `/healthz` is no longer a static `"ok"` string. It runs `vpay_db::check_connection` (a real `SELECT 1`) and returns `200`/`"ok"` or `503`/`"database unreachable"` depending on the result — see "Database connectivity" above for exactly what is and is not tested about that mapping. The router now requires a `PgPool` to construct at all (`vpay_api::router(pool)`), so a router without a database connection cannot exist |
| Database schema / migrations (core) | ✅ | Five migrations exist in `backends/migrations/` (`0001_create-currencies.sql` … `0005_create-ledger.sql`), applied via `sqlx::migrate!` to a real `postgres:16-alpine` (testcontainers) and asserted against in `backends/tests/integration/tests/postgres_smoke.rs`: a clean migration run on an empty database, the `one_charge_per_intent` unique index, two cross-column `CHECK` constraints firing (`partial_refunds_imply_refunds` on `providers`, `no_over_refund` on `payment_intents`), a plain `amount >= 0` check, an FK violation, and an out-of-range currency exponent. Marked ✅ and not 🟡 because the claim this row makes — "the schema and migrations exist, apply cleanly, and their constraints actually fire" — is fully implemented and tested; a broken migration or a dead constraint would fail a real test. **This is narrower than "the database works."** No route reads or writes an application row through this schema yet — that gap is now tracked by "HTTP surface" and "Database connectivity" above (a connection pool and a migration runner now exist and are wired into both binaries, closing the exact gap this row used to describe — "there is no connection pool" is no longer true, see those rows for what is and is not proven), the same way "Provider port trait" being ✅ above does not imply the adapters' wire calls work. **This repository now has twelve migrations in total** (`0001`-`0012`); this row covers only the first five — see the rows below for `0006`-`0012` |
| Authkestra OP tables (`0006_create-authkestra-op-tables.sql`) | ✅ | `CREATE SCHEMA authkestra` plus `oauth_clients`, `oauth_codes`, `oauth_refresh_tokens`, `oauth_device_codes` — a byte-faithful transcription of the `CREATE TABLE` string literal hardcoded inside `authkestra-op` `=0.3.4`'s own `SqlxOpStore::migrate()` (not a vpay design; table/column names and types are not configurable — see the migration's header comment). Proven compatible, not just transcribed correctly by eye: `backends/tests/integration/tests/authkestra_op_smoke.rs`'s `sqlx_op_store_round_trips_a_client_and_enforces_single_use_codes` drives the real `SqlxOpStore<Postgres>` against this schema end to end — inserts a client, `find_client` (JSONB columns decode through the store's own type), `store_code`, `consume_code`, and asserts a second `consume_code` of the same code returns `None`, proving the crate's single-use `UPDATE … WHERE used = FALSE` actually fires here. A second test in `postgres_smoke.rs` proves the `oauth_codes → oauth_clients` FK fires. `oauth_device_codes` is created even though vpay's login flow (PKCE only) never uses the device grant, because `SqlxOpStore` implements `DeviceCodeStore` unconditionally. **Marked ✅ for what this row claims — the DDL exists, matches the pinned crate, and is proven compatible against a real store — not for dashboard auth working.** No shipping binary uses any of this: `authkestra-op`/`authkestra-engine` are dev-dependencies of `vpay-tests-integration` only (`backends/tests/integration/Cargo.toml`); `vpay-server` and `vpay-worker-bin` depend on neither. See "Dashboard auth" below. **Coupling risk:** this migration is pinned to `authkestra-op = "=0.3.4"` (root `Cargo.toml`) and must move in lockstep with it — the crate hand-builds SQL against these exact table/column names as string literals, so nothing type-checks a mismatch. Any future version bump of `authkestra-op` requires re-reading `sqlx_store.rs`'s `migrate()` block at the new version and re-diffing against this file before assuming compatibility still holds; the migration's own header comment says the same and this is not to be treated as a routine dependency bump |
| OAuth signing keys (`0007_create-oauth-signing-keys.sql`, reshaped by `0010_reshape-oauth-signing-keys.sql`) | 🟡 | vpay-owned table (authkestra ships no signing-key type, store, or rotation logic at any published version — confirmed by grepping `authkestra-op-0.3.4` and `authkestra-engine-0.3.4` source for `struct SigningKey`, `trait KeyStore` and `fn rotate`, with no hits). **Reshaped this pass: `private_key_pem TEXT` is dropped entirely and replaced with `public_jwk JSONB`; `id` is renamed to `kid`.** The decision (migration `0010`'s own header comment) is that the RS256 private key comes from a Kubernetes Secret via env at process boot and is parsed once by `authkestra_engine::TokenManager::new_asymmetric`, never persisted — so this table now stores only what `/jwks.json` needs to publish across a rotation window: the public half, its `kid`, and the validity window. **This corrects last pass's own note, which said the private key PEM was stored in plaintext and readable by anyone who could `SELECT` the column — that is no longer true; no private key material exists in this table or this repository at all.** The three constraints (partial unique index `one_active_signing_key`, `active_key_has_no_expiry`, `expiry_after_creation`, the last two renamed alongside the column) are proven to still fire *after* the reshape by the same dedicated tests in `postgres_smoke.rs`, updated to insert `kid`/`public_jwk` rather than `id`/`private_key_pem`. **Still marked 🟡, not ✅: there is no key-generation or rotation code at all** — the table only proves its own constraints, not that a key will ever be written to it correctly |
| Merchant API keys — dropped (`0008_create-merchant-api-keys.sql`, dropped by `0009_drop-merchant-api-keys.sql`) | ⛔ | The Stripe-shaped `sk_live_`/`sk_test_` bearer-key design this table backed is reversed by [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md): `authkestra_op::sqlx_store::SqlxOpStore::find_client` hardcodes `token_endpoint_auth_method: None`/`jwks: None` on every row, so an OP-backed client registry could never actually serve `private_key_jwt` at the pinned `authkestra-op = "=0.3.4"`. Per this repo's hard-cutover rule, `0009` is a straight `DROP TABLE`, not a deprecation — nothing had ever read or written a row here (last pass's own note said so), and the two tests that proved this table's constraints were deleted in the same migration rather than left passing against a table that no longer exists. **A reader must not infer from ADR-0010's continued reference to this migration number, or from this row remaining in the table for historical clarity, that `merchant_api_keys` still exists — it does not.** See "Merchant auth" below for the model that replaces it |
| Merchant auth (`/v1`: `client_credentials` + `private_key_jwt`, [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md)) | ⛔ | Each merchant is meant to be a statically registered OAuth2 client with a `client_id` and **public** JWK in vpay's YAML config, authenticating via a signed `private_key_jwt` assertion — no API key, no database-stored secret. **Nothing in this repository implements any part of this yet.** `vpay-config::Config` does not model merchant OAuth clients at all (its own module doc says so — deliberately, the ADR was still in flight when the loader was written); there is no `/v1` token endpoint, no `/v1` request-auth middleware, and no `ClientAssertionStore` implementation — ADR-0010's own Consequences section states plainly "Not started — no such implementation exists anywhere in this repository." `oauth_client_assertion_jtis` (migration `0011`, row below) exists as the future replay-guard table this flow will need, and is proven to enforce single-use `jti`s at the database level, but nothing constructs a store against it |
| Client-assertion replay protection (`oauth_client_assertion_jtis`, `0011_create-oauth-client-assertion-jtis.sql`) | 🟡 | Backs `authkestra_op::client_assertion::ClientAssertionStore::record_jti`, which neither of `authkestra-op`'s two shipped implementations can satisfy for vpay's deployment: `NoClientAssertionStore` fails closed unconditionally, and `MemoryClientAssertionStore` is single-process only (its own doc comment names exactly vpay's situation — multiple replicas — as needing "something shared... instead"). This table's `jti TEXT PRIMARY KEY` is the atomic single-use guard, meant to be used as `INSERT ... ON CONFLICT (jti) DO NOTHING` read via `rows_affected()`, never check-then-insert (the migration's own header comment explains the TOCTOU race a separate SELECT would reintroduce). Two dedicated tests in `postgres_smoke.rs` prove this at the database level: `a_duplicate_client_assertion_jti_is_rejected_by_the_database` (a plain duplicate `INSERT` is rejected on the `jti` primary key specifically) and `on_conflict_do_nothing_reports_zero_rows_affected_for_a_replayed_jti` (proves the exact `ON CONFLICT DO NOTHING` + `rows_affected()` pattern reports 1 on first use and 0 on replay, rather than erroring or double-counting). **Marked 🟡, not ✅: no Rust `ClientAssertionStore` implementation exists anywhere in this repository** — this table has no reader or writer yet, only its own constraint proven. **Known, not handled:** there is no cleanup job for expired rows (the worker job loop is still ⛔, see "Poll ladder" above), so this table would grow unbounded once something starts writing to it — the migration's own header comment says so and does not add the sweep |
| Disabled-clients kill switch (`disabled_clients`, `0012_create-disabled-clients.sql`) | 🟡 | An operator revocation mechanism for an OAuth client (dashboard or merchant `client_credentials`) that takes effect without a deploy — `client_id` plus a disable flag/reason, no credential and no identity of its own (YAML stays authoritative for identity; this table only ever *subtracts* access). Its uniqueness is proven by two tests in `postgres_smoke.rs`: `disabled_clients_accepts_an_insert` and `a_duplicate_disabled_client_id_is_rejected_by_the_database` (rejected specifically on the `client_id` primary key). **This table genuinely exists** — flagged explicitly here because ADR-0010's own author could not verify its existence from within that document's stated scope (`docs/adr/**`, not `backends/migrations/**`) and hedged accordingly; a reader of this status page should not carry that hedge forward. **Marked 🟡, not ✅: no code anywhere reads or writes this table.** There is no kill-switch check in any auth path, because there is no auth path at all yet — see "Merchant auth" above and "Dashboard auth" below |
| Dashboard auth (`/dash/v1` as an Authkestra OP) | ⛔ | Decision recorded in [ADR-0009](adr/0009-dashboard-oidc-provider.md), design in [docs/flows/dashboard-auth.md](flows/dashboard-auth.md). **Still no dashboard-auth code and no `/dash/v1` route.** `authkestra-op`/`authkestra-engine`/`authkestra-axum`/`authkestra-resource` now appear in the root `Cargo.toml` as pinned workspace dependency versions, and `authkestra-op`/`authkestra-engine` are real dev-dependencies of `vpay-tests-integration` (used only by `tests/authkestra_op_smoke.rs`, above) — but **no shipping binary depends on any of it**: `vpay-server` and `vpay-worker-bin` do not list `authkestra*` in their `Cargo.toml`s. The migrations above give this a real, tested schema — including this pass's reshape of `oauth_signing_keys` to hold no private key material — but a reader must not conclude login works from that: no login has ever been performed, no token has ever been issued by this code, and no key has ever been rotated. **One prerequisite this row used to cite is gone:** a real connection pool and migration runner now exist (`vpay-db`, "Database connectivity" above) and both binaries use them at boot. **That does not move this row forward** — the actual blocker is that no shipping binary ever constructs an `authkestra_op::sqlx_store::SqlxOpStore`, mounts `/dash/v1`, or registers a dashboard OAuth client; `vpay-db`'s pool is a generic `PgPool`, not wired to Authkestra's store type anywhere outside the one integration test named above |
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
| `deny.toml` | ✅ | `cargo deny check` passes clean: `advisories ok, bans ok, licenses ok, sources ok`. The three advisories that failed before were fixed by **upgrading dependencies, not by suppressing them** — see below. One advisory is explicitly ignored: **RUSTSEC-2023-0071** (Marvin Attack in `rsa`, no patched release, an unconditional dependency of `authkestra-engine` per [ADR-0009](adr/0009-dashboard-oidc-provider.md)), accepted deliberately with the reasoning recorded inline in `deny.toml`. **This entry was preemptive when added and now genuinely fires**: `authkestra-op`/`authkestra-engine` landed as dev-dependencies of `vpay-tests-integration` in this pass (see "Authkestra OP tables" above), and `cargo deny -L info check advisories` now reports `note[advisory-ignored]` against `rsa v0.9.10` via that path — independently re-run for this update, output confirmed. `cargo deny check` still passes with 0 errors because an `ignore`d advisory downgrades to a note, not a failure; the exposure itself is still narrower than "in production," since the only path to `rsa` is `vpay-tests-integration`'s dev-dependencies — no shipping binary pulls it in. Also bans `aws-lc-rs`/`aws-lc-sys` so a second rustls crypto provider cannot reappear. **New this pass:** `CDLA-Permissive-2.0` was added to the allow list, with its justification recorded inline — it covers `webpki-roots` (Mozilla's CA bundle, data not code), pulled in through `sqlx`'s `tls-rustls-ring` feature now that `vpay-db` is a non-dev dependency using it (root `Cargo.toml`'s own comment: previously latent in the workspace's pins, now actually reachable). `tls-rustls-ring` (vendored roots) was chosen deliberately over `tls-rustls-ring-native-roots`: the runtime image is `FROM scratch` ([ADR-0004](adr/0004-musl-mimalloc.md)) with no OS trust store for `rustls-native-certs` to read, so native roots would fail TLS to Postgres in the shipped image only, while passing locally and in CI where a trust store exists — exactly the kind of gap that would not be caught until a real deployment. `rustls-native-certs` does still appear in the dependency graph (via `bollard → testcontainers → vpay-testkit`), but only as a `[dev-dependencies]` chain — `cargo tree -i rustls-native-certs` shows every path terminating in a dev-dependency of `vpay-testkit`/`vpay-db`/`vpay-tests-integration`, never a shipping binary, independently confirmed for this update |
| GitHub Actions | 🟡 | Workflow written; **never executed** |
| `schemas/*.cstack` | 🟡 | **Syntax verified against real CrateStack 0.7.10** (and 0.7.8 before it); content remains a design sketch, excluded from the build graph — see below. **The migrations are now the authoritative schema, and this file has diverged from them on two constraints**: raw SQL in `backends/migrations/0002_create-providers.sql` and `0003_create-payment-intents.sql` expresses two `CHECK` constraints (`partial_refunds_imply_refunds`, `no_over_refund`) that CrateStack's grammar cannot — no `@@check(expr)` exists in 0.7.8 or 0.7.10
(the only `@@` attributes the parser accepts are `@@index` and `@@unique`, and
`cratestack-migrate` still gates CHECK emission on a single field's validator). The `.cstack` file's own `GAP` comments on those two models now point at the migrations that implement them |

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

Independently re-run against `cratestack 0.7.10` on its release, and against
`0.7.8` before it — same output verbatim from both, so the 0.7.10 release is a
clean re-verification rather than a claim inherited from an older run.

What this does and does not prove:

- **Syntax is verified.** Every scalar, attribute, relation and enum in the
  file parses and type-checks against the real CrateStack 0.7.10 grammar.
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
  syntax-verified against real CrateStack 0.7.10 and still excluded from the
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
4. `/v1/payment_intents` create + confirm, form-encoded, with idempotency,
   authenticated. **The credential model changed this pass and the old
   answer to this item is gone**, not just incomplete: `merchant_api_keys`
   (migration `0008`) is dropped (migration `0009`) and the design it backed
   is reversed by [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md) —
   there is no opaque key of any kind on `/v1` anymore. The replacement
   (`client_credentials` + `private_key_jwt`, merchant clients in YAML) has
   **no implementation at all yet**: no config schema for merchant OAuth
   clients, no `/v1` token endpoint, no `/v1` auth middleware, and no
   `ClientAssertionStore` — see "Merchant auth" above. The one piece that
   does exist, `oauth_client_assertion_jtis` (migration `0011`), is a
   replay-guard table with a proven database constraint and no reader or
   writer. This item has moved backward in apparent completeness relative to
   last pass's note, which is correct: the schema-only credential store this
   item used to point at no longer exists, and its replacement has not been
   started.
5. Signed webhooks with the two-step outbox.
6. `just test-e2e` green against the compose stack.
7. `/dash/v1` login working end to end against a real database — issuing an
   access token, verifying it on a subsequent call, and rotating a signing
   key at least once. The schema this needs now exists (migrations `0006`,
   `0007`/`0010`, rows above) and is proven compatible with `SqlxOpStore`;
   `0010` additionally reshaped `oauth_signing_keys` so no private key
   material is ever persisted, closing a real risk the previous schema had.
   **A real connection pool and migration runner now exist and are wired
   into both binaries** ("Database connectivity" above) — one prerequisite
   this item used to lack is gone. That still does not move this item
   forward: nothing in this repository has ever performed a login, issued a
   token, or rotated a key, no shipping binary constructs a `SqlxOpStore`,
   and no `/dash/v1` route exists at all — see "Dashboard auth" above. Not
   part of "does this take payments," included here because it is the other
   place this pass's migrations land, and the same "schema ≠ working
   feature" caution applies.

Until every one of those is ✅, this README's own claim is: **it does not take payments.**
