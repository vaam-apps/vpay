# STATUS

**What actually works today.** This page is the contract behind the repo's second
rule: *never advertise a feature as done when it clearly is not.*

It is machine-checked. `cargo xtask verify-status` scans the workspace for every
`ProviderError::NotImplemented("…")` token and fails the build if one is missing
from this file. You cannot quietly ship an unimplemented path.

Last verified: 2026-09-02, on branch `claude/sdk-rust-nodejs-0c1ecf` against
`8c0760e`. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
-- -D warnings`, `cargo xtask verify-no-mocks`, `cargo xtask verify-status`,
`cargo deny check` (`advisories ok, bans ok, licenses ok, sources ok`),
`pnpm -r typecheck`, `pnpm -r test` (136 passed) and `RUSTDOCFLAGS="-D
warnings" cargo doc -p vpay-sdk` all clean. `cargo nextest run --workspace`:
**248 tests, 214 passed, 34 failed, 3 skipped** — and the 34 failures must be
read exactly: every one is in a suite that starts a `postgres:16-alpine`
testcontainer (`vpay-db`, `vpay-server::cli`, `vpay-worker-bin::cli`,
`vpay-tests-integration`), and every one failed before its first assertion
with the Docker daemon on the authoring machine refusing to start *any*
container ("failed to start shim … unsupported protocol: Yunix", a
containerd fault; 24 unrelated containers were running, so the daemon was
not restarted). None of those 34 tests could be run here; none is claimed
to pass. The 3 skipped are the pre-existing `#[ignore]`d adapter-conformance
cases. What landed this pass: (1) **`authkestra-*` 0.5.4 → 0.7.1** (latest on
crates.io that day), with the DDL re-diff the OP-tables row demanded done for
real and its additive delta transcribed as migration `0013` — **proven
against a real Postgres despite the daemon fault**: a throwaway harness
(outside the repo) applied migrations 0001–0013 to a fresh database on an
already-running Postgres 18 container and drove the real 0.7.1
`SqlxOpStore<Postgres>` through `find_client` (decoding the two new
columns), `store_code`/`consume_code`, `store_token`/`get_token` (`jkt`),
and `check_and_record_dpop_jti`, all passing, plus a negative control on a
second database migrated only to 0012 where the same store's `store_token`
and DPoP writes fail as expected — the repo's own three
`authkestra_op_smoke.rs` tests encode exactly those checks and remain unrun
here for the daemon reason above; (2) **CrateStack re-verified at 0.10.1**
(`schema OK`); (3) two `cargo deny` advisory regressions on `master` fixed by
upgrade (`h2`, `chacha20`); (4) **two merchant SDKs** and the wire contract
they implement — see the new "Merchant SDKs" section and
`docs/flows/merchant-auth.md`; the Rust SDK adds 107 tests to the workspace
count (248 = 141 pre-existing + 107). The previous pass's own note is
unchanged below, describing the state before this one:
`cargo nextest run --workspace` (139 passed, 3 skipped), `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo xtask verify-status`, `just verify`, and
`cargo deny check` (`advisories ok, bans ok, licenses ok, sources ok`), all
run against the working tree of three things landing together this pass,
labelled OP-1/OP-2/OP-3 below: (1) **OP-1** — `vpay_config::oauth` now models
both kinds of OAuth2 client ADR-0010/`docs/flows/dashboard-auth.md` need
(`MerchantClient`, `DashboardClient`), with seven boot-time validation rules,
and — the actually load-bearing change — **`Config::load` is now called by
both binaries**, ordered before the database connection, with `--config` /
`VPAY_CONFIG` treated as required at the binary level and proven so by
subprocess tests (`a_missing_config_is_a_non_zero_exit_naming_the_problem`,
`a_bad_config_causes_a_non_zero_exit_naming_the_problem`,
`a_valid_config_lets_the_server_boot_and_serve_healthz` /
`a_valid_config_lets_the_worker_boot`, in each binary's `tests/cli.rs`); (2)
**OP-2** — a new repository layer in `vpay-db`
(`SqlClientAssertionStore`, `is_client_disabled`/`disable_client`/
`enable_client`, `publishable_signing_keys`/`active_signing_key_kid`/
`rotate_signing_key`), each tested against a real Postgres, the replay store
additionally proven race-safe by a 10-way concurrent test; (3) **OP-3** — a
`JwtValidator`/`AuthenticatedMerchant`/`AuthenticatedDashboard` extractor pair
in `vpay-api::resource_auth`, validating against a cached JWKS, with a real
`jsonwebtoken` audience-validation gap found and closed
(`set_required_spec_claims(&["exp","aud","iss"])` — see that row below).
**None of this makes login or merchant authentication work.** The router is
still `/healthz` plus the Stripe-shaped 404: no `/v1/*` route, no `/dash/v1/*`
route, no OP endpoints (`/authorize`, `/token`, `/jwks.json`, discovery), no
`ClientStore` converting a configured client into
`authkestra_op::client::ClientRegistration`, and no shipping binary ever
constructs `SqlClientAssertionStore`, calls the kill-switch functions, or
calls `rotate_signing_key` — the stores above are proven correct in
isolation, not proven wired into anything that serves traffic. No signing key
has ever been generated; `rotate_signing_key` rotates *to* a key its caller
already has, it does not create one. See the rows below for exactly what each
piece proves and does not prove, and the "Resource-server JWT validation" and
"rustls `CryptoProvider` process default" rows in particular for a landmine
this pass surfaced but did not fix: `authkestra_resource::jwt::Jwks::fetch`
panics without a process-wide default TLS crypto provider installed, and
nothing in a shipping binary installs one. **A dependency-graph fact the
previous pass's own note got wrong going forward, caught while verifying this
pass, not claimed by whoever wrote OP-1/OP-2/OP-3:** the "`cargo deny`"
infrastructure row below used to say the `rsa` advisory's only path was
`vpay-tests-integration`'s dev-dependencies, "no shipping binary pulls it
in." That stopped being true the moment `vpay-db` added `authkestra-op` as a
*production* dependency for OP-2 — `cargo tree -i rsa` now shows
`rsa → authkestra-engine → authkestra-op → vpay-db → vpay-server` /
`vpay-worker-bin` with no `(dev)` marker anywhere on that path. `cargo deny
check` still exits 0 (an `ignore`d advisory is a note, not an error), so nothing
here is a CI regression, but the row's own narrowing of the exposure to
"dev-only" is now false and is corrected below. The Rust count moved from 105
passed / 3 skipped to **139 passed / 3 skipped in this pass**: 34 new tests,
counted directly against `932d8a4` (the commit `docs/status.md` last verified
against): `vpay-config` gains 12 (5 in the new `oauth.rs`, 7 new
OAuth-client validation-fixture tests in `config.rs`), `vpay-db` gains 5 (all
of `tests/repositories.rs`, listed above), `vpay-api` gains 11 (the entire
new `resource_auth.rs` test module: signature/expiry/audience/issuer/`kid`
coverage plus the extractor-and-error-envelope tests), and both binaries'
`tests/cli.rs` gain 3 apiece (6 total) proving the config-required-at-startup
behaviour end to end. 12 + 5 + 11 + 6 = 34. The previous pass's own note is
unchanged below, describing the state before
this one: the Rust count moved from 80 passed / 3 skipped to **105 passed / 3
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
| YAML config loading (`vpay-config::Config::load`) | ✅ | Figment layers `application.yml` with an optional `application-{profile}.yml` overlay (same directory, `<stem>-<profile>.<ext>`); `${VAR}` placeholders are resolved by hand-rolled string scanning (figment's own `Env` provider does not interpolate inside YAML scalars) before typed deserialization, so an unresolved placeholder is a named, fatal error, never an empty string; validation runs `garde`'s structural derive, then the existing `validate_host`/`validate_secret` guard rules over every provider, then a currency-exponent-vs-canonical-table check, duplicate-code checks, and (new this pass) the OAuth-client rules below. 23 dedicated tests in `vpay-config/src/config.rs` cover all of that plus (see "Secret redaction" below) that neither `ProviderHost`'s nor the whole `Config`'s `Debug` output ever contains a credential value. **Upgraded from 🟡 to ✅ this pass, for the two reasons the previous note gave for withholding it — both are now closed and both are proven by an end-to-end subprocess test, not just a library-level one:** (1) **now wired into both binaries.** `vpay-server` and `vpay-worker-bin` both call `Config::load` before opening a database connection, and `--config`/`VPAY_CONFIG` is now required at the binary level (still `Option<PathBuf>` at the `clap` type level, exactly like `--database-url`) — proven by three subprocess tests per binary in each `tests/cli.rs`: a missing config is a non-zero exit naming `--config`/`VPAY_CONFIG` (`a_missing_config_is_a_non_zero_exit_naming_the_problem`), a config that fails validation is a non-zero exit (`a_bad_config_causes_a_non_zero_exit_naming_the_problem`), and a valid config lets the process boot and (for `vpay-server`) actually serve `/healthz` (`a_valid_config_lets_the_server_boot_and_serve_healthz` / `a_valid_config_lets_the_worker_boot`). (2) **merchant and dashboard OAuth clients are now modelled** — see `crate::oauth` (new this pass: `MerchantClient`, `DashboardClient`) and the new "Merchant OAuth clients" notes folded into this row below. **What is still explicitly out of scope, unchanged from before and stated in the module's own doc comment:** two boot-guard rules from [docs/flows/configuration.md](flows/configuration.md)'s table remain unimplemented on purpose, because they need a *payment-routing* `merchants` concept this config shape does not have — "every merchant's rail host is in the allowlist" and "every referenced provider exists and is enabled." An OAuth `MerchantClient`'s `client_id` is not that merchant concept and has no rail host to check. Boot-sequence step 4 (reconciling into the database in one transaction) is also still out of scope here. Neither gap weakens the claim this row actually makes — that `Config::load` loads, validates, and is used — so ✅ stands for that claim specifically |
| Merchant/dashboard OAuth client modelling (`vpay-config::oauth`, ADR-0010) | ✅ | New this pass, folded into the row above operationally but broken out here because it is a distinct piece of new modelling: `MerchantClient` (public JWK set, `client_credentials` only) and `DashboardClient` (redirect URIs, a single `scope` — enforced by the type being a `String`, not a `Vec<String>`), plus a closed local `GrantType` enum whose serde wire form matches `authkestra_op::client::GrantType`'s. Both carry a `client_secret: Option<String>` trap field that must always be `None`, with hand-written redacting `Debug` impls (5 tests in `oauth.rs`, including one proving a populated `client_secret` never appears in `{:?}` output). Seven boot-time validation rules run from `Config::validate_all`, each with a dedicated fixture-driven test asserting the *specific* `ConfigError` variant: duplicate `client_id` across merchants and the dashboard, an empty/keyless merchant JWKS, a merchant declaring a grant other than `client_credentials`, a dashboard client with no redirect URI, a non-`https` livemode dashboard redirect URI (reusing `validate_host`), and a client secret present anywhere (merchant or dashboard, tested separately). **This is authentication-client modelling only, not merchant *payment routing*** — see the row above's "still out of scope" note for exactly what that distinction means and does not cover |
| Secret redaction (`ProviderHost`/`CommonArgs` hand-written `Debug`) | ✅ | `ProviderHost` (rail credentials) and `CommonArgs` (`--database-url`, which routinely embeds a plaintext password) both hand-write `fmt::Debug` to redact secret values while keeping every other field, and credential *keys*, visible. `Config`, `ServerArgs` and `WorkerArgs` keep `#[derive(Debug)]` — safe because a derive formats each field via *that field's own* `Debug` impl, so the redaction composes upward without a second hand-written impl at every level. Six dedicated tests prove this holds, not just for the leaf types but through the composition: `provider_host_debug_output_never_contains_a_credential_value`, `a_whole_config_debug_output_never_contains_a_credential_value` (`vpay-config/src/config.rs`), `common_args_debug_output_never_contains_the_database_password`, `server_and_worker_args_debug_output_never_contains_the_database_password` (`vpay-config/src/cli.rs`), plus two more asserting the non-secret fields (rail code, host, `[redacted]` marker itself, `database_url: None` when unset) stay visible so the redaction does not silently swallow useful debugging signal. Marked ✅ because a re-derived `Debug` on either type — the exact regression these tests exist to catch — fails the build. **Residual risk stated, not hidden:** `ProviderHost::settings` and `::credentials` are both plain `BTreeMap<String, String>` and only `credentials` is redacted; a value accidentally placed in `settings` instead would leak in plaintext, and no test (and no type) can catch a value merely misclassified between the two maps — the boundary is enforced by convention, not by the type system |
| CLI / env configuration (`vpay-config::cli`) | 🟡 | `--version` reports `0.1.0`. Every option auto-resolves from an env var with an explicit flag winning, shared between both binaries via a flattened `CommonArgs`, covered by unit tests on the built `clap::Command` plus subprocess tests that set real env vars on a child process. **`--database-url` is no longer inert** — both binaries now treat it as required at runtime and use it to open a real connection pool and run migrations before serving (see "Database connectivity" below); it stays `Option<String>` at the clap type level, so the CLI itself does not enforce presence, only the two binaries' own startup logic does. **`--config` is no longer inert either, as of this pass** — both binaries now treat it as required at runtime too (same `Option<PathBuf>`-at-the-clap-level, required-in-`main.rs` pattern as `--database-url`), calling `vpay_config::Config::load` and refusing to start on a missing or invalid file; see the "YAML config loading" row above for the three subprocess tests per binary that prove this. **`--public-base-url` remains the one flag still accepted and parsed but consumed by nothing** — no redirect/webhook URL is ever built from it anywhere in this repository |
| Database connectivity (`vpay-db`: pool, migrations, healthcheck) | 🟡 | New crate this pass: `connect()` (a `PgPoolOptions` pool, max 10 connections, 5s acquire/connect timeout, eager — it does not return until at least one connection succeeds or the timeout elapses), `run_migrations()` (`sqlx::migrate!` against `backends/migrations`, idempotent by construction), and `check_connection()` (`SELECT 1`). All three are tested against a real `postgres:16-alpine` via testcontainers in `vpay-db/tests/postgres.rs`: `run_migrations_applies_cleanly_and_is_idempotent`, `check_connection_succeeds_against_a_live_database`, and `check_connection_fails_against_a_dead_database` (the container is stopped mid-test to prove the failure path, not just asserted by reading the code). Both `vpay-server` and `vpay-worker-bin` now call `connect()` then `run_migrations()` before doing anything else observable, and this happy path is proven end-to-end, not just at the crate level: `backends/apps/vpay-server/tests/cli.rs` spawns the real binary against a real testcontainers Postgres and polls `GET /healthz` until it returns **200** (`bind_and_log_format_env_vars_are_actually_applied` and others); `vpay-worker-bin`'s equivalent tests prove the same connect-then-migrate sequence via its startup log lines. **Marked 🟡, not ✅, because two specific claims this pass makes are implemented but not proven by any test:** (1) **"a missing `--database-url` is a hard startup failure"** — true by reading `main.rs` in both binaries (`args.common.database_url.as_deref().context(...)?`), but every subprocess test in both `tests/cli.rs` files always supplies `DATABASE_URL`; no test spawns either binary without it and asserts a non-zero exit. (2) **"`/healthz` returns 503 when the database is unreachable"** — true by reading `vpay-api/src/lib.rs`'s `healthz` handler, which maps a `check_connection` error to `StatusCode::SERVICE_UNAVAILABLE`, and `check_connection`'s own failure path is unit-tested in `vpay-db` (above) — but nothing kills the database mid-request and polls the real HTTP endpoint to observe a 503; the handler's status-code mapping itself is unexercised by any test |
| Provider port trait | ✅ | Interface defined; both adapters implement it |
| Process lifecycle (SIGINT/SIGTERM) | ✅ | `vpay-server` shuts down via `axum::serve(...).with_graceful_shutdown(...)` on SIGINT or SIGTERM instead of requiring `docker compose down` to SIGKILL it. `vpay-worker-bin` no longer exits immediately on boot — it stays up, answers the same signals, and logs a startup WARN banner plus a 60-second WARN heartbeat stating the job loop is not implemented and no jobs are being processed. **Startup race fixed this pass:** both binaries used to construct their shutdown-signal future late (inside `with_graceful_shutdown`'s argument, or just before the worker's select loop) — `tokio::signal::unix::signal(..)` and `tokio::signal::ctrl_c()` both install their OS-level handler on first *poll*, not at construction, so a SIGTERM delivered before that first poll (CLI parsing, tracing init, adapter-registry logging, `TcpListener::bind` all had to complete first) kept its default disposition and killed the process outright, skipping graceful shutdown and dropping any in-flight request. Confirmed by reproduction (`kill -TERM` sent tens of milliseconds after spawn reliably produced exit 143 with no shutdown log line) and by reading `tokio`'s own source (`signal_hook_registry::register` runs synchronously inside `tokio::signal::unix::signal`'s function body, not inside the future it returns). Fixed by `vpay_config::signal::ShutdownSignals`, a new type in `backends/crates/vpay-config/src/signal.rs` shared by both binaries (precedented by `CommonArgs` living in the same crate): `ShutdownSignals::install()` is now the first thing either binary's `main` does, before tracing init, registering SIGTERM/SIGINT handlers before any slower startup work can run. On Unix, SIGINT is now handled via `signal(SignalKind::interrupt())` rather than `tokio::signal::ctrl_c()` specifically because `ctrl_c()` is an `async fn` and would reintroduce the same late-installation race; non-Unix platforms still fall back to `ctrl_c()` inside `ShutdownSignals::wait()`, unchanged from before. A failure to install a handler is now a **hard startup failure** (`main` returns `Err`), not a logged warning that lets the process run its whole life with no graceful-shutdown path — deliberately stricter than before, since silently continuing would reintroduce the exact bug for the entire process lifetime rather than a brief window. Both binaries are exercised by subprocess tests that send a real `SIGTERM` and assert a clean exit (`backends/apps/vpay-server/tests/cli.rs`, `backends/apps/vpay-worker-bin/tests/cli.rs`), including a new regression test per binary (`sigterm_immediately_after_startup_still_triggers_graceful_shutdown`) that sends SIGTERM almost immediately after spawn and asserts both exit 0 and the graceful-shutdown log line. **That regression test's own limits, stated plainly:** it is a statistical majority-vote test (`ATTEMPTS`/`MIN_SUCCESSES` spawn-signal-wait trials), not a deterministic one, because the actual race window on modern hardware is on the order of a millisecond once other confounds (binary cold-start, CPU frequency ramp-up) are controlled for — verified in isolation to reliably fail against the pre-fix code and pass against the fix (repeated hundreds of times across macOS and a Linux container). But `cargo nextest run --workspace`'s real contention from ~20 concurrently running test binaries widens that window for *both* fixed and unfixed code enough that no single delay was both safe against the fixed binary and sensitive to the bug under full-suite load; the delay actually shipped (`DELAY = 50ms`) was chosen to never fail the full suite on correctly fixed code, at the cost of not reliably catching the bug when run as part of the full suite — its demonstrated sensitivity is strongest when run scoped/alone. This is disclosed in the test's own doc comment, not hidden. |
| `--shutdown-grace-seconds` bounded drain | 🟡 | On `vpay-server` this is now wired in: `serve_with_bounded_drain` in `backends/apps/vpay-server/src/main.rs` races the axum drain against a `shutdown_grace_seconds`-long clock and exits non-zero if the clock wins, logging that in-flight work was cut off. **No test exercises the timeout path itself** — the existing SIGTERM tests never have in-flight work to drain, so they would pass identically with the grace clock deleted; nothing here proves the bound actually holds under load. On `vpay-worker-bin` the flag is accepted and logged ("has no effect yet") but genuinely does nothing — there is no drain to bound because there is no job loop |
| Poll ladder | 🟡 | `poll_delay` done + 3 tests. **Job loop not started** |
| HTTP surface | 🟡 | Still only `/healthz` and the Stripe-shaped 404 — **no `/v1/*` route exists**. What changed this pass: `/healthz` is no longer a static `"ok"` string. It runs `vpay_db::check_connection` (a real `SELECT 1`) and returns `200`/`"ok"` or `503`/`"database unreachable"` depending on the result — see "Database connectivity" above for exactly what is and is not tested about that mapping. The router now requires a `PgPool` to construct at all (`vpay_api::router(pool)`), so a router without a database connection cannot exist |
| Database schema / migrations (core) | ✅ | Five migrations exist in `backends/migrations/` (`0001_create-currencies.sql` … `0005_create-ledger.sql`), applied via `sqlx::migrate!` to a real `postgres:16-alpine` (testcontainers) and asserted against in `backends/tests/integration/tests/postgres_smoke.rs`: a clean migration run on an empty database, the `one_charge_per_intent` unique index, two cross-column `CHECK` constraints firing (`partial_refunds_imply_refunds` on `providers`, `no_over_refund` on `payment_intents`), a plain `amount >= 0` check, an FK violation, and an out-of-range currency exponent. Marked ✅ and not 🟡 because the claim this row makes — "the schema and migrations exist, apply cleanly, and their constraints actually fire" — is fully implemented and tested; a broken migration or a dead constraint would fail a real test. **This is narrower than "the database works."** No route reads or writes an application row through this schema yet — that gap is now tracked by "HTTP surface" and "Database connectivity" above (a connection pool and a migration runner now exist and are wired into both binaries, closing the exact gap this row used to describe — "there is no connection pool" is no longer true, see those rows for what is and is not proven), the same way "Provider port trait" being ✅ above does not imply the adapters' wire calls work. **This repository now has twelve migrations in total** (`0001`-`0012`); this row covers only the first five — see the rows below for `0006`-`0012` |
| Authkestra OP tables (`0006_create-authkestra-op-tables.sql`, extended by `0013_add-authkestra-op-0-7-columns.sql`) | ✅ | `CREATE SCHEMA authkestra` plus `oauth_clients`, `oauth_codes`, `oauth_refresh_tokens`, `oauth_device_codes` — a byte-faithful transcription of the `CREATE TABLE` string literal hardcoded inside `authkestra-op` `=0.3.4`'s own `SqlxOpStore::migrate()` (not a vpay design; table/column names and types are not configurable — see the migration's header comment). **Upgraded to `authkestra-op = "=0.7.1"` this pass (from `=0.5.4`), and the re-diff the previous note demanded was done, not assumed:** `diff` over the extracted 0.3.4 and 0.7.1 crate sources shows the four tables 0006 creates are byte-identical, and 0.7.1's `migrate()` adds exactly one table (`authkestra.oauth_dpop_jti`, RFC 9449 DPoP replay tracking, authkestra#291) and three columns (`oauth_refresh_tokens.jkt`, `oauth_clients.token_endpoint_auth_method JSONB`, `oauth_clients.jwks JSONB`, authkestra#287). Migration `0013` transcribes those additions; it is **not optional** at this pin — `get_token`/`consume_token` now `SELECT … jkt` unconditionally and would fail at runtime against 0006's table alone. Proven compatible, not just transcribed correctly by eye: `backends/tests/integration/tests/authkestra_op_smoke.rs`'s `sqlx_op_store_round_trips_a_client_and_enforces_single_use_codes` drives the real `SqlxOpStore<Postgres>` against this schema end to end — inserts a client, `find_client` (JSONB columns decode through the store's own type, **now including `token_endpoint_auth_method` decoding to `TokenEndpointAuthMethod::PrivateKeyJwt` and `jwks` round-tripping as raw JSON**), `store_code`, `consume_code`, and asserts a second `consume_code` of the same code returns `None`, proving the crate's single-use `UPDATE … WHERE used = FALSE` actually fires here. Two new tests in the same file cover 0013's other additions through the store's own SQL: `sqlx_op_store_round_trips_a_refresh_token_with_its_jkt_column` (`store_token`/`get_token` round-trip `jkt`) and `sqlx_op_store_records_a_dpop_jti_once_against_migration_0013s_table` (`check_and_record_dpop_jti` accepts a fresh `jti` and refuses its unexpired replay). Neither refresh tokens nor DPoP are features vpay offers — see `docs/flows/dashboard-auth.md` — these prove schema compatibility with the pinned crate, nothing more. **Two API breaks absorbed in the same test file:** `AuthorizationCode` is `#[non_exhaustive]` since 0.6.0 (constructed via `AuthorizationCode::new` now), and `ClientRegistration::require_pkce` is deprecated since 0.7.0 because PKCE is unconditional on the authorization-code grant (authkestra#273) — the test no longer asserts on a field nothing reads. A second test in `postgres_smoke.rs` proves the `oauth_codes → oauth_clients` FK fires. `oauth_device_codes` is created even though vpay's login flow (PKCE only) never uses the device grant, because `SqlxOpStore` implements `DeviceCodeStore` unconditionally. **Marked ✅ for what this row claims — the DDL exists, matches the pinned crate, and is proven compatible against a real store — not for dashboard auth working.** No shipping binary constructs a `SqlxOpStore` or uses these tables — see "Dashboard auth" below. **Correcting a claim this row used to make, which this pass's dependency-graph check found stale:** it used to say `authkestra-op`/`authkestra-engine` were dev-dependencies of `vpay-tests-integration` only, with neither `vpay-server` nor `vpay-worker-bin` depending on `authkestra*` at all. That second half is no longer true — `vpay-db` added `authkestra-op` as a **production** dependency this pass (for `SqlClientAssertionStore`, OP-2), and both binaries depend on `vpay-db`, so `authkestra-op` (and, transitively, `authkestra-engine`) is now in both binaries' production dependency graph. `vpay-server`/`vpay-worker-bin` still do not name `authkestra*` directly in their own `Cargo.toml`s, but "depend on neither" is no longer an accurate description of the resolved graph — see the "cargo deny" infrastructure row for the concrete consequence (the `rsa` advisory's exposure is narrower than "dev-only" now claims). **Coupling risk:** this migration pair is pinned to `authkestra-op = "=0.7.1"` (root `Cargo.toml`) and must move in lockstep with it — the crate hand-builds SQL against these exact table/column names as string literals, so nothing type-checks a mismatch. Any future version bump of `authkestra-op` requires re-reading `sqlx_store.rs`'s `migrate()` block at the new version and re-diffing against this file before assuming compatibility still holds; the migration's own header comment says the same and this is not to be treated as a routine dependency bump |
| OAuth signing keys (`0007_create-oauth-signing-keys.sql`, reshaped by `0010_reshape-oauth-signing-keys.sql`) | 🟡 | vpay-owned table (authkestra ships no signing-key type, store, or rotation logic at any published version — confirmed by grepping `authkestra-op-0.3.4` and `authkestra-engine-0.3.4` source for `struct SigningKey`, `trait KeyStore` and `fn rotate`, with no hits). **Reshaped this pass: `private_key_pem TEXT` is dropped entirely and replaced with `public_jwk JSONB`; `id` is renamed to `kid`.** The decision (migration `0010`'s own header comment) is that the RS256 private key comes from a Kubernetes Secret via env at process boot and is parsed once by `authkestra_engine::TokenManager::new_asymmetric`, never persisted — so this table now stores only what `/jwks.json` needs to publish across a rotation window: the public half, its `kid`, and the validity window. **This corrects last pass's own note, which said the private key PEM was stored in plaintext and readable by anyone who could `SELECT` the column — that is no longer true; no private key material exists in this table or this repository at all.** The three constraints (partial unique index `one_active_signing_key`, `active_key_has_no_expiry`, `expiry_after_creation`, the last two renamed alongside the column) are proven to still fire *after* the reshape by the same dedicated tests in `postgres_smoke.rs`, updated to insert `kid`/`public_jwk` rather than `id`/`private_key_pem`. **New this pass: a Rust repository layer exists** (`vpay_db::signing_keys` — `publishable_signing_keys`, `active_signing_key_kid`, `rotate_signing_key`), tested against a real Postgres in `vpay-db/tests/repositories.rs` — `publishable_signing_keys_includes_active_and_unexpired_retired_but_excludes_expired` proves the `WHERE active OR expires_at > now()` overlap-window query keeps a just-retired key publishable and drops a long-expired one, and `rotate_signing_key_leaves_exactly_one_active_key` proves the one-transaction retire-then-insert both bootstraps cleanly (no prior active key) and rotates cleanly (an active key already exists), leaving `one_active_signing_key` intact either way. **Still marked 🟡, not ✅, for two reasons:** (1) **there is still no key-*generation* code anywhere** — `rotate_signing_key` rotates the database's record of which key is active to a `kid`/`public_jwk` its caller already computed; nothing in this repository ever generates an RSA keypair or derives a JWK from one. (2) **no shipping binary ever calls any function in this module** — the repository layer is proven correct in isolation against a real database, not proven wired into anything that serves traffic; this table only ever gets a row through a `sqlx::query` call made directly from a test |
| Merchant API keys — dropped (`0008_create-merchant-api-keys.sql`, dropped by `0009_drop-merchant-api-keys.sql`) | ⛔ | The Stripe-shaped `sk_live_`/`sk_test_` bearer-key design this table backed is reversed by [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md): `authkestra_op::sqlx_store::SqlxOpStore::find_client` hardcoded `token_endpoint_auth_method: None`/`jwks: None` on every row at the then-pinned `authkestra-op = "=0.3.4"`, so an OP-backed client registry could not serve `private_key_jwt`. **That premise is no longer true at the current pin (`=0.7.1`): both columns are persisted and read back (authkestra#287), proven here by migration `0013` and the `find_client` assertions in `authkestra_op_smoke.rs`.** ADR-0010's *decision* — merchant clients in YAML, no database-stored merchant identity — is unchanged; an ADR is superseded, never edited, and whether the now-available OP-backed registry should replace YAML is a maintainer question this pass raises and does not answer. Per this repo's hard-cutover rule, `0009` is a straight `DROP TABLE`, not a deprecation — nothing had ever read or written a row here (last pass's own note said so), and the two tests that proved this table's constraints were deleted in the same migration rather than left passing against a table that no longer exists. **A reader must not infer from ADR-0010's continued reference to this migration number, or from this row remaining in the table for historical clarity, that `merchant_api_keys` still exists — it does not.** See "Merchant auth" below for the model that replaces it |
| Merchant auth (`/v1`: `client_credentials` + `private_key_jwt`, [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md)) | 🟡 | Each merchant is meant to be a statically registered OAuth2 client with a `client_id` and **public** JWK in vpay's YAML config, authenticating via a signed `private_key_jwt` assertion — no API key, no database-stored secret. **Upgraded from ⛔ this pass: three of the pieces this flow needs now genuinely exist, each tested in isolation** — merchant clients are modelled and validated in config (`vpay_config::oauth::MerchantClient`, row above), a `ClientAssertionStore` implementation exists and is proven race-safe (`vpay_db::SqlClientAssertionStore`, row below), and generic bearer-token validation exists and is proven correct against a real JWKS (`vpay_api::resource_auth::JwtValidator` pinned to `Surface::Merchant`, its own row below). **Still ⛔ in the sense that matters most — nothing is connected to anything:** there is no `/v1` token endpoint (nothing issues a `client_credentials` + `private_key_jwt` access token), no `/v1` route at all, no request-auth middleware mounted on one, and — the piece that would actually wire the three proven parts together — no `ClientStore` converting a `vpay_config::oauth::MerchantClient` into an `authkestra_op::client::ClientRegistration` the OP could look up. No shipping binary constructs `SqlClientAssertionStore` or a `JwtValidator`. ADR-0010's own Consequences section still says "Not started" and that remains true for the flow as a whole, even though several of its prerequisites no longer are. **New this pass, on the *client* side of this flow: two merchant SDKs** (`sdks/rust` → crate `vpay-sdk`; `sdks/nodejs` → `@vpay/sdk`) implement the whole handshake — RS256 `private_key_jwt` assertion minting with the exact claim set the pinned OP verifies, the `client_credentials` token request, token caching with single-flight refresh, one re-auth on `401`, every planned `/v1` resource call form-encoded the way Stripe's SDKs do, the error envelope, and `Vpay-Signature` webhook verification — against the contract written down in [docs/flows/merchant-auth.md](flows/merchant-auth.md). See the "Merchant SDKs" section below for exactly what their tests prove; the short version is that the Rust SDK's assertions are accepted by `authkestra_op::client_assertion::verify_client_assertion` at `=0.7.1` (the real verifier, in a test), and that **neither SDK has ever completed a request against a running vpay, because no vpay serves `/v1`**. The server half of this row is as absent as before |
| Client-assertion replay protection (`oauth_client_assertion_jtis`, `0011_create-oauth-client-assertion-jtis.sql`) | 🟡 | Backs `authkestra_op::client_assertion::ClientAssertionStore::record_jti`, which neither of `authkestra-op`'s two shipped implementations can satisfy for vpay's deployment: `NoClientAssertionStore` fails closed unconditionally, and `MemoryClientAssertionStore` is single-process only (its own doc comment names exactly vpay's situation — multiple replicas — as needing "something shared... instead"). This table's `jti TEXT PRIMARY KEY` is the atomic single-use guard, meant to be used as `INSERT ... ON CONFLICT (jti) DO NOTHING` read via `rows_affected()`, never check-then-insert (the migration's own header comment explains the TOCTOU race a separate SELECT would reintroduce). Two dedicated tests in `postgres_smoke.rs` prove the constraint at the database level (`a_duplicate_client_assertion_jti_is_rejected_by_the_database`, `on_conflict_do_nothing_reports_zero_rows_affected_for_a_replayed_jti`). **New this pass: a real Rust implementation exists — `vpay_db::SqlClientAssertionStore`**, implementing `authkestra_op::client_assertion::ClientAssertionStore::record_jti` with exactly that `INSERT ... ON CONFLICT DO NOTHING` pattern, converting `authkestra-op`'s `chrono::DateTime<Utc>` boundary type to vpay's own `time::OffsetDateTime` convention explicitly at the crossing (`chrono_to_offset_date_time`, `client_assertion.rs`). **Proven race-safe, not just correct when called sequentially**: `concurrent_record_jti_calls_for_the_same_jti_yield_exactly_one_fresh_result` fires 10 concurrent `record_jti` calls with the same `jti` against a real Postgres and asserts exactly 1 reports fresh and 9 report replayed — the same shape of proof `authkestra-op`'s own `sqlx_store` tests use for `consume_code`. **Still marked 🟡, not ✅: no shipping binary ever constructs a `SqlClientAssertionStore`** — the test above proves the store is safe to use, not that anything uses it; see "Merchant auth" above. **Known, not handled:** there is still no cleanup job for expired rows (the worker job loop is still ⛔), so this table would grow unbounded once something starts writing to it in production |
| Disabled-clients kill switch (`disabled_clients`, `0012_create-disabled-clients.sql`) | 🟡 | An operator revocation mechanism for an OAuth client (dashboard or merchant `client_credentials`) that takes effect without a deploy — `client_id` plus a disable flag/reason, no credential and no identity of its own (YAML stays authoritative for identity; this table only ever *subtracts* access). Its uniqueness is proven by two tests in `postgres_smoke.rs`: `disabled_clients_accepts_an_insert` and `a_duplicate_disabled_client_id_is_rejected_by_the_database` (rejected specifically on the `client_id` primary key). **New this pass: query functions exist — `vpay_db::is_client_disabled`/`disable_client`/`enable_client`** (`vpay-db/src/disabled_clients.rs`), deliberately uncached (the module's own doc comment argues a cache would reintroduce the revocation delay this table exists to remove). `disabled_client_lookup_reflects_disable_and_enable` in `vpay-db/tests/repositories.rs` proves all three functions observe the same underlying table consistently against a real Postgres, including that `disable_client` is idempotent (a second disable of an already-disabled client updates `reason` without erroring) and `enable_client` is a no-op on a client that was never disabled. **Marked 🟡, not ✅: no code outside this module and its own tests calls any of these three functions.** There is no kill-switch check in any auth path, because there is no auth path at all yet — see "Merchant auth" above and "Dashboard auth" below |
| Dashboard auth (`/dash/v1` as an Authkestra OP) | 🟡 | Decision recorded in [ADR-0009](adr/0009-dashboard-oidc-provider.md), design in [docs/flows/dashboard-auth.md](flows/dashboard-auth.md). **Upgraded from ⛔ this pass, on the strength of the same three prerequisites "Merchant auth" above lists** — the dashboard client is now modelled and validated in config (`vpay_config::oauth::DashboardClient`), and `vpay_api::resource_auth::JwtValidator`/`AuthenticatedDashboard` pinned to `Surface::Dashboard` is proven to validate a correctly-audienced token and reject a merchant-audienced one on this surface specifically (`a_dashboard_audience_token_is_accepted_by_the_dashboard_validator`, `a_merchant_audience_token_is_rejected_by_the_dashboard_validator`, in `resource_auth.rs`). **Still no `/dash/v1` route, and a reader must not conclude login works from any of this**: no login has ever been performed, no token has ever been issued by this code, and no key has ever been rotated — `rotate_signing_key` (OP-2, row above) rotates to a key it is handed, it does not generate one. `authkestra-op`/`authkestra-engine`/`authkestra-axum`/`authkestra-resource` are pinned in the root `Cargo.toml`; `authkestra-resource` is now a genuine production dependency of `vpay-api` (for `JwtValidator`), and `authkestra-op`/`authkestra-engine` are production dependencies of `vpay-db` (for `SqlClientAssertionStore`, OP-2) — **so, unlike what this row used to say, `authkestra-*` is no longer dev-dependency-only; it is in both shipping binaries' resolved graph** (see the "Authkestra OP tables" row above and the "cargo deny" infrastructure row for the concrete consequence). What is still missing, unchanged in kind from before: no shipping binary ever constructs an `authkestra_op::sqlx_store::SqlxOpStore`, mounts `/dash/v1`, registers the dashboard OAuth client, or generates/loads a signing key from the Kubernetes Secret `oauth_signing_keys`'s own design assumes exists |
| Resource-server JWT validation (`vpay-api::resource_auth`, OP-3) | 🟡 | New this pass: `JwtValidator`, pinned per `Surface` (`Merchant` or `Dashboard`, distinguished by required `aud`), backed by `authkestra_resource::jwt::JwksCache` — fetched once and cached for `jwks_refresh_interval`, not a network round trip per request (confirmed by reading `authkestra-resource-0.3.4`'s own source, cited in the module doc, and re-confirmed unchanged at `0.7.1`: `JwksCache::get_key` still refreshes only on a cache miss or once the TTL has elapsed). `AuthenticatedMerchant`/`AuthenticatedDashboard` are axum extractors that pull a bearer token, validate it, and hand a handler `ResourceClaims { client_id, scope }`. **A real vulnerability class found and fixed, not merely inherited from the library:** `jsonwebtoken::Validation::validate_aud` defaults to `true` but its own doc comment says the check "only happens if `aud` claim is present" — a token minted with no `aud` claim at all would sail through unchecked. Fixed with `set_required_spec_claims(&["exp", "aud", "iss"])`, which makes the claim's mere presence mandatory before the membership check runs, and proven by `a_token_with_no_audience_claim_at_all_is_rejected`. 11 tests in `resource_auth.rs` cover this plus: a validly-signed token round-trips its claims and scopes; a token signed by a different key (same advertised `kid`) is rejected; an expired token is rejected; a merchant-audience token is rejected by the dashboard validator and vice versa (both directions proven, not assumed from one); an unrecognized `kid` is rejected rather than falling back to any available key; and, over a real axum router, a missing/malformed `Authorization` header and a valid bearer token each produce the right status and Stripe-shaped envelope. Every failure mode collapses to the same generic `invalid_token` response (`AuthRejection::InvalidToken`), deliberately, so the endpoint cannot be used as an oracle for *which* check tripped. **Marked 🟡, not ✅: this module validates tokens correctly in isolation but is mounted on no route.** `vpay-api::router` still only serves `/healthz` and the 404 fallback (see "HTTP surface" above); nothing in this crate's `lib.rs` references `resource_auth` outside its own module tests. See the next row for a startup hazard this module carries that no shipping binary has hit yet, precisely because nothing calls it yet |
| rustls `CryptoProvider` process default, for `authkestra_resource::jwt::Jwks::fetch` | 🟡 | **A known, documented landmine — given its own row rather than buried as a footnote on "Resource-server JWT validation" above, because it will panic in production on a path no test exercises.** `authkestra_resource::jwt::Jwks::fetch` builds a fresh `reqwest::Client` on every JWKS fetch, which eagerly constructs a rustls TLS config at build time — even for a plain-HTTP target — and **panics** unless a process-wide default `rustls::crypto::CryptoProvider` was already installed (confirmed by reading `authkestra-resource-0.3.4`'s source; the root `Cargo.toml`'s own comment on the `authkestra-*` pins names this exact prerequisite). **Still true at `0.7.1`, with one new lever:** `JwksCache::with_client` (authkestra#301) lets vpay hand the cache a `reqwest::Client` it built itself, so the JWKS fetch no longer *has* to construct a client at fetch time — but building that client without a process-default provider panics in exactly the same way, so `install_default()` in `main()` remains the fix; `with_client` only changes where the panic would fire. **`vpay-sdk` (`sdks/rust`) is the one `reqwest` consumer in this workspace that is proven *not* to need `install_default()`:** it builds its own `rustls::ClientConfig` (ring provider, vendored `webpki-roots`, `h2`/`http/1.1` ALPN) and hands it to `reqwest::ClientBuilder::tls_backend_preconfigured`, so reqwest never consults the process default — `sdks/rust/tests/tls.rs` builds a client in a process asserted to have no default provider and asserts the SDK installs none (a library must not claim a merchant's process-wide default). That is the pattern the server-side fix should copy rather than `install_default()` if it wants the same property. Today the only place in this repository that calls `rustls::crypto::ring::default_provider().install_default()` is `vpay-api/src/resource_auth.rs`'s own `#[cfg(test)]` module (`ensure_crypto_provider_installed`, gated behind a `std::sync::Once`) — that comment says outright "production code in this crate never calls this; whichever agent wires this module into a real binary must make the same call once in `main()`, before the first JWKS fetch." **Not yet a live bug, because nothing is live yet**: `JwtValidator` is mounted on no route (row above), so no shipping binary has ever called `Jwks::fetch` and this panic path has never actually fired outside the test process. It becomes a real, first-request startup panic the moment `/v1` or `/dash/v1` is mounted without also adding this call to `main()`. **This is a different, and already-resolved, question from whether `vpay-db` needs the same call** — it does not: `vpay-db/src/lib.rs`'s own module doc documents that `sqlx-core` builds its own rustls provider inline via `builder_with_provider` and never consults the process-wide default, so `vpay-db`'s Postgres TLS connections cannot hit this panic regardless of `install_default()`. The two crates are genuinely different here, not inconsistently handled — `vpay-db`'s omission is investigated and correct, `vpay-api`'s omission is investigated and still a gap |
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
| `@vpay/sdk` (`sdks/nodejs`, the Node merchant SDK) | 🟡 | See the "Merchant SDKs" section below for what its 126 tests prove. 🟡 rather than ✅ for one reason only: the server side of the contract it implements does not exist, so nothing has ever proven it against vpay itself |
| `pnpm -r test` sweep | ✅ | 136 tests total (126 `@vpay/sdk` + 3 `@vpay/tokens` + 4 `@vpay/api-client` + 3 `@vpay/ui`), all passing — was 10 before the Node SDK landed on 2026-09-02. Previously broken: `@vpay/e2e`'s `test` script ran `cypress run`, so the recursive sweep tried to launch Cypress and failed with no binary installed — `just ci` and the CI `web` job could never pass. Fixed by renaming that package's script to `e2e` (`frontends/tests/e2e/package.json`), which `pnpm -r test` no longer touches |
| Cypress e2e | 🟡 | 3 specs written against the compose stack. **Still never executed here** — now purely because the Cypress binary itself isn't installed (its CDN is unreachable from this sandbox), not because of the script-wiring bug above. Run `pnpm exec cypress install` on a machine that can reach the CDN, then `pnpm --filter @vpay/e2e run e2e` |

---

## Infrastructure

| Area | Status | Notes |
|---|---|---|
| `compose.yml` (Postgres + 2 WireMock rails) | 🟡 | Written; **still never started as a stack.** Docker itself works here — `backends/tests/integration` runs real `postgres:16-alpine` containers against it — but **Docker Hub is unreachable**, and `wiremock/wiremock:3.9.2` is not in the local image cache, so the two rail stubs cannot be pulled. Postgres is cached and does run |
| `compose.e2e.yml` (full stack) | 🟡 | Revised this pass; **still never run** — see below |
| `backends/Dockerfile` (musl → scratch) | 🟡 | Rewritten this pass; **still never built** — see below |
| `frontends/Dockerfile` | 🟡 | Rewritten this pass; **still never built** — see below |
| `deny.toml` | ✅ | `cargo deny check` passes clean: `advisories ok, bans ok, licenses ok, sources ok`. The three advisories that failed before were fixed by **upgrading dependencies, not by suppressing them** — see below. One advisory is explicitly ignored: **RUSTSEC-2023-0071** (Marvin Attack in `rsa`, no patched release, an unconditional dependency of `authkestra-engine` per [ADR-0009](adr/0009-dashboard-oidc-provider.md)), accepted deliberately with the reasoning recorded inline in `deny.toml`. **This entry was preemptive when added and now genuinely fires — and this pass found that the previous pass's own note on *how* it fires was already stale, before this note could even be written once.** The last pass said `authkestra-op`/`authkestra-engine` reached `rsa` only via `vpay-tests-integration`'s dev-dependencies, so "the exposure itself is still narrower than 'in production' ... no shipping binary pulls it in." **That is no longer true, independently re-run and confirmed for this update:** `vpay-db` added `authkestra-op` as a genuine, non-dev dependency this pass (for `SqlClientAssertionStore`, OP-2), and both `vpay-server` and `vpay-worker-bin` depend on `vpay-db`. `cargo tree -i rsa` now shows `rsa v0.9.10 ← authkestra-engine ← authkestra-op ← vpay-db ← vpay-api/vpay-server/vpay-worker-bin`, with no `(dev)` marker anywhere on that specific path (the pre-existing `vpay-tests-integration` dev-only path still exists too, unchanged, in parallel). `cargo deny -L info check advisories` still reports the same `note[advisory-ignored]`/`note[vulnerability]` pair it did before — nothing about the ignore mechanism changed, and `cargo deny check` still exits 0 with 0 errors, so this is **not a CI regression**. What changed is the honesty of this row's own claim about scope: `rsa`'s Marvin-Attack timing side-channel is now reachable from both shipping binaries' production dependency graph, not merely from a test-only crate, even though nothing in either binary calls into `rsa` yet (no shipping code path constructs anything from `authkestra-engine`/`authkestra-op` — see "Merchant auth"/"Dashboard auth" above). The original `deny.toml` comment's own reasoning for accepting the advisory (no patched release exists; RS256 has no alternative in this stack; `/dash/v1` is staff-only, not the merchant payment path) does not depend on which dependency edge is dev-only, so the acceptance itself still stands — only the "no shipping binary pulls it in" line needs correcting, which this row now does. Also bans `aws-lc-rs`/`aws-lc-sys` so a second rustls crypto provider cannot reappear. **New this pass:** `CDLA-Permissive-2.0` was added to the allow list, with its justification recorded inline — it covers `webpki-roots` (Mozilla's CA bundle, data not code), pulled in through `sqlx`'s `tls-rustls-ring` feature now that `vpay-db` is a non-dev dependency using it (root `Cargo.toml`'s own comment: previously latent in the workspace's pins, now actually reachable). `tls-rustls-ring` (vendored roots) was chosen deliberately over `tls-rustls-ring-native-roots`: the runtime image is `FROM scratch` ([ADR-0004](adr/0004-musl-mimalloc.md)) with no OS trust store for `rustls-native-certs` to read, so native roots would fail TLS to Postgres in the shipped image only, while passing locally and in CI where a trust store exists — exactly the kind of gap that would not be caught until a real deployment. `rustls-native-certs` does still appear in the dependency graph (via `bollard → testcontainers → vpay-testkit`), but only as a `[dev-dependencies]` chain — `cargo tree -i rustls-native-certs` shows every path terminating in a dev-dependency of `vpay-testkit`/`vpay-db`/`vpay-tests-integration`, never a shipping binary, independently confirmed for this update |
| GitHub Actions | 🟡 | Workflow written; **never executed** |
| `schemas/*.cstack` | 🟡 | **Syntax verified against real CrateStack 0.10.1** (and 0.7.10 / 0.7.8 before it); content remains a design sketch, excluded from the build graph — see below. **The migrations are now the authoritative schema, and this file has diverged from them on two constraints**: raw SQL in `backends/migrations/0002_create-providers.sql` and `0003_create-payment-intents.sql` expresses two `CHECK` constraints (`partial_refunds_imply_refunds`, `no_over_refund`) that CrateStack's grammar cannot — no `@@check(expr)` exists in 0.7.8, 0.7.10 or 0.10.1
(0.10.1's parser adds `@@sql`/`@@embedded_sql`/`@@server_sql` — for views, not
constraints — plus `@@paged`, `@@subscribe`, `@@audit` and `@@soft_delete`, none
of which is a cross-column constraint, and `cratestack-migrate` still gates
CHECK emission on a single field's validator). The `.cstack` file's own `GAP` comments on those two models now point at the migrations that implement them |

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

**2026-09-02:** `cargo deny check` had regressed to `advisories FAILED` on
`master` as the advisory database moved, not because of any vpay change:
RUSTSEC-2026-0258 (`h2` 0.4.15, unbounded empty DATA frames, reachable
through `reqwest`/`hyper` in every shipping binary) and a yanked `chacha20`
0.10.1 (dev-only, via `testcontainers → ferroid → rand`). Both fixed the same
way as before — `cargo update -p h2 -p chacha20` to 0.4.19 / 0.10.2 — with
`ignore` still holding only the `rsa` entry above. The `authkestra` upgrade to
0.7.1 added one new crate to the graph, `authkestra-crypto-util` (MIT OR
Apache-2.0); `cargo tree -i aws-lc-rs`/`-i aws-lc-sys`/`-i openssl-sys`/
`-i native-tls` all still report no match, so the single-`ring`-provider and
rustls-only invariants survived the bump.

**Also 2026-09-02, from the SDK work:** the workspace's `reqwest` pin was
`0.12`, and nothing had ever consumed it — every `reqwest` in the lock was
`0.13.4` via `authkestra-*`. `vpay-sdk` became the pin's first consumer,
which would have compiled two HTTP+TLS stacks (`cargo deny` warns on
duplicate majors but does not fail). The pin moved to `0.13` with
`rustls-no-provider` (0.13 has no `ring`-flavoured feature — its `rustls`
feature means aws-lc-rs, which `deny.toml` bans); `cargo tree -d` now shows
one `reqwest`, and the four "no match" checks above were re-run after the
move. The pre-existing duplicate `webpki-roots` (0.26 via `sqlx-core`, 1.0
via `hyper-rustls`) is unchanged and predates all of this.

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

Independently re-run against `cratestack 0.10.1` (installed via `cargo install
cratestack-cli --version 0.10.1 --locked` on 2026-09-02, the latest release on
crates.io that day), and against `0.7.10` and `0.7.8` before it — same output
verbatim from all three, so each release is a clean re-verification rather
than a claim inherited from an older run. The 0.10 grammar's new block
attributes are listed in the file's own header; none affects this file.

What this does and does not prove:

- **Syntax is verified.** Every scalar, attribute, relation and enum in the
  file parses and type-checks against the real CrateStack 0.10.1 grammar.
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
  syntax-verified against real CrateStack 0.10.1 and still excluded from the
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

## Merchant SDKs

Two client libraries for the merchant API, both implementing the wire
contract in [docs/flows/merchant-auth.md](flows/merchant-auth.md): the RFC
7523 `private_key_jwt` handshake, token caching, every planned `/v1`
resource, and `Vpay-Signature` webhook verification. They landed on
2026-09-02, ahead of any server route — **no vpay serves `/v1`, so neither
has ever completed a request against a real vpay.** Every claim below is
about what the tests prove against stubs and against the real Authkestra
verifier, and nothing more.

| SDK | Status | What is proven, and how strongly |
|---|---|---|
| Rust — `sdks/rust`, crate `vpay-sdk` (workspace member, `publish = false`) | 🟡 | **107 tests, 0 ignored**, run by `cargo nextest run --workspace` and therefore by `just ci`. **The assertion it mints is accepted by the real OP verifier** — `authkestra_op::client_assertion::verify_client_assertion` at the pinned `=0.7.1`, called directly in `tests/op_conformance.rs` against a `ClientRegistration` holding the matching public JWK, with `expected_audiences = [token_endpoint, issuer]` exactly as `handlers/token.rs` passes them — with and without a `kid`, and refused for a different keypair, a different `aud`, and a `kid` the key did not sign with. That is a **CI-gated** proof. Everything on the wire — token form fields, `Bearer` header, caching, single-flight refresh, the single 401 re-auth replaying the identical `Idempotency-Key` and body, each resource's exact path and form body, the Stripe-shaped error envelope, transport and timeout errors — is asserted byte-for-byte against a `wiremock` stub; the webhook verifier and form encoder are unit-tested. The README's Status section lists every source mutation that was run and which test each one fails. Cross-SDK parity is pinned by `src/form.rs` tests carrying the exact body string the Node encoder emits for the same parameters. TLS is built by the SDK itself (ring + vendored roots) and proven not to require or install a process-default provider (`tests/tls.rs`); **there is no live TLS test** — nothing here serves TLS, so certificate verification against the vendored roots is exercised by no test, and a merchant behind a private-CA proxy is not trusted. 🟡, not ✅, only because the server side does not exist |
| Node — `sdks/nodejs`, package `@vpay/sdk` (`private: true`, zero runtime dependencies, Node ≥ 22.11) | 🟡 | **126 tests, 0 skipped**, run by `pnpm -r test` and therefore by `just ci`; `pnpm --filter @vpay/sdk build` is a CI step too. The same wire assertions as Rust, against a real `node:http` server started by each test — never a mocked `fetch` — including the fake-timer expiry and short-TTL margin cases, five-way concurrent single-flight, the 401 retry replaying the same `Idempotency-Key` and body on a `POST`, path ids percent-encoded so `../../admin` or `pi_1#frag` cannot leave `/v1`, a stalled response body surfacing as `VpayTransportError` rather than a raw `DOMException`, amounts refused unless a non-negative safe integer, `exactOptionalPropertyTypes`-safe public types (a compile-time test), and every README code block type-checked against `dist/`. The assertion's RS256 signature is verified with `node:crypto` and its claim set pinned to exactly `aud, exp, iat, iss, jti, sub`. **Node cannot link the Rust verifier, so its real-OP proof is weaker than Rust's and must not be read as equivalent:** `just sdk-conformance-node` mints an assertion with the built Node SDK and pipes it into `sdks/rust/examples/verify_assertion.rs`, which runs the real `verify_client_assertion`. It is a manual recipe, **not part of `just ci`**. Last run 2026-09-02 09:19 UTC, on this tree: `verified: the pinned authkestra-op verifier accepted this assertion for client_id=merchant_a` (`jti=e6ff9a35-59a9-4663-bd0a-7316609e817e`, `exp=2026-09-02 09:20:37 UTC`), exit 0; the same recipe exits 1 for a wrong `client_id`, a wrong `aud`, and a single flipped signature byte, so it discriminates. Re-run it and update this line whenever `auth.ts` or the pinned `authkestra-op` changes |

**Decisions this work leaves to a maintainer, deliberately:**

- **The token endpoint path.** No ADR or code fixes it. Both SDKs default to
  issuer `{base_url}/v1/oauth` and token endpoint `{issuer}/token` (the path
  `examples/merchant-curl` has used all along) and make both configurable.
  When the server mounts the OP, either that default becomes true or the
  SDK defaults change — one line each.
- **`audience=vpay:v1`.** Both SDKs request it because
  `vpay_api::resource_auth::Surface::Merchant.audience()` requires it and the
  OP would otherwise mint `aud = client_id`. The string is marked provisional
  in `resource_auth.rs`; whoever wires issuance must add it to each merchant
  client's `allowed_audiences` or change both constants together.
- **Array encoding.** The SDKs send Stripe's indexed form (`k[0]=v`); the
  curl examples use `k[]=v`. The server must accept both, as Stripe does.
- **Whether ADR-0010's YAML-only merchant registry still stands** now that
  `authkestra-op` 0.7.1 persists `token_endpoint_auth_method`/`jwks` (see
  the "Merchant API keys" row). The SDKs are indifferent to it.

**Not done, stated plainly:** no `/v1` route, no token endpoint, no merchant
OP mount — the server half of the contract. Neither SDK is published
anywhere (`publish = false` / `private: true`) and should not be until a
server exists for it to talk to. `examples/merchant-node` now uses
`@vpay/sdk` and, like `examples/merchant-curl`, still cannot succeed
against a running vpay today.

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
   authenticated. **The credential model changed two passes ago and the old
   answer to this item is gone**, not just incomplete: `merchant_api_keys`
   (migration `0008`) is dropped (migration `0009`) and the design it backed
   is reversed by [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md) —
   there is no opaque key of any kind on `/v1` anymore. The replacement
   (`client_credentials` + `private_key_jwt`, merchant clients in YAML) has
   **three real, individually-tested prerequisites now, and no wiring between
   them**: merchant clients are modelled and validated in config
   (`vpay_config::oauth::MerchantClient`), a `ClientAssertionStore` exists and
   is proven race-safe (`vpay_db::SqlClientAssertionStore`), and bearer-token
   validation exists and is proven correct against a real JWKS
   (`vpay_api::resource_auth::JwtValidator`) — see "Merchant auth" above for
   all three. What is still entirely absent: no `/v1` token endpoint (nothing
   issues an access token), no `/v1` route of any kind, no request-auth
   middleware mounted on one, and no `ClientStore` converting a configured
   `MerchantClient` into an `authkestra_op::client::ClientRegistration` the OP
   could look up — that last piece is what would actually connect the three
   proven parts to each other. `oauth_client_assertion_jtis` (migration
   `0011`) now has a real reader/writer (`SqlClientAssertionStore`) where the
   previous pass's note said it had none. This item has moved forward from
   last pass's note, but "moved forward" here means "more individually-tested
   pieces exist," not "closer to serving a real request" — nothing routes a
   request through any of them yet. **2026-09-02: the *caller* of this item
   now exists** — the merchant SDKs (`sdks/`) speak exactly the handshake
   and resource contract this item must serve
   ([docs/flows/merchant-auth.md](flows/merchant-auth.md)), so whoever builds
   it has a conformance target rather than a blank page. That changes nothing
   about this item's own status. One more premise shifted underneath it: at
   `authkestra-op = "=0.7.1"` the OP's own `SqlxOpStore::find_client` now
   persists `token_endpoint_auth_method`/`jwks` (migration `0013`), so the
   YAML-only merchant registry ADR-0010 chose is no longer the *only*
   buildable option — whether to keep it is a maintainer decision this pass
   does not make.
5. Signed webhooks with the two-step outbox.
6. `just test-e2e` green against the compose stack.
7. `/dash/v1` login working end to end against a real database — issuing an
   access token, verifying it on a subsequent call, and rotating a signing
   key at least once. The schema this needs now exists (migrations `0006`,
   `0007`/`0010`, rows above) and is proven compatible with `SqlxOpStore`;
   `0010` additionally reshaped `oauth_signing_keys` so no private key
   material is ever persisted, closing a real risk the previous schema had.
   **This pass adds the same three prerequisite categories item 4 gained**:
   the dashboard client is modelled and validated in config
   (`vpay_config::oauth::DashboardClient`), a signing-key repository exists
   with a tested rotate operation (`vpay_db::signing_keys`, though nothing
   generates a key yet), and `JwtValidator` pinned to `Surface::Dashboard` is
   proven to enforce the audience separation from the merchant surface. That
   still does not move this item forward in the sense that matters: nothing
   in this repository has ever performed a login, issued a token, or rotated
   a key against a real deployment, no shipping binary constructs a
   `SqlxOpStore`, and no `/dash/v1` route exists at all — see "Dashboard
   auth" above. Not part of "does this take payments," included here because
   it is the other place this pass's work lands, and the same "individually-
   tested pieces ≠ a working feature" caution applies.

Until every one of those is ✅, this README's own claim is: **it does not take payments.**
