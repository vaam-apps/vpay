# STATUS

**What actually works today.** This page is the contract behind the repo's second
rule: *never advertise a feature as done when it clearly is not.*

It is machine-checked. `cargo xtask verify-status` scans the workspace for every
`ProviderError::NotImplemented("…")` token and fails the build if one is missing
from this file. You cannot quietly ship an unimplemented path. Since 2026-09-02
`cargo xtask verify-errors` likewise fails the build if an error type in
`backends/crates` is not classified per [ADR-0011](adr/0011-error-modelling.md),
or if `anyhow` leaks into a library crate.

Last verified: 2026-09-02, on branch `claude/step1-merchant-tokens` (the
"Step 1" merchant-token pass) against `master` at `8ace988`. **For the first
time, every suite ran on the authoring machine**: the rootless Docker daemon
that could not start containers was repaired mid-pass (a stale in-daemon
containerd talking to a newer shim; restarted with the other projects'
containers restored), so `cargo nextest run --workspace` here means the
container-backed suites too. With the toolchain pinned to `1.95.0`: `cargo
fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `just verify` (`verify-no-mocks` clean, `verify-status` `8
unimplemented item(s)`, `verify-errors` `11 error type(s), all classified` —
`SigningKeyError` and `HttpClientError` are new), `just verify-ignored` (`3
ignored, 33 test binaries, 421 total`), `cargo deny check` (`advisories ok,
bans ok, licenses ok, sources ok`), `pnpm -r typecheck`, `pnpm -r test` (136
passed), `RUSTDOCFLAGS="-D warnings" cargo doc` for `vpay-api`, `vpay-core`,
`vpay-worker`, `vpay-sdk`, `vpay-config` and `vpay-db` all clean. `cargo
nextest run --workspace` with `DOCKER_HOST` set: **418 passed, 0 failed, 3
skipped** (the three `#[ignore]`d conformance cases), run four times in a
row after every container-starting package was moved into the serialised
nextest group and `vpay_testkit::containers::start_postgres_with_retry`
landed — the earlier runs had lost one to three random tests per run to
rootlesskit's `bind: address already in use` on a host running ~24
unrelated containers. `just demo` ran green end to end against the
containerised stack (`compose.yml` + `compose.e2e.yml` + `compose.demo.yml`):
discovery and JWKS, a token whose claims are `iss={base}/v1/oauth`,
`aud=vpay:v1`, `sub=demo-merchant`, `exp=+900 s`, the 401 envelope without a
bearer, and the authenticated 404 `unknown_route`. The CI run for this
branch is the other half of the evidence and is named in the pull request.
**Everything else in this paragraph was measured directly for this note,
not reported from elsewhere.** `just verify`: ok — `verify-no-mocks` clean,
`verify-status` `8 unimplemented item(s)` (unchanged; this pass added no
`NotImplemented` token and removed none), `verify-errors` `10 error type(s),
all classified` (up from 9 — the new one is `vpay_api::op::keys::SigningKeyError`).
`just verify-ignored`: `3 ignored (expected 3), 32 test binaries (expected
32), 396 total (minimum 320)` — `expected_suites` moved 30 → 32 in this
pass, for the two new `vpay-tests-integration` binaries `client_store` and
`merchant_token_flow`. `cargo nextest run -p vpay-api -p vpay-config -p
xtask`: **161 tests run, 161 passed, 0 skipped** (74 `vpay-api`, 53 `vpay-config`, 34 `xtask`) — that is the whole
of this pass's unit coverage, and none of it needs Docker. `cargo nextest
list -p vpay-tests-integration -p vpay-db` enumerates **36 container-backed
tests** (25 in `vpay-tests-integration`, of which 6 are the new
`merchant_token_flow` suite and 2 the new `client_store` suite; 11 in
`vpay-db`, of which 5 are the new `ensure_active_signing_key` cases) —
**listed, not run**: the testcontainers bootstrap still cannot start a
container on this machine (same rootless-daemon fault the Step 0 note below
describes), so this note claims none of the 36 as passing.

**The evidence behind the merchant token flow, stated exactly, because every
🟡 in the merchant rows below rests on it.** The implementer ran the six
`merchant_token_flow` tests against a scratch database on an
already-running Postgres, bypassing the testcontainers bootstrap this
machine cannot run; **all six passed**. **The Docker-backed form of those
tests has not yet run in CI. The two `client_store` tests and the five new
`vpay-db` signing-key tests have not been run anywhere at all** — not here,
not in CI. So: the code is written and the tests that would fail if it broke
exist, but the only observation of the whole handshake working end to end is
one manual run, on one machine, outside the harness CI will use.

**What this pass adds is the merchant half of Phase 2 — a token a merchant
can actually obtain, and an authentication boundary in front of `/v1`:** the
merchant OP at `/v1/oauth` (token, discovery, JWKS), RS256 signing-key
generation/loading/activation, the `disabled_clients` kill switch enforced
on the one interception point every token request passes through, replay
protection wired into the live store, and every `/v1` path other than
`/v1/oauth` nested behind `AuthenticatedMerchant`. **It adds no `/v1`
business resource** — an authenticated `/v1` request gets the honest 404,
deliberately — **and no `/dash/v1` anything**: there is still no login, no
session store, and no dashboard route. The rows say which is which. The
"Step 0" operability pass's own note follows, unchanged: on branch
`claude/vpay-production-readiness-56b122` (the "Step 0" operability pass)
against `master` at `03d34cc`. On the authoring machine, with the toolchain
now pinned to `1.95.0`: `cargo fmt --all -- --check`, `cargo clippy
--workspace --all-targets -- -D warnings`, `just verify` (`verify-no-mocks`,
`verify-status` — `8 unimplemented item(s)` — and `verify-errors` — `9
error type(s)`), the new `just verify-ignored` (`3 ignored, 30 test binaries, 332 total`,
proven to fail at `expected_ignored=4`, `expected_suites=31` and `min_tests=400`), `cargo deny
check`, and `RUSTDOCFLAGS="-D warnings" cargo doc -p vpay-api` all clean.
`cargo nextest run` for `vpay-config` (48 passed, the one that loads the
real `config/application.yml`), `vpay-api` + `vpay-core` (64 passed) and
the two binaries (15 passed; **9 failed for environmental reasons** — every
one at the testcontainers `start_postgres` call, before a vpay binary is
spawned, because this machine's rootless Docker daemon cannot start a
container: `failed to start shim … unsupported protocol`; none was marked
`#[ignore]`). **The container-backed suites are therefore not claimed to
pass here.** The CI `rust` job on `ubuntu-latest` is the evidence for them
— it runs the whole workspace with a working daemon and last reported `320
passed, 3 skipped` on run `33626567174` — and the `e2e (compose)` job is
now the evidence for the images and the stack; see "GitHub Actions" under
Infrastructure for what this pass changed there and what is still pending
a green run. **What this pass adds is operability plumbing, not features:**
a CI workflow that can actually go green, a compose stack and image that
can actually boot, the rustls provider install both binaries were missing,
request ids on every API response, the pinned toolchain, and the
`verify-ignored` coverage guard — no route, no rail call, no job loop. The
error-modelling pass's own note follows, unchanged: on branch
`claude/error-modelling` (the
error-modelling pass, [ADR-0011](adr/0011-error-modelling.md)) rebased on
`master` at `985bd96` (the merge of the SDK/authkestra pass below).
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `just verify` (`verify-no-mocks`, `verify-status`, and the new
`verify-errors` — `9 error type(s), all classified`), `cargo deny check`,
`pnpm -r typecheck`, and `RUSTDOCFLAGS="-D warnings" cargo doc` for
`vpay-core`/`vpay-api`/`vpay-worker`/`vpay-sdk` all clean. `cargo nextest
run` over every crate that needs no container (`vpay-core`, `vpay-api`,
`vpay-worker`, `vpay-provider`, `vpay-ledger`, `vpay-config`, `xtask`,
`vpay-sdk`): **262 passed, 0 skipped**; the six new exit-code CLI tests in
the two binaries pass without Docker. The container-backed suites still
cannot run on the authoring machine (same daemon fault as the previous
pass, described below) and are not claimed to pass here — the previous
pass's CI run on `master` is the last evidence for them, and this pass
changed no code in `vpay-db` or the migrations. **What this pass adds is
the error model: the `Classify` seam and policy table in `vpay-core`, a
`Classify` impl on every leaf error, the `ApiError` and `JobError`
composites, exit codes from `Category` in both binaries, and the
`verify-errors` self-check — see the six "Error …" rows in the Backend
table and [docs/flows/errors.md](flows/errors.md); none of it serves a
request or moves money, and the rows say so.** The SDK/authkestra pass's
own note follows, unchanged: on branch `claude/sdk-rust-nodejs-0c1ecf`
against `8c0760e`, `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo xtask verify-no-mocks`, `cargo xtask
verify-status`, `cargo deny check` (`advisories ok, bans ok, licenses ok,
sources ok`), `pnpm -r typecheck`, `pnpm -r test` (136 passed) and
`RUSTDOCFLAGS="-D warnings" cargo doc -p vpay-sdk` all clean. `cargo nextest run --workspace`:
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
| Error classification seam (`vpay_core::error`, [ADR-0011](adr/0011-error-modelling.md), [docs/flows/errors.md](flows/errors.md)) | ✅ | `Category` (12 variants), `Retry`, `Severity`, the `Classify` trait and `find_in_chain`. The whole policy table — HTTP status, Stripe `type`, default `code`, retry, severity, public message, exit code — is one set of exhaustive `match`es on `Category`, pinned two ways: invariant tests over every category (caller categories are 4xx and system categories 5xx, only `Rail`/`Storage`/`RateLimited` retry after backoff, only `Internal` pages, every `type` is in Stripe's closed vocabulary, no generic message names anything internal, exit codes follow `sysexits`) **and** a literal transcription of [docs/flows/errors.md](flows/errors.md)'s twelve-row table, so the document and the code fail together; `Category::ALL` is proven complete by an exhaustive index function, so a thirteenth variant fails to compile there and fails the test if left out of `ALL`. 28 test functions in `vpay-core`. ✅ for what this row claims — the seam exists and its policy is proven — not for any request being answered through it (see the `ApiError` row) |
| Leaf errors classified (`MoneyError`, `UnknownCurrency`, `LedgerError`, `ConfigError`, `DbError`, `ProviderError`, `AuthRejection`) | ✅ | Each has an `impl Classify` next to its definition, with a comment per non-obvious choice (`DbError::Migrate` is `Configuration` not `Storage`; `ProviderError::Rejected` is `Conflict` with `Retry::NewAttempt`; `LedgerError::Unbalanced` pages). `ProviderError::Rejected` is the one that carries policy of its own: its envelope `code` is the constant `charge_declined` (an earlier draft reused the `FailureCode` string, which collided with `Transport`'s `provider_unavailable` at a different status), the `FailureCode` is in the public message, and its severity follows [docs/flows/failures.md](flows/failures.md)'s own table — `provider_account_blocked` pages, `provider_unavailable`/`provider_error` warn — proven exhaustively over all eleven codes. **Machine-checked**: `cargo xtask verify-errors` fails `just verify` and CI if any `pub` type under `backends/crates` that derives `thiserror::Error` **or** is named `*Error`/`*Rejection` lacks an `impl Classify` outside test code, or if a library crate lists `anyhow` under `[dependencies]` — proven live by deleting `UnknownCurrency`'s impl, by moving an impl into a `#[cfg(test)]` module, and by injecting `anyhow`, all three refused; it currently counts 9 types. Scope, stated plainly: `pub` types only, `backends/crates` only, `tests/` directories and `#[cfg(test)]` blocks excluded, line and block comments stripped; the SDKs and `backends/apps` are outside it by design |
| `vpay_api::ApiError` (HTTP composite) | 🟡 | `#[from]` every leaf the HTTP layer can meet (`DbError`, `ProviderError`, `MoneyError`, `UnknownCurrency`, `LedgerError`, `ConfigError`, `AuthRejection`) plus `UnknownRoute`/`InvalidParam`/`IdempotencyKeyReused`/`Internal`; axum's `Form`/`Json`/`Path`/`Query` rejections convert into `InvalidParam` with a curated sentence (a real `Form` extractor failing over a router yields the 400 envelope with `param: "body"`, never axum's plain text); no blanket `From<serde_json::Error>`, by design. `Classify` delegates all five methods to the leaf, pinned by a test asserting a wrapped leaf answers exactly as the bare leaf for `retry`/`severity`/`public_message` too. `IntoResponse` derives status, `type`, `code`, `message` and optional `param` from the classification and logs the full `Display` **and** source chain at the mapped `tracing` level (`alert = true` on `Page`) — a `DbError` carrying `host-secret-xyz` reaches the log and never the body (tested). `InvalidParam.param` must look like a field name (else `request`) and `message` is capped at 200 chars at render time; a 1 MB input yields a body under 1 KiB (tested). **The two envelope renderers are `pub(crate)`**, so a handler cannot build one by hand — one renderer is structural; `error_envelope` itself is now test-only (its pinned-shape test remains), and `IntoResponse` calls `error_envelope_with_param`. `AuthRejection` is classified and rendered through it; the 404 fallback is an `ApiError`; the pre-existing 404 and 401 envelope bytes are pinned unchanged. 29 test functions in `vpay-api`, 0 ignored. **Changed 2026-09-02: the 401 envelope is now reachable in a running `vpay-server`.** `AuthenticatedMerchant` is mounted in front of the `/v1` nest (see "HTTP surface"), so `ApiError::Auth` is produced by real traffic, not only by this module's own tests: `an_unauthenticated_v1_request_is_401_not_404` and `the_unauthenticated_v1_401_is_the_stripe_shaped_envelope` (`lib.rs`) drive it over the real router, and `a_v1_request_with_no_bearer_token_is_the_401_envelope` (`backends/tests/integration/tests/merchant_token_flow.rs`) does it over a socket against a booted server. Two reachable envelopes now, then: the 401 and the 404. `the_404_fallback_is_byte_for_byte_what_it_was_before_api_error` had to move its URI off `/v1` to keep testing the fallback at all — a `/v1` path with no token is a 401 now and never reaches it — and the pinned bytes are unchanged, because the envelope never echoed the path. **Still 🟡, for what is left rather than for what was**: every other variant (`DbError`, `ProviderError`, `MoneyError`, `UnknownCurrency`, `LedgerError`, `IdempotencyKeyReused`, `InvalidParam` from a body extractor) is still produced by no shipping handler, because no `/v1` business resource exists to produce one. `vpay-api` gained `vpay-config` and `vpay-ledger` as runtime dependencies for variants no handler can produce today. `vpay-config` was already in both binaries' graphs; `vpay-ledger` is a workspace crate that **neither binary linked before** and now both do (via `vpay-api` and `vpay-worker`). No third-party package is new to either binary, but `vpay-api`'s own graph now includes `clap`/`figment`/`garde`/`serde_yaml_ng`; `cargo deny check` still clean |
| `vpay_worker::JobError` (job-loop composite) | 🟡 | `Db`/`Provider`/`Money`/`Ledger` wrapped with all-five-method delegation, plus `Poisoned` (`Internal`) and `Exhausted` — the reconciler's `unresolved` state, which [docs/flows/reconciler.md](flows/reconciler.md) defines as "still polled, once an hour, and now raising an alert": `Rail`, `Retry::AfterBackoff`, severity `Error`, code `charge_unresolved`. `decision(attempt)` is a wildcard-free `match` on `Classify::retry` alone: `AfterBackoff → RetryAfter { delay, alert }` with `delay = poll_delay(attempt)` (or the documented hour, `UNRESOLVED_POLL_INTERVAL`, for `Exhausted`) and `alert = severity ≥ Error`; `NewAttempt → Terminal`; `Never → DeadLetter`. 12 test functions: a declined charge is `Terminal`, `NotImplemented` dead-letters, `Db::Connect` rides the ladder *and* alerts (Storage is severity `Error`), `Transport` rides it and wakes nobody, `Exhausted` retries hourly with `alert: true` at every attempt and is never a `DeadLetter`. **🟡 because nothing calls `decision()`**: the job loop is ⛔ (Poll ladder row); `JobError` has no consumer anywhere in the workspace and is the contract Phase 5 consumes |
| Binary exit codes (`vpay-server`, `vpay-worker-bin`) | ✅ | `main` returns `ExitCode`: on a startup error the full `anyhow` chain is printed to stderr and the code comes from the first classifiable leaf in that chain (`ConfigError` looked up before `DbError`, since a config naming a dead database is still a config problem), `Internal`/1 if nothing matched. Proven by subprocess tests that need no Docker: missing `--config` → 78, invalid config → 78, a closed Postgres port → 69 (the `sqlx` acquire timeout makes that test take ~5 s, documented on the constant). A mutation forcing `1` fails all six. The drain-timeout `exit(1)` on `vpay-server` is unchanged |
| `Money` — integer minor units, XAF zero-decimal | ✅ | 6 tests incl. cross-currency and over-refund rejection |
| Canonical failure taxonomy | ✅ | 3 tests |
| Charge / intent state + `ProviderFlow` | ✅ | 3 tests incl. live-xor-terminal exhaustiveness |
| Ledger balancing invariant | 🟡 | Types and `validate()` done + 3 tests. **Persistence not started** |
| Config guard rails (stub host, literal secret) | 🟡 | The two rules (`validate_host`, `validate_secret`) are unchanged and still directly unit-tested (the original 5 tests). They are now also exercised through real YAML loading: `a_livemode_config_with_an_http_host_is_rejected` and `a_livemode_config_with_a_literal_secret_is_rejected` in `vpay-config`'s `config.rs` drive them through `Config::load_with_env` against fixture files, not just as bare function calls. **DB reconciliation (boot-sequence step 4) still not started** — see "YAML config loading" below for what changed and what did not |
| YAML config loading (`vpay-config::Config::load`) | ✅ | Figment layers `application.yml` with an optional `application-{profile}.yml` overlay (same directory, `<stem>-<profile>.<ext>`); `${VAR}` placeholders are resolved by hand-rolled string scanning (figment's own `Env` provider does not interpolate inside YAML scalars) before typed deserialization, so an unresolved placeholder is a named, fatal error, never an empty string; validation runs `garde`'s structural derive, then the existing `validate_host`/`validate_secret` guard rules over every provider, then a currency-exponent-vs-canonical-table check, duplicate-code checks, and (new this pass) the OAuth-client rules below. 23 dedicated tests in `vpay-config/src/config.rs` cover all of that plus (see "Secret redaction" below) that neither `ProviderHost`'s nor the whole `Config`'s `Debug` output ever contains a credential value. **Upgraded from 🟡 to ✅ this pass, for the two reasons the previous note gave for withholding it — both are now closed and both are proven by an end-to-end subprocess test, not just a library-level one:** (1) **now wired into both binaries.** `vpay-server` and `vpay-worker-bin` both call `Config::load` before opening a database connection, and `--config`/`VPAY_CONFIG` is now required at the binary level (still `Option<PathBuf>` at the `clap` type level, exactly like `--database-url`) — proven by three subprocess tests per binary in each `tests/cli.rs`: a missing config is a non-zero exit naming `--config`/`VPAY_CONFIG` (`a_missing_config_is_a_non_zero_exit_naming_the_problem`), a config that fails validation is a non-zero exit (`a_bad_config_causes_a_non_zero_exit_naming_the_problem`), and a valid config lets the process boot and (for `vpay-server`) actually serve `/healthz` (`a_valid_config_lets_the_server_boot_and_serve_healthz` / `a_valid_config_lets_the_worker_boot`). (2) **merchant and dashboard OAuth clients are now modelled** — see `crate::oauth` (new this pass: `MerchantClient`, `DashboardClient`) and the new "Merchant OAuth clients" notes folded into this row below. **What is still explicitly out of scope, unchanged from before and stated in the module's own doc comment:** two boot-guard rules from [docs/flows/configuration.md](flows/configuration.md)'s table remain unimplemented on purpose, because they need a *payment-routing* `merchants` concept this config shape does not have — "every merchant's rail host is in the allowlist" and "every referenced provider exists and is enabled." An OAuth `MerchantClient`'s `client_id` is not that merchant concept and has no rail host to check. Boot-sequence step 4 (reconciling into the database in one transaction) is also still out of scope here. Neither gap weakens the claim this row actually makes — that `Config::load` loads, validates, and is used — so ✅ stands for that claim specifically |
| Merchant/dashboard OAuth client modelling (`vpay-config::oauth`, ADR-0010) | ✅ | New this pass, folded into the row above operationally but broken out here because it is a distinct piece of new modelling: `MerchantClient` (public JWK set, `client_credentials` only) and `DashboardClient` (redirect URIs, a single `scope` — enforced by the type being a `String`, not a `Vec<String>`), plus a closed local `GrantType` enum whose serde wire form matches `authkestra_op::client::GrantType`'s. Both carry a `client_secret: Option<String>` trap field that must always be `None`, with hand-written redacting `Debug` impls (5 tests in `oauth.rs`, including one proving a populated `client_secret` never appears in `{:?}` output). Seven boot-time validation rules run from `Config::validate_all`, each with a dedicated fixture-driven test asserting the *specific* `ConfigError` variant: duplicate `client_id` across merchants and the dashboard, an empty/keyless merchant JWKS, a merchant declaring a grant other than `client_credentials`, a dashboard client with no redirect URI, a non-`https` livemode dashboard redirect URI (reusing `validate_host`), and a client secret present anywhere (merchant or dashboard, tested separately). **This is authentication-client modelling only, not merchant *payment routing*** — see the row above's "still out of scope" note for exactly what that distinction means and does not cover |
| Secret redaction (`ProviderHost`/`CommonArgs` hand-written `Debug`) | ✅ | `ProviderHost` (rail credentials) and `CommonArgs` (`--database-url`, which routinely embeds a plaintext password) both hand-write `fmt::Debug` to redact secret values while keeping every other field, and credential *keys*, visible. `Config`, `ServerArgs` and `WorkerArgs` keep `#[derive(Debug)]` — safe because a derive formats each field via *that field's own* `Debug` impl, so the redaction composes upward without a second hand-written impl at every level. Six dedicated tests prove this holds, not just for the leaf types but through the composition: `provider_host_debug_output_never_contains_a_credential_value`, `a_whole_config_debug_output_never_contains_a_credential_value` (`vpay-config/src/config.rs`), `common_args_debug_output_never_contains_the_database_password`, `server_and_worker_args_debug_output_never_contains_the_database_password` (`vpay-config/src/cli.rs`), plus two more asserting the non-secret fields (rail code, host, `[redacted]` marker itself, `database_url: None` when unset) stay visible so the redaction does not silently swallow useful debugging signal. Marked ✅ because a re-derived `Debug` on either type — the exact regression these tests exist to catch — fails the build. **Residual risk stated, not hidden:** `ProviderHost::settings` and `::credentials` are both plain `BTreeMap<String, String>` and only `credentials` is redacted; a value accidentally placed in `settings` instead would leak in plaintext, and no test (and no type) can catch a value merely misclassified between the two maps — the boundary is enforced by convention, not by the type system |
| CLI / env configuration (`vpay-config::cli`) | 🟡 | `--version` reports `0.1.0`. Every option auto-resolves from an env var with an explicit flag winning, shared between both binaries via a flattened `CommonArgs`, covered by unit tests on the built `clap::Command` plus subprocess tests that set real env vars on a child process. **`--database-url` is no longer inert** — both binaries now treat it as required at runtime and use it to open a real connection pool and run migrations before serving (see "Database connectivity" below); it stays `Option<String>` at the clap type level, so the CLI itself does not enforce presence, only the two binaries' own startup logic does. **`--config` is no longer inert either, as of this pass** — both binaries now treat it as required at runtime too (same `Option<PathBuf>`-at-the-clap-level, required-in-`main.rs` pattern as `--database-url`), calling `vpay_config::Config::load` and refusing to start on a missing or invalid file; see the "YAML config loading" row above for the three subprocess tests per binary that prove this. **New 2026-09-02 (Step 1): `--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE`**, on `vpay-server` only — the worker issues no tokens, so mounting the Secret into it would widen its blast radius for no capability. Same `Option<PathBuf>`-at-clap, required-in-`main` pattern as `--config`, and required *before* the database connection, so all three failure modes exit `78` with no Docker needed: `a_missing_signing_key_flag_is_exit_78_naming_the_problem`, `a_signing_key_file_that_does_not_exist_is_exit_78_naming_the_path`, `a_signing_key_file_that_is_not_a_key_is_exit_78_without_echoing_its_contents`. The *path* is deliberately not redacted from `Debug` (a path is not a secret, and "which file did it try" is the first thing an operator needs); the file's contents never enter `ServerArgs` at all. Three unit tests in `cli.rs` pin that shape: `the_signing_key_file_flag_parses_to_the_path_it_was_given`, `the_signing_key_path_stays_visible_in_debug_output`, and `the_worker_is_not_handed_the_signing_key`. **`--public-base-url` remains the one flag still accepted and parsed but consumed by nothing, and Step 1 did not change that** — it is easy to assume otherwise now that `/v1/oauth` publishes an issuer, so to be exact: the issuer is `vpay_api::op::issuer_for(&config)`, which reads **`deployment.public_base_url` from the YAML config**, never the CLI flag. Grepping the workspace for `public_base_url` outside `vpay-config` finds only `vpay-api`'s uses of the config field. Two sources of the same idea, one of them inert, is a trap worth closing; nothing in this pass closes it. **A pre-existing gap this pass looked at and left alone:** a missing `--database-url` still exits `1`, not `78`, because `main` produces a bare `anyhow` error there with nothing for `exit_code_for` to classify — the `StartupError` introduced for the signing key covers only the signing key. Out of scope, and stated so nobody reads the new `78`s as meaning every missing flag is classified |
| Database connectivity (`vpay-db`: pool, migrations, healthcheck) | 🟡 | New crate this pass: `connect()` (a `PgPoolOptions` pool, max 10 connections, 5s acquire/connect timeout, eager — it does not return until at least one connection succeeds or the timeout elapses), `run_migrations()` (`sqlx::migrate!` against `backends/migrations`, idempotent by construction), and `check_connection()` (`SELECT 1`). All three are tested against a real `postgres:16-alpine` via testcontainers in `vpay-db/tests/postgres.rs`: `run_migrations_applies_cleanly_and_is_idempotent`, `check_connection_succeeds_against_a_live_database`, and `check_connection_fails_against_a_dead_database` (the container is stopped mid-test to prove the failure path, not just asserted by reading the code). Both `vpay-server` and `vpay-worker-bin` now call `connect()` then `run_migrations()` before doing anything else observable, and this happy path is proven end-to-end, not just at the crate level: `backends/apps/vpay-server/tests/cli.rs` spawns the real binary against a real testcontainers Postgres and polls `GET /healthz` until it returns **200** (`bind_and_log_format_env_vars_are_actually_applied` and others); `vpay-worker-bin`'s equivalent tests prove the same connect-then-migrate sequence via its startup log lines. **Marked 🟡, not ✅, because two specific claims this pass makes are implemented but not proven by any test:** (1) **"a missing `--database-url` is a hard startup failure"** — true by reading `main.rs` in both binaries (`args.common.database_url.as_deref().context(...)?`), but every subprocess test in both `tests/cli.rs` files always supplies `DATABASE_URL`; no test spawns either binary without it and asserts a non-zero exit. (2) **"`/healthz` returns 503 when the database is unreachable"** — true by reading `vpay-api/src/lib.rs`'s `healthz` handler, which maps a `check_connection` error to `StatusCode::SERVICE_UNAVAILABLE`, and `check_connection`'s own failure path is unit-tested in `vpay-db` (above) — but nothing kills the database mid-request and polls the real HTTP endpoint to observe a 503; the handler's status-code mapping itself is unexercised by any test |
| Provider port trait | ✅ | Interface defined; both adapters implement it |
| Process lifecycle (SIGINT/SIGTERM) | ✅ | `vpay-server` shuts down via `axum::serve(...).with_graceful_shutdown(...)` on SIGINT or SIGTERM instead of requiring `docker compose down` to SIGKILL it. `vpay-worker-bin` no longer exits immediately on boot — it stays up, answers the same signals, and logs a startup WARN banner plus a 60-second WARN heartbeat stating the job loop is not implemented and no jobs are being processed. **Startup race fixed this pass:** both binaries used to construct their shutdown-signal future late (inside `with_graceful_shutdown`'s argument, or just before the worker's select loop) — `tokio::signal::unix::signal(..)` and `tokio::signal::ctrl_c()` both install their OS-level handler on first *poll*, not at construction, so a SIGTERM delivered before that first poll (CLI parsing, tracing init, adapter-registry logging, `TcpListener::bind` all had to complete first) kept its default disposition and killed the process outright, skipping graceful shutdown and dropping any in-flight request. Confirmed by reproduction (`kill -TERM` sent tens of milliseconds after spawn reliably produced exit 143 with no shutdown log line) and by reading `tokio`'s own source (`signal_hook_registry::register` runs synchronously inside `tokio::signal::unix::signal`'s function body, not inside the future it returns). Fixed by `vpay_config::signal::ShutdownSignals`, a new type in `backends/crates/vpay-config/src/signal.rs` shared by both binaries (precedented by `CommonArgs` living in the same crate): `ShutdownSignals::install()` is now the first thing either binary's `main` does, before tracing init, registering SIGTERM/SIGINT handlers before any slower startup work can run. On Unix, SIGINT is now handled via `signal(SignalKind::interrupt())` rather than `tokio::signal::ctrl_c()` specifically because `ctrl_c()` is an `async fn` and would reintroduce the same late-installation race; non-Unix platforms still fall back to `ctrl_c()` inside `ShutdownSignals::wait()`, unchanged from before. A failure to install a handler is now a **hard startup failure** (`main` returns `Err`), not a logged warning that lets the process run its whole life with no graceful-shutdown path — deliberately stricter than before, since silently continuing would reintroduce the exact bug for the entire process lifetime rather than a brief window. Both binaries are exercised by subprocess tests that send a real `SIGTERM` and assert a clean exit (`backends/apps/vpay-server/tests/cli.rs`, `backends/apps/vpay-worker-bin/tests/cli.rs`), including a new regression test per binary (`sigterm_immediately_after_startup_still_triggers_graceful_shutdown`) that sends SIGTERM almost immediately after spawn and asserts both exit 0 and the graceful-shutdown log line. **That regression test's own limits, stated plainly:** it is a statistical majority-vote test (`ATTEMPTS`/`MIN_SUCCESSES` spawn-signal-wait trials), not a deterministic one, because the actual race window on modern hardware is on the order of a millisecond once other confounds (binary cold-start, CPU frequency ramp-up) are controlled for — verified in isolation to reliably fail against the pre-fix code and pass against the fix (repeated hundreds of times across macOS and a Linux container). But `cargo nextest run --workspace`'s real contention from ~20 concurrently running test binaries widens that window for *both* fixed and unfixed code enough that no single delay was both safe against the fixed binary and sensitive to the bug under full-suite load; the delay actually shipped (`DELAY = 50ms`) was chosen to never fail the full suite on correctly fixed code, at the cost of not reliably catching the bug when run as part of the full suite — its demonstrated sensitivity is strongest when run scoped/alone. This is disclosed in the test's own doc comment, not hidden. |
| `--shutdown-grace-seconds` bounded drain | 🟡 | On `vpay-server` this is now wired in: `serve_with_bounded_drain` in `backends/apps/vpay-server/src/main.rs` races the axum drain against a `shutdown_grace_seconds`-long clock and exits non-zero if the clock wins, logging that in-flight work was cut off. **No test exercises the timeout path itself** — the existing SIGTERM tests never have in-flight work to drain, so they would pass identically with the grace clock deleted; nothing here proves the bound actually holds under load. On `vpay-worker-bin` the flag is accepted and logged ("has no effect yet") but genuinely does nothing — there is no drain to bound because there is no job loop |
| Poll ladder | 🟡 | `poll_delay` done + 3 tests. **Job loop not started** |
| HTTP surface | 🟡 | **Changed 2026-09-02 (Step 1): the surface is no longer `/healthz` plus a 404.** `router(RouterDeps)` now serves, unauthenticated by necessity rather than omission, `GET /healthz`, `POST /v1/oauth/token`, `GET /v1/oauth/.well-known/openid-configuration` and `GET /v1/oauth/jwks.json` (`the_oauth_routes_are_reachable_without_a_token`); **every other path under `/v1` is nested behind the `AuthenticatedMerchant` extractor, and that nest's only route is the honest 404.** That pair of answers is the whole observable boundary: `GET /v1/payment_intents/pi_x` with no bearer token is a 401 envelope (`an_unauthenticated_v1_request_is_401_not_404`, `the_unauthenticated_v1_401_is_the_stripe_shaped_envelope`), and the same request with a valid merchant token is a 404 `unknown_route` (`an_sdk_client_authenticates_and_reaches_the_honest_404`, integration). **A 200 there would mean someone invented a resource, which is the failure this repo's `CLAUDE.md` names first — so `/v1` still implements no business resource at all, and this row stays 🟡 for exactly that reason.** The auth layer is `Router::layer`, not `route_layer`: `route_layer` does not wrap a fallback, and since this nest's only route *is* its fallback, axum refuses the build — verified by making the swap and watching every router test panic. That protection disappears the moment a real `/v1` route lands, which is why the choice is written down in `router`'s own doc comment. **Two known edges, both documented in code and neither fixed:** `GET /v1/oauth/token` (right path, wrong method) gets axum's bare `405` with an empty body instead of the Stripe envelope — the status is correct and turning it into the 404 envelope would be worse, so it waits for a `method_not_allowed` renderer for the whole surface; and `GET /v1/` (the bare trailing-slash form) falls through to the *outer* 404 rather than the nest's 401 (`the_bare_trailing_slash_form_of_v1_falls_through_to_the_outer_404` pins this as the current behaviour, not as desirable). What the previous pass changed, unchanged since: `/healthz` is no longer a static `"ok"` string. It runs `vpay_db::check_connection` (a real `SELECT 1`) and returns `200`/`"ok"` or `503`/`"database unreachable"` depending on the result — see "Database connectivity" above for exactly what is and is not tested about that mapping. The router's constructor argument changed with Step 1 from a bare `PgPool` to `RouterDeps { pool, merchant_op, merchant_validator }` — so a router cannot exist without a database connection *or* without the OP and the validator that guard `/v1`; there is no way to build a partially-wired one. **New 2026-09-02: request ids and a per-request span.** `router()` mounts, in this order, a guard that drops a caller-supplied `x-request-id` unless it is 1–64 bytes of ASCII `[A-Za-z0-9._-]` (removed, never rejected: a bad diagnostic header must not block a payment request — the caller merely loses the right to choose the id; one unusable value drops the whole header), tower-http's `SetRequestIdLayer` (mints a v4 UUID `x-request-id` unless the caller sent one the guard kept), a `TraceLayer` whose span records `method`, `path` and `request_id`, and `PropagateRequestIdLayer` (copies the id onto the response). Seven of `lib.rs`'s fourteen tests (the other seven cover the route tree above): a request with no id gets a UUID back, a caller-supplied id comes back unchanged, the `api error` line `ApiError` logs while serving a 404 carries the request id, a 4 KB id / an id with a space, `/`, `"` or a non-ASCII byte / a 65-byte id are each replaced by a minted UUID while a 64-byte one survives, and one unusable value among several drops the header — each proven decisive by disabling the guard, the span field or the propagate layer and watching the relevant tests fail. This is what makes `Category::Internal`'s "Contact support with the request id" a sentence a merchant can act on; `error.rs`'s "No `request_id` field here" section now points at the mounted layers instead of saying they are not mounted. `/healthz`'s plain-text body and the 404 envelope bytes are unchanged (pinned). **Security review the same day:** the `/v1/oauth` nest has its own explicit 404 fallback — measured, an unmatched path under it used to fall into the *authenticated* `/v1/{*rest}` route and answer 401, telling an integrator who mistyped an OP path to present a bearer token on the one subtree that issues them; `the_oauth_nest_answers_its_own_404` pins the 404 |
| Merchant OP (`/v1/oauth`) — `vpay_api::op` | 🟡 | New 2026-09-02. `MerchantOp` (`op/mod.rs`) assembles an `authkestra_op` provider whose issuer is `{deployment.public_base_url}/v1/oauth` (`issuer_for`, the single derivation in the workspace — `main` and `MerchantOp::new` both call it, so the `iss` a token is stamped with and the `iss` the validator pins cannot drift; `the_issuer_and_endpoints_are_what_the_sdk_derives_from_a_base_url` and `a_trailing_slash_on_the_public_base_url_does_not_change_the_issuer`). `grant_types_supported` is `["client_credentials"]` and nothing else. The store is a `CompositeOpStore` over `YamlClientStore` (row below), `SqlClientAssertionStore` for replay, and **three `SqlxOpStore<Postgres>` slots that serve no `/v1` grant** — they exist because `OpStore` is a supertrait of the code/refresh/device stores, and they are the real Postgres stores rather than an "always empty" stub because AGENTS.md rule 1 forbids a test double reachable from a shipping binary and a hand-written empty store would become a silent lie the day another grant is mounted. `op/token.rs` is thin: `token_handler` delegates to `authkestra_op::handlers::token::handle_token` and maps its RFC 6749 error JSON to a status (`invalid_client` → 401, everything else → 400); the discovery document is hand-built so it advertises only what this deployment serves (`discovery_publishes_the_endpoints_the_sdk_would_have_guessed`, `discovery_advertises_no_endpoint_this_deployment_does_not_serve`, `discovery_advertises_only_private_key_jwt`). **`authkestra-axum` is deliberately not a dependency** — its bundled router mounts `/authorize`, `/userinfo` and a one-key JWKS handler this deployment must not serve; see the JWKS row below. `op/jwks.rs` serves `/v1/oauth/jwks.json` from `vpay_db::publishable_signing_keys` (the whole rotation window, not just the active key), `Cache-Control: public, max-age=300` (`the_response_is_publicly_cacheable_for_the_documented_window`), skipping and warning about a row whose `public_jwk` is unusable rather than failing the whole document (`a_jwk_without_a_kid_is_skipped_and_warned_about`, `a_public_jwk_that_is_not_an_object_is_skipped_and_warned_about`, `a_kid_that_disagrees_with_its_row_is_published_but_warned_about`). 33 unit tests across `op::{mod,clients,keys,jwks,token}`. **Two numbers in here are defaults this pass chose, not decisions anyone recorded:** `ACCESS_TOKEN_TTL_SECS = 900` and `keys::ROTATION_OVERLAP = 24 h`. `docs/roadmap.md` lists both the access-token TTL and the rotation-overlap window as open questions and this pass does not close either; the only property that is *tested* is the relationship between them (`the_access_token_ttl_fits_inside_the_key_rotation_overlap`, `the_rotation_overlap_dwarfs_the_access_token_ttl_it_has_to_cover`), not that either value is right. **🟡 and not ✅** because the end-to-end proof — `the_jwks_and_discovery_documents_describe_this_process` and the rest of `merchant_token_flow` — has run exactly once, manually, against a scratch database, and never under Docker or in CI; see the header paragraph. **Not done here:** no rate limit on `/token` (left to ingress per [ADR-0009](adr/0009-dashboard-oidc-provider.md), and nothing in this repo checks that ingress actually does it), and no `/v1` resource for a minted token to reach.. **Known limitation, recorded not fixed:** no rate limit exists anywhere in this repository in front of `/v1/oauth/token` (one `disabled_clients` `SELECT` per request for any public `client_id`, before any signature check) — ADR-0009 leaves it to the ingress, which nothing here verifies |
| Database schema / migrations (core) | ✅ | Five migrations exist in `backends/migrations/` (`0001_create-currencies.sql` … `0005_create-ledger.sql`), applied via `sqlx::migrate!` to a real `postgres:16-alpine` (testcontainers) and asserted against in `backends/tests/integration/tests/postgres_smoke.rs`: a clean migration run on an empty database, the `one_charge_per_intent` unique index, two cross-column `CHECK` constraints firing (`partial_refunds_imply_refunds` on `providers`, `no_over_refund` on `payment_intents`), a plain `amount >= 0` check, an FK violation, and an out-of-range currency exponent. Marked ✅ and not 🟡 because the claim this row makes — "the schema and migrations exist, apply cleanly, and their constraints actually fire" — is fully implemented and tested; a broken migration or a dead constraint would fail a real test. **This is narrower than "the database works."** No route reads or writes an application row through this schema yet — that gap is now tracked by "HTTP surface" and "Database connectivity" above (a connection pool and a migration runner now exist and are wired into both binaries, closing the exact gap this row used to describe — "there is no connection pool" is no longer true, see those rows for what is and is not proven), the same way "Provider port trait" being ✅ above does not imply the adapters' wire calls work. **This repository now has twelve migrations in total** (`0001`-`0012`); this row covers only the first five — see the rows below for `0006`-`0012` |
| Authkestra OP tables (`0006_create-authkestra-op-tables.sql`, extended by `0013_add-authkestra-op-0-7-columns.sql`) | ✅ | `CREATE SCHEMA authkestra` plus `oauth_clients`, `oauth_codes`, `oauth_refresh_tokens`, `oauth_device_codes` — a byte-faithful transcription of the `CREATE TABLE` string literal hardcoded inside `authkestra-op` `=0.3.4`'s own `SqlxOpStore::migrate()` (not a vpay design; table/column names and types are not configurable — see the migration's header comment). **Upgraded to `authkestra-op = "=0.7.1"` this pass (from `=0.5.4`), and the re-diff the previous note demanded was done, not assumed:** `diff` over the extracted 0.3.4 and 0.7.1 crate sources shows the four tables 0006 creates are byte-identical, and 0.7.1's `migrate()` adds exactly one table (`authkestra.oauth_dpop_jti`, RFC 9449 DPoP replay tracking, authkestra#291) and three columns (`oauth_refresh_tokens.jkt`, `oauth_clients.token_endpoint_auth_method JSONB`, `oauth_clients.jwks JSONB`, authkestra#287). Migration `0013` transcribes those additions; it is **not optional** at this pin — `get_token`/`consume_token` now `SELECT … jkt` unconditionally and would fail at runtime against 0006's table alone. Proven compatible, not just transcribed correctly by eye: `backends/tests/integration/tests/authkestra_op_smoke.rs`'s `sqlx_op_store_round_trips_a_client_and_enforces_single_use_codes` drives the real `SqlxOpStore<Postgres>` against this schema end to end — inserts a client, `find_client` (JSONB columns decode through the store's own type, **now including `token_endpoint_auth_method` decoding to `TokenEndpointAuthMethod::PrivateKeyJwt` and `jwks` round-tripping as raw JSON**), `store_code`, `consume_code`, and asserts a second `consume_code` of the same code returns `None`, proving the crate's single-use `UPDATE … WHERE used = FALSE` actually fires here. Two new tests in the same file cover 0013's other additions through the store's own SQL: `sqlx_op_store_round_trips_a_refresh_token_with_its_jkt_column` (`store_token`/`get_token` round-trip `jkt`) and `sqlx_op_store_records_a_dpop_jti_once_against_migration_0013s_table` (`check_and_record_dpop_jti` accepts a fresh `jti` and refuses its unexpired replay). Neither refresh tokens nor DPoP are features vpay offers — see `docs/flows/dashboard-auth.md` — these prove schema compatibility with the pinned crate, nothing more. **Two API breaks absorbed in the same test file:** `AuthorizationCode` is `#[non_exhaustive]` since 0.6.0 (constructed via `AuthorizationCode::new` now), and `ClientRegistration::require_pkce` is deprecated since 0.7.0 because PKCE is unconditional on the authorization-code grant (authkestra#273) — the test no longer asserts on a field nothing reads. A second test in `postgres_smoke.rs` proves the `oauth_codes → oauth_clients` FK fires. `oauth_device_codes` is created even though vpay's login flow (PKCE only) never uses the device grant, because `SqlxOpStore` implements `DeviceCodeStore` unconditionally. **Marked ✅ for what this row claims — the DDL exists, matches the pinned crate, and is proven compatible against a real store — not for dashboard auth working.** No shipping binary constructs a `SqlxOpStore` or uses these tables — see "Dashboard auth" below. **Correcting a claim this row used to make, which this pass's dependency-graph check found stale:** it used to say `authkestra-op`/`authkestra-engine` were dev-dependencies of `vpay-tests-integration` only, with neither `vpay-server` nor `vpay-worker-bin` depending on `authkestra*` at all. That second half is no longer true — `vpay-db` added `authkestra-op` as a **production** dependency this pass (for `SqlClientAssertionStore`, OP-2), and both binaries depend on `vpay-db`, so `authkestra-op` (and, transitively, `authkestra-engine`) is now in both binaries' production dependency graph. `vpay-server`/`vpay-worker-bin` still do not name `authkestra*` directly in their own `Cargo.toml`s, but "depend on neither" is no longer an accurate description of the resolved graph — see the "cargo deny" infrastructure row for the concrete consequence (the `rsa` advisory's exposure is narrower than "dev-only" now claims). **Coupling risk:** this migration pair is pinned to `authkestra-op = "=0.7.1"` (root `Cargo.toml`) and must move in lockstep with it — the crate hand-builds SQL against these exact table/column names as string literals, so nothing type-checks a mismatch. Any future version bump of `authkestra-op` requires re-reading `sqlx_store.rs`'s `migrate()` block at the new version and re-diffing against this file before assuming compatibility still holds; the migration's own header comment says the same and this is not to be treated as a routine dependency bump |
| OAuth signing keys (`0007_create-oauth-signing-keys.sql`, reshaped by `0010_reshape-oauth-signing-keys.sql`) | 🟡 | vpay-owned table (authkestra ships no signing-key type, store, or rotation logic at any published version — confirmed by grepping `authkestra-op-0.3.4` and `authkestra-engine-0.3.4` source for `struct SigningKey`, `trait KeyStore` and `fn rotate`, with no hits). **Reshaped this pass: `private_key_pem TEXT` is dropped entirely and replaced with `public_jwk JSONB`; `id` is renamed to `kid`.** The decision (migration `0010`'s own header comment) is that the RS256 private key comes from a Kubernetes Secret via env at process boot and is parsed once by `authkestra_engine::TokenManager::new_asymmetric`, never persisted — so this table now stores only what `/jwks.json` needs to publish across a rotation window: the public half, its `kid`, and the validity window. **This corrects last pass's own note, which said the private key PEM was stored in plaintext and readable by anyone who could `SELECT` the column — that is no longer true; no private key material exists in this table or this repository at all.** The three constraints (partial unique index `one_active_signing_key`, `active_key_has_no_expiry`, `expiry_after_creation`, the last two renamed alongside the column) are proven to still fire *after* the reshape by the same dedicated tests in `postgres_smoke.rs`, updated to insert `kid`/`public_jwk` rather than `id`/`private_key_pem`. **New this pass: a Rust repository layer exists** (`vpay_db::signing_keys` — `publishable_signing_keys`, `active_signing_key_kid`, `rotate_signing_key`), tested against a real Postgres in `vpay-db/tests/repositories.rs` — `publishable_signing_keys_includes_active_and_unexpired_retired_but_excludes_expired` proves the `WHERE active OR expires_at > now()` overlap-window query keeps a just-retired key publishable and drops a long-expired one, and `rotate_signing_key_leaves_exactly_one_active_key` proves the one-transaction retire-then-insert both bootstraps cleanly (no prior active key) and rotates cleanly (an active key already exists), leaving `one_active_signing_key` intact either way. **Both of this row's previous reasons for 🟡 are now closed, 2026-09-02 (Step 1).** (1) **Key generation exists**: `cargo xtask gen-signing-key --out <dir>` writes a 3072-bit RSA PKCS#8 PEM, `0600`, refusing to overwrite — `a_generated_key_parses_back_off_disk_with_the_same_kid`, `the_key_file_is_only_readable_by_its_owner`, `it_refuses_to_overwrite_an_existing_key_file` (`.xtask`). `just gen-e2e-signing-key` is the openssl equivalent for the compose stack, so the CI e2e job needs no Rust toolchain. (2) **A shipping binary now calls this module.** `vpay_api::op::keys::LoadedSigningKey::from_file` parses the PEM into `authkestra_engine::TokenManager`, derives the `kid` as the RFC 7638 thumbprint of the public JWK — a function of the key, not of the file or the process (`the_kid_is_a_function_of_the_key_and_not_of_the_encoding_or_the_process`) — and cross-checks the JWK it publishes against `TokenManager::public_jwk`, so the key announced and the key signed with cannot diverge (`the_published_jwk_is_the_key_authkestra_signs_with`, `the_published_jwk_has_the_six_members_a_verifier_needs_and_a_self_consistent_kid`). Anything that is not an RSA private key, and anything under 2048 bits, is refused (`anything_that_is_not_an_rsa_private_key_is_refused`), and no error message or source chain echoes the PEM (`no_error_message_or_source_chain_echoes_the_pem`). `vpay-server` loads it **before** connecting to Postgres, so the three failure modes are testable without Docker and all exit `78`: `a_missing_signing_key_flag_is_exit_78_naming_the_problem`, `a_signing_key_file_that_does_not_exist_is_exit_78_naming_the_path`, `a_signing_key_file_that_is_not_a_key_is_exit_78_without_echoing_its_contents` (`backends/apps/vpay-server/tests/cli.rs`, subprocess). Activation goes through the new `vpay_db::ensure_active_signing_key`, which takes a Postgres advisory lock and does the whole read-decide-write in one transaction, so N replicas booting on the same Secret rotate once between them (`ensure_active_signing_key_bootstraps_is_idempotent_then_rotates_once`, `concurrent_ensure_active_signing_key_calls_with_the_same_kid_rotate_exactly_once`, `ensure_active_signing_key_refuses_to_reactivate_a_retired_kid` — a rollback to a retired `kid` is refused rather than silently resurrecting a key). **Still 🟡, for three new and smaller reasons, none of them "nothing calls it":** (a) **there is no rotation at runtime** — `TokenManager` holds exactly one key for the life of the process, so rotating means restarting with a new Secret; nothing re-reads the file, and no operator runbook describes the sequence; (b) the five `ensure_active_signing_key` tests are Docker-backed and **have not been run on any machine yet** (see the header paragraph) — the code is written and the tests exist, nothing has observed them pass; (c) the PEM is **not zeroized** — `LoadedSigningKey::from_file` reads it into a `String` that is dropped normally, so key bytes may linger in freed heap. That is a deliberate, stated limitation, not an oversight (`op/keys.rs`'s own module docs say so), and it is not fixed here. **Rollback to a retired key (security review 2026-09-02):** `ensure_active_signing_key` now refuses it with `DbError::SigningKeyRetired { kid, retired_at }` (`Category::Configuration`, so `vpay-server` exits 78 naming the kid and the retirement instant) instead of a raw duplicate-key SQL error — proven against a real Postgres by `ensure_active_signing_key_refuses_to_reactivate_a_retired_kid` and by `a_rollback_to_a_retired_signing_key_exits_78_and_a_dead_database_still_exits_69` in `vpay-server`'s `tests/cli.rs`. Re-activating a still-publishable retired key is deliberately *not* done — that is the rotation-policy decision [docs/roadmap.md](roadmap.md) leaves open; the operational consequence, that `kubectl rollout undo` after a rotation is a clean exit 78 rather than a degraded boot, is stated here on purpose. The `bootstraps_is_idempotent_then_rotates_once` test had never executed before this pass and failed on its first real run — it compared a nanosecond `OffsetDateTime` with the microsecond `TIMESTAMPTZ` read back; fixed by building the expected instant at microsecond precision, and all 12 `vpay-db` tests now pass on a real container on the authoring machine |
| Merchant API keys — dropped (`0008_create-merchant-api-keys.sql`, dropped by `0009_drop-merchant-api-keys.sql`) | ⛔ | The Stripe-shaped `sk_live_`/`sk_test_` bearer-key design this table backed is reversed by [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md): `authkestra_op::sqlx_store::SqlxOpStore::find_client` hardcoded `token_endpoint_auth_method: None`/`jwks: None` on every row at the then-pinned `authkestra-op = "=0.3.4"`, so an OP-backed client registry could not serve `private_key_jwt`. **That premise is no longer true at the current pin (`=0.7.1`): both columns are persisted and read back (authkestra#287), proven here by migration `0013` and the `find_client` assertions in `authkestra_op_smoke.rs`.** ADR-0010's *decision* — merchant clients in YAML, no database-stored merchant identity — is unchanged; an ADR is superseded, never edited, and whether the now-available OP-backed registry should replace YAML is a maintainer question this pass raises and does not answer. Per this repo's hard-cutover rule, `0009` is a straight `DROP TABLE`, not a deprecation — nothing had ever read or written a row here (last pass's own note said so), and the two tests that proved this table's constraints were deleted in the same migration rather than left passing against a table that no longer exists. **A reader must not infer from ADR-0010's continued reference to this migration number, or from this row remaining in the table for historical clarity, that `merchant_api_keys` still exists — it does not.** See "Merchant auth" below for the model that replaces it |
| Merchant auth (`/v1`: `client_credentials` + `private_key_jwt`, [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md)) | 🟡 | **The server half of this flow now exists.** A merchant is a statically registered OAuth2 client with a `client_id` and **public** JWK in YAML, authenticating with a signed `private_key_jwt` assertion; `vpay_api::op::clients::registration_for` is the conversion into `authkestra_op::client::ClientRegistration` this row spent two passes calling "the missing piece", and it is mechanical by design — `token_endpoint_auth_method: Some(PrivateKeyJwt)`, `client_secret_hash: None`, `redirect_uris: []`, and `grant_types` mapped from the config enum rather than hardcoded so `ConfigError::DisallowedMerchantGrant` stays observable (`the_conversion_maps_every_field_the_op_reads`, `the_conversion_maps_grants_it_is_given_rather_than_hardcoding_one`). **The registration is proven to be one the real verifier accepts, not one that merely type-checks:** `an_sdk_minted_assertion_verifies_against_the_registration_this_module_builds` mints an assertion with the shipping `vpay-sdk` (the merchant SDK itself, added as a `[dev-dependency]` — not a test double) and feeds it to `authkestra_op::client_assertion::verify_client_assertion` at the pinned `=0.7.1`; `an_assertion_signed_by_a_key_this_merchant_did_not_register_is_refused` is the negative control. The `vpay:v1` audience the three parties must agree on is now one constant, `vpay_config::MERCHANT_AUDIENCE`, returned by `Surface::Merchant.audience()`, and a deployment whose merchant cannot target it **refuses to boot** (`ConfigError::MerchantMissingV1Audience`, proven by `a_merchant_client_that_cannot_target_the_v1_audience_is_rejected` against a fixture that is verbatim what `config/application.yml` shipped until this pass, plus `the_example_config_registers_its_merchant_for_the_v1_audience` on the real file). End to end, over a booted server on a real database, `backends/tests/integration/tests/merchant_token_flow.rs` covers all six claims this row makes: a token is obtained by the SDK and reaches the authenticated 404 (`an_sdk_client_authenticates_and_reaches_the_honest_404`), no bearer is a 401 envelope (`a_v1_request_with_no_bearer_token_is_the_401_envelope`), a disabled client is `invalid_client`/401 with no restart (`a_disabled_client_is_refused_with_invalid_client_and_401`), a dashboard-audience token this same server signed is refused on `/v1` (`a_dashboard_audience_token_is_refused_on_v1`), JWKS lists exactly the active `kid` and discovery matches the URLs the SDK derived independently (`the_jwks_and_discovery_documents_describe_this_process`), and one assertion cannot be spent twice (`the_same_client_assertion_cannot_be_spent_twice`). **🟡 and not ✅, for one reason and it is about evidence, not code:** those six tests **have never run under Docker, here or in CI** — the only observation of them passing is the implementer's single manual run against a scratch database on an already-running Postgres (header paragraph). When the CI `rust` job runs them green, this row is ✅ and should say which run. **Separately not done, and not blocked on that:** there is no `/v1` business resource for a valid token to reach — an authenticated request gets the honest 404, deliberately — and no rate limit on `/token` (ADR-0009 leaves it to ingress; nothing here verifies ingress does it). One property that is real and easy to misread as a bug: **an access token already issued to a client stays valid for its remaining TTL after that client is disabled** — the kill switch acts on token *issuance*, which is what a stateless bearer token means, and `a_disabled_client_is_refused_with_invalid_client_and_401` builds a fresh SDK client precisely so it tests the endpoint rather than a cache. The *client* side of the flow — the two merchant SDKs, `sdks/rust` and `sdks/nodejs` — is unchanged this pass and described in the "Merchant SDKs" section below; what changed is that the contract they were written against is now served by something. |
| Client-assertion replay protection (`oauth_client_assertion_jtis`, `0011_create-oauth-client-assertion-jtis.sql`) | 🟡 | Backs `authkestra_op::client_assertion::ClientAssertionStore::record_jti`, which neither of `authkestra-op`'s two shipped implementations can satisfy for vpay's deployment: `NoClientAssertionStore` fails closed unconditionally, and `MemoryClientAssertionStore` is single-process only (its own doc comment names exactly vpay's situation — multiple replicas — as needing "something shared... instead"). This table's `jti TEXT PRIMARY KEY` is the atomic single-use guard, meant to be used as `INSERT ... ON CONFLICT (jti) DO NOTHING` read via `rows_affected()`, never check-then-insert (the migration's own header comment explains the TOCTOU race a separate SELECT would reintroduce). Two dedicated tests in `postgres_smoke.rs` prove the constraint at the database level (`a_duplicate_client_assertion_jti_is_rejected_by_the_database`, `on_conflict_do_nothing_reports_zero_rows_affected_for_a_replayed_jti`). **New this pass: a real Rust implementation exists — `vpay_db::SqlClientAssertionStore`**, implementing `authkestra_op::client_assertion::ClientAssertionStore::record_jti` with exactly that `INSERT ... ON CONFLICT DO NOTHING` pattern, converting `authkestra-op`'s `chrono::DateTime<Utc>` boundary type to vpay's own `time::OffsetDateTime` convention explicitly at the crossing (`chrono_to_offset_date_time`, `client_assertion.rs`). **Proven race-safe, not just correct when called sequentially**: `concurrent_record_jti_calls_for_the_same_jti_yield_exactly_one_fresh_result` fires 10 concurrent `record_jti` calls with the same `jti` against a real Postgres and asserts exactly 1 reports fresh and 9 report replayed — the same shape of proof `authkestra-op`'s own `sqlx_store` tests use for `consume_code`. **Wired 2026-09-02 (Step 1): `MerchantOp::new` passes a `SqlClientAssertionStore` to `CompositeOpStore::with_client_assertion_store`, so every `/v1` token request goes through it.** That it is genuinely wired — rather than merely constructed — is what `the_same_client_assertion_cannot_be_spent_twice` proves: one assertion is sent by hand twice (the SDK correctly mints a fresh one per request, which is exactly why the SDK cannot reach this case), the first exchange succeeds, and the second is refused `invalid_client`/401 while the assertion is still well inside its own lifetime and would verify perfectly on its own. Drop `with_client_assertion_store` and that test fails. **Still 🟡, for two reasons.** (1) That test is Docker-backed and has never run under Docker — one manual scratch-database run is all the evidence there is (header paragraph). (2) **There is still no cleanup job.** What landed instead is a stopgap and is labelled one in the code: `vpay_db::delete_expired_client_assertion_jtis` runs **once, at `vpay-server` boot**, non-fatally, which bounds the table at roughly "assertions since the last restart" rather than "assertions forever". It is not a timer, the worker job loop is still ⛔, and a long-lived process still grows this table monotonically. The sweep's own correctness is proven by reading rows back rather than trusting a count (`expired_client_assertion_jtis_are_swept_and_live_ones_are_kept`, `backends/tests/integration/tests/client_store.rs`) — **a test that has not been run anywhere yet**. **Known limitation, recorded not fixed (security review 2026-09-02):** the replay namespace is global — `jti` alone is the primary key and the upstream `record_jti` seam carries no `client_id` — so a merchant using low-entropy `jti`s could collide with or pre-spend another merchant's; [docs/flows/merchant-auth.md](flows/merchant-auth.md) now states `jti` MUST be a UUID v4 (both SDKs comply) and leaves the `(client_id, jti)` re-keying as a maintainer decision |
| Disabled-clients kill switch (`disabled_clients`, `0012_create-disabled-clients.sql`) | 🟡 | An operator revocation mechanism for an OAuth client (dashboard or merchant `client_credentials`) that takes effect without a deploy — `client_id` plus a disable flag/reason, no credential and no identity of its own (YAML stays authoritative for identity; this table only ever *subtracts* access). Its uniqueness is proven by two tests in `postgres_smoke.rs`: `disabled_clients_accepts_an_insert` and `a_duplicate_disabled_client_id_is_rejected_by_the_database` (rejected specifically on the `client_id` primary key). **New this pass: query functions exist — `vpay_db::is_client_disabled`/`disable_client`/`enable_client`** (`vpay-db/src/disabled_clients.rs`), deliberately uncached (the module's own doc comment argues a cache would reintroduce the revocation delay this table exists to remove). `disabled_client_lookup_reflects_disable_and_enable` in `vpay-db/tests/repositories.rs` proves all three functions observe the same underlying table consistently against a real Postgres, including that `disable_client` is idempotent (a second disable of an already-disabled client updates `reason` without erroring) and `enable_client` is a no-op on a client that was never disabled. **Enforced 2026-09-02 (Step 1), and in the one place where enforcing it is sufficient.** `vpay_api::op::clients::YamlClientStore::find_client` consults `is_client_disabled` — and `find_client` is step 1 of `authkestra_op`'s `handle_token_request`, the single point every token request passes through for every grant. That is not a convenience: reading the pinned `authkestra-op-0.7.1/src/handlers/token.rs`, `handle_client_credentials` takes the already-resolved registration and mints straight through `TokenManager`, consulting no store afterwards, so a kill switch enforced anywhere else would not be enforced at all on the one grant `/v1` uses. Three properties, three tests. A disabled client is reported as `Ok(None)` — "no such client" — so the token endpoint cannot be used as an oracle for whether a merchant exists but is suspended (`find_client_reflects_the_disabled_clients_kill_switch`, integration, which also proves disable and re-enable take effect on the next lookup with no restart). An unknown `client_id` — the shape every credential-stuffing attempt has — is answered from the in-memory YAML index and never reaches Postgres (`an_unknown_client_id_is_refused_without_touching_the_database`). **And a failed lookup fails closed:** a database error becomes `OpError::Storage`, which `handle_token_request` maps to `server_error`, so an outage produces no token rather than a token for a client that may have been revoked (`a_failed_kill_switch_lookup_refuses_a_known_client_rather_than_admitting_it` — returning `Ok(None)` there would have rendered as `invalid_client` and pointed an operator at the merchant instead of at Postgres). End to end: `a_disabled_client_is_refused_with_invalid_client_and_401`. **Still 🟡, for two reasons.** (1) Evidence: both integration tests are Docker-backed and neither has run under Docker (header paragraph). (2) The switch acts on **issuance only** — an already-issued token remains valid for the rest of its TTL, which is what a stateless bearer token means and what ADR-0009's revocation-gap open question is about; nothing in this repo shortens that window. `disable_client`/`enable_client` are still called by no shipping code — an operator flips the row by hand, and **no runbook documents the `disabled_clients`-plus-YAML dual authority yet** |
| Dashboard auth (`/dash/v1` as an Authkestra OP) | 🟡 | Decision recorded in [ADR-0009](adr/0009-dashboard-oidc-provider.md), design in [docs/flows/dashboard-auth.md](flows/dashboard-auth.md). **Upgraded from ⛔ this pass, on the strength of the same three prerequisites "Merchant auth" above lists** — the dashboard client is now modelled and validated in config (`vpay_config::oauth::DashboardClient`), and `vpay_api::resource_auth::JwtValidator`/`AuthenticatedDashboard` pinned to `Surface::Dashboard` is proven to validate a correctly-audienced token and reject a merchant-audienced one on this surface specifically (`a_dashboard_audience_token_is_accepted_by_the_dashboard_validator`, `a_merchant_audience_token_is_rejected_by_the_dashboard_validator`, in `resource_auth.rs`). **Still no `/dash/v1` route, and a reader must not conclude login works from any of this**: no login has ever been performed, no token has ever been issued by this code, and no key has ever been rotated — `rotate_signing_key` (OP-2, row above) rotates to a key it is handed, it does not generate one. `authkestra-op`/`authkestra-engine`/`authkestra-axum`/`authkestra-resource` are pinned in the root `Cargo.toml`; `authkestra-resource` is now a genuine production dependency of `vpay-api` (for `JwtValidator`), and `authkestra-op`/`authkestra-engine` are production dependencies of `vpay-db` (for `SqlClientAssertionStore`, OP-2) — **so, unlike what this row used to say, `authkestra-*` is no longer dev-dependency-only; it is in both shipping binaries' resolved graph** (see the "Authkestra OP tables" row above and the "cargo deny" infrastructure row for the concrete consequence). **Status unchanged 2026-09-02 (Step 1) — still 🟡, and the reader must not infer otherwise from the merchant rows above: no login has ever been performed and no `/dash/v1` route exists.** Two of this row's stated prerequisites did close, and they are worth naming precisely because they are the ones most easily mistaken for the feature. (1) **Signing keys and JWKS are real now**: a key is generated, loaded, announced in `oauth_signing_keys` and published at `/v1/oauth/jwks.json` across a rotation window — see the "OAuth signing keys" and "Merchant OP" rows. (2) A shipping binary does now construct `SqlxOpStore<Postgres>` — but as three slots the `OpStore` supertrait demands and **no `/v1` grant reaches**, not as anything serving `/dash/v1`. What is still missing, and it is the whole feature: **no `/login` route, no `/authorize`, no `/dash/v1` anything**; **no `SessionStore`** — `authkestra-engine` is pinned with `features = ["rustls-no-provider", "token", "session"]` and **without `sql-postgres`**, so no SQL-backed session store is even compiled in; and a design problem this pass surfaced but did not solve — `authkestra-op`'s `default_handle_authorization_code` mints the access token with `Some(client_id)` as the audience (`authkestra-op-0.7.1/src/handlers/token.rs`, step 7), with **no requested-audience path at all**, so a token from that grant would carry `aud = <client_id>` and `Surface::Dashboard.audience()` (`vpay:dash/v1`) would reject every one of them. Whoever builds `/dash/v1` has to resolve that first; the merchant surface does not hit it because `handle_client_credentials` *does* honour a requested audience. Rotation is also restart-based (the "OAuth signing keys" row), so "rotating a signing key at least once" — this flow's own definition of done — has still never happened |
| Resource-server JWT validation (`vpay-api::resource_auth`, OP-3) | 🟡 | New this pass: `JwtValidator`, pinned per `Surface` (`Merchant` or `Dashboard`, distinguished by required `aud`), backed by `authkestra_resource::jwt::JwksCache` — fetched once and cached for `jwks_refresh_interval`, not a network round trip per request (confirmed by reading `authkestra-resource-0.3.4`'s own source, cited in the module doc, and re-confirmed unchanged at `0.7.1`: `JwksCache::get_key` still refreshes only on a cache miss or once the TTL has elapsed). `AuthenticatedMerchant`/`AuthenticatedDashboard` are axum extractors that pull a bearer token, validate it, and hand a handler `ResourceClaims { client_id, scope }`. **A real vulnerability class found and fixed, not merely inherited from the library:** `jsonwebtoken::Validation::validate_aud` defaults to `true` but its own doc comment says the check "only happens if `aud` claim is present" — a token minted with no `aud` claim at all would sail through unchecked. Fixed with `set_required_spec_claims(&["exp", "aud", "iss"])`, which makes the claim's mere presence mandatory before the membership check runs, and proven by `a_token_with_no_audience_claim_at_all_is_rejected`. 11 tests in `resource_auth.rs` cover this plus: a validly-signed token round-trips its claims and scopes; a token signed by a different key (same advertised `kid`) is rejected; an expired token is rejected; a merchant-audience token is rejected by the dashboard validator and vice versa (both directions proven, not assumed from one); an unrecognized `kid` is rejected rather than falling back to any available key; and, over a real axum router, a missing/malformed `Authorization` header and a valid bearer token each produce the right status and Stripe-shaped envelope. Every failure mode collapses to the same generic `invalid_token` response (`AuthRejection::InvalidToken`), deliberately, so the endpoint cannot be used as an oracle for *which* check tripped. **Mounted 2026-09-02 (Step 1).** `AuthenticatedMerchant` is now the layer in front of the whole `/v1` nest (`vpay_api::router`, "HTTP surface" above), so this module is on the path of every merchant request, not only its own tests: `an_unauthenticated_v1_request_is_401_not_404` and `the_unauthenticated_v1_401_is_the_stripe_shaped_envelope` drive it over the real router, and `an_sdk_client_authenticates_and_reaches_the_honest_404` / `a_dashboard_audience_token_is_refused_on_v1` drive it over a socket against a booted `vpay-server`. The provisional `vpay:v1` string is gone: `Surface::Merchant.audience()` returns `vpay_config::MERCHANT_AUDIENCE`, so the validator and the config validation rule cannot disagree about the spelling. `AuthenticatedDashboard` remains mounted on nothing, because `/dash/v1` does not exist. **Still 🟡, for two reasons.** (1) The router-level tests that cover the mounted path are unit-level for the 401 and Docker-backed for everything past it, and the Docker-backed ones have not run under Docker (header paragraph). (2) **The validator fetches its JWKS over an HTTP round trip to this same process's own loopback port** — `vpay-server` binds first, then builds the validator with `loopback_jwks_url(bound)` (`the_validators_jwks_url_is_always_loopback_on_the_bound_port`, `the_validators_jwks_url_ends_at_the_route_the_router_mounts`, unit tests in the binary). It is always loopback, never the public URL, so no external dependency is introduced — but a process validating its own tokens by asking itself over TCP is a seam that exists because `authkestra_resource` offers no in-process key source, not because it is desirable. It also means the row below is no longer hypothetical. **Two findings from the security review, both fixed and pinned without Docker:** (1) an unauthenticated caller could force one loopback JWKS fetch (a Postgres `SELECT`) per request and hold the cache's write lock across it by presenting junk tokens with random `kid`s — `authkestra_resource`'s `JwksCache` refreshes on every miss; `JwtValidator` now decodes the header first (no `kid` → refused with zero cache access), delegates immediately for a `kid` already in the cached JWKS, and otherwise grants at most one refresh per `UNKNOWN_KID_REFRESH_INTERVAL` (30 s) per process — `a_hundred_unknown_kids_force_at_most_two_jwks_fetches` asserts wiremock saw ≤ 2 fetches for 100 junk tokens (101 with the throttle disabled), and `a_refused_token_does_not_spend_the_permit_for_a_good_one_on_the_same_key` pins that the predicate is membership of the published key set, not "validated before" — the stated cost is that a token signed by a key this replica has not yet fetched can be refused for up to 30 s during a junk burst; (2) a JWKS fetch failure (our own endpoint down because Postgres is down) was rendered as `401 invalid_token`, which the SDKs answer by re-authenticating — an outage amplifier; it is now `AuthRejection::KeysUnavailable`, `Category::Storage`, a 503 `service_unavailable` envelope with `Retry::AfterBackoff` (`a_jwks_that_cannot_be_fetched_is_keys_unavailable_not_invalid_token`, `a_jwks_outage_is_a_503_envelope_over_the_router`, with `a_bad_signature_is_still_a_401_over_the_router` as the control); every claim/signature/unknown-key failure still collapses to the oracle-free 401. Also pinned: a token the OP mints with no requested audience (`aud = client_id`) is refused on `/v1` (`a_token_for_the_client_id_audience_is_refused_on_the_merchant_surface`, and the Docker-backed `a_token_minted_with_no_audience_is_addressed_to_the_client_and_refused_on_v1`) — the decisive mutation is *widening* `set_audience`, not deleting it, since `jsonwebtoken` 11 fails closed on a missing audience list. **Found by the first `just demo` run, not by any test (2026-09-02):** inside the `FROM scratch` image `vpay-server` panicked at boot — `JwksCache::new` builds `reqwest::Client::new()`, which on the workspace's reqwest 0.13 pin loads trust roots from the OS store the image does not have (`No CA certificates were loaded from the system`), exactly the failure the root `Cargo.toml`'s comment on that pin predicted. The prescribed fix (`JwksCache::with_client`) was **not sufficient**: `with_client` replaces a client `new` has already constructed, and 0.7.1 (the latest release) has no other constructor. So `vpay_api::jwks_cache` is a deliberate, narrowed port of `authkestra_resource::jwt::JwksCache` + `validate_jwt_generic` (~15 lines of refresh policy; every cryptographic step still calls authkestra's `Jwks::fetch_with`, `Jwk::to_decoding_key` and `jsonwebtoken::decode`), taking the client as a constructor argument, and `vpay_api::http_client::client()` builds that client on vendored `webpki-roots` + `ring` via `tls_backend_preconfigured` — the twin of `sdks/rust`'s `rustls_client_config`. The module doc lists the deviations and the re-diff obligation on an authkestra bump; the clean answer is an upstream constructor that takes a client, after which the port can be deleted. **Proven three ways:** `a_server_with_no_os_trust_store_boots_and_still_validates_tokens` in `vpay-server`'s `tests/cli.rs` spawns the real binary with `SSL_CERT_FILE`/`SSL_CERT_DIR` pointing at nothing and asserts `/healthz` 200 and a bogus-`kid` `/v1` request answering the 401 envelope (a 503 would mean the fetch failed) — it fails with the original panic when `http_client::client()` is replaced by `reqwest::Client::new()`; the real image booted and answered the same two requests under `docker compose` on the authoring machine; and the CI `e2e (compose)` job exercises the same path. **Latent, stated:** `authkestra-engine` still writes `reqwest::Client::new()` in its device-flow, client-credentials-flow and captcha modules — none reachable from vpay today; if one ever becomes reachable it panics in the image the same way, and `install_crypto_provider` does not prevent that. **Remediation review, later the same day:** the port's `get_jwks` refresh re-checks the entry under the write guard (`refresh_if_stale`), so waiters that queued behind the first refresh at a TTL boundary reuse its result instead of fetching again — a fifth deviation from upstream, documented in the module. Measured before claiming: with the re-check deleted, 32 callers released on a barrier produced 1 extra fetch on 17 of 20 boundaries and 2 on 3, never 32 — `tokio`'s write-preferring `RwLock` was already doing most of the coalescing — so this removes an occasional redundant `SELECT` taken while every validation is queued, not an N-fold amplification. The concurrent form of the test passes with the bug present most of the time and was deliberately **not** shipped; `a_caller_that_reaches_the_refresh_with_a_fresh_entry_does_not_fetch_again` pins the property deterministically (1 fetch with the re-check, 2 without) |
| rustls `CryptoProvider` process default, for `authkestra_resource::jwt::Jwks::fetch` | ✅ | **Closed 2026-09-02.** Both `vpay-server` and `vpay-worker-bin` now call `rustls::crypto::ring::default_provider().install_default()` (`install_crypto_provider()` in each `main.rs`) as the second thing in `run()`, after the signal handlers and before tracing init, so no client construction can precede it. The result is `.ok()`-dropped on purpose — `Err` means a default already exists, which is the wanted state — per the root `Cargo.toml`'s own note on the `authkestra-*` pins; no `unwrap`/`expect`. **What the ✅ rests on:** a unit test per binary (`installing_the_crypto_provider_leaves_a_process_default_and_is_idempotent`) asserts `CryptoProvider::get_default()` is `Some` afterwards and that a second call does not panic — emptying the function's body fails both. The existing exit-69 subprocess tests spawn the real binaries through this call and on to the database stage, so it is exercised on a real startup. **Updated 2026-09-02 (Step 1): the path that used to panic now runs in a shipping process.** `vpay-server` builds a `JwtValidator` at startup, and the first authenticated `/v1` request makes it fetch its own loopback JWKS — a real `Jwks::fetch`, not a test one. **What proves it, and how strongly:** `an_sdk_client_authenticates_and_reaches_the_honest_404` and the rest of `backends/tests/integration/tests/merchant_token_flow.rs` boot the real router in-process and complete that fetch; the test binary installs the provider itself at the top of `harness()` for exactly the reason this row exists, and its own comment says so. That is an in-process exercise of the fetch, and it has run once, manually, against a scratch database — **never under Docker or in CI** (header paragraph). It is therefore stronger evidence than this row had before and weaker than "a shipping `vpay-server` container has served an authenticated request": the CI `e2e (compose)` job boots `vpay-server` but its Cypress spec only touches the dashboard, so no containerised `/v1` request has ever been made. The rail adapters are still `NotImplemented`, so no rail client has ever been built. The previous row's analysis — that `vpay-db` never needed this because sqlx builds its own provider inline, and that `sdks/rust` sidesteps it with a pre-built `ClientConfig` — is unchanged and still correct. **Scope narrowed 2026-09-02:** the JWKS client this row was written for is now built by `vpay_api::http_client` from a pre-configured rustls `ClientConfig`, so it no longer consults the process default at all (see the row above); the `install_default()` call stays in both binaries because `authkestra-engine`'s own `reqwest::Client::new()` call sites would need it if ever reached, and because it costs nothing |
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
| `@vpay/sdk` (`sdks/nodejs`, the Node merchant SDK) | 🟡 | See the "Merchant SDKs" section below for what its 126 tests prove. 🟡 rather than ✅ because **nothing has ever run it against vpay itself.** Its 126 tests all run against its own `node:http` stub; the `/v1/oauth` half of the contract now exists server-side (2026-09-02), but the integration suite that exercises it uses the *Rust* SDK, and `@vpay/sdk` is in no test that touches a vpay process. The only bridge to the real verifier remains `just sdk-conformance-node`, a manual recipe outside `just ci` |
| `pnpm -r test` sweep | ✅ | 136 tests total (126 `@vpay/sdk` + 3 `@vpay/tokens` + 4 `@vpay/api-client` + 3 `@vpay/ui`), all passing — was 10 before the Node SDK landed on 2026-09-02. Previously broken: `@vpay/e2e`'s `test` script ran `cypress run`, so the recursive sweep tried to launch Cypress and failed with no binary installed — `just ci` and the CI `web` job could never pass. Fixed by renaming that package's script to `e2e` (`frontends/tests/e2e/package.json`), which `pnpm -r test` no longer touches |
| Cypress e2e | ✅ | 3 tests in one spec, written against the compose stack, asserting only the dashboard's scaffold notice. **Never executed on the authoring machine** (the Cypress CDN is unreachable from it). **Executed for the first time by CI's `e2e (compose)` job on run `33647189156` (2026-09-02): `dashboard.cy.ts`, 3 tests, 3 passing, against the real compose stack** (see "GitHub Actions" below). The job downloads the binary during `pnpm install` (the workflow-level `CYPRESS_INSTALL_BINARY: 0` that used to prevent exactly that is gone — see "GitHub Actions" below), builds both images, waits for `/healthz` to answer 200, and runs the spec. Whether that job is green is the evidence for this row; read it, not this sentence |

---

## Infrastructure

| Area | Status | Notes |
|---|---|---|
| `compose.yml` (Postgres + 2 WireMock rails) | ✅ | Written; **never started as a stack on any authoring machine** (Docker Hub unreachable from one; the rootless daemon on another cannot start containers at all). **Started for the first time by the CI `e2e (compose)` job on run `33647189156` (2026-09-02)**, with both WireMock rails and Postgres up and `vpay-server` answering `/healthz` 200 against them — see "GitHub Actions" below. `config/application.yml` used to point both rails at a host named `wiremock`, which no compose file defines; fixed 2026-09-02 to `wiremock-mtn` / `wiremock-orange` (with Orange's `/orange-money-webpay/dev` path prefix, per its flow doc), proven by `vpay-config`'s own test that loads the real file |
| `compose.e2e.yml` (full stack) | ✅ | **Could not have booted before 2026-09-02**: both binaries exit 78 without `--config`/`VPAY_CONFIG` (mandatory since 2026-08-11), and this file never set it. Now sets `VPAY_CONFIG` and the three `${VAR}` rail placeholders (`MTN_SUBSCRIPTION_KEY`, `MTN_API_KEY`, `ORANGE_MERCHANT_KEY`, stub values for stub rails) on both services. **Run for the first time by CI's `e2e (compose)` job on run `33647189156` (2026-09-02)**: `vpay-server` answered `/healthz` 200 one second after the stack came up, the dashboard answered 200 on `/` a second later, and Cypress passed — see below. **Changed after that run, and therefore not covered by it (Step 1, same day):** `vpay-server` now exits `78` without an RS256 signing key, so this file sets `VPAY_OAUTH_SIGNING_KEY_FILE=/secrets/oauth-signing-key.pem` and read-only bind-mounts `.e2e/oauth-signing-key.pem` (git-ignored, `0644` because the scratch image runs as UID 65532, generated per stack by `just gen-e2e-signing-key` and thrown away with it; CI runs that recipe before `docker compose up`). **No CI run has yet booted the stack with that mount** — the ✅ above is evidence for the previous shape of this file, and the next `e2e (compose)` run is what proves the new one |
| `backends/Dockerfile` (musl → scratch) | ✅ | Last rewritten 2026-08-09 (musl host target, UID 65532). **Could not have produced a bootable image before 2026-09-02**: it never copied `config/` in, so there was no file for `VPAY_CONFIG` to name. Now bakes `config/` at `/config` and sets `ENV VPAY_CONFIG=/config/application.yml` in both runtime stages (secrets stay `${VAR}`, so the layer holds none). **A second reason it could never have built, found in review the same day:** the builder stage copied every workspace member except `sdks/rust`, and cargo refuses to load a workspace whose `members` list names a missing directory — proven by reconstructing the build context outside Docker and running the Dockerfile's own `cargo build`, which failed at manifest load; `COPY sdks/rust` added. **A third, found by the first CI build that reached the Docker step (run `33646048616`, the fix's own PR):** on the alpine builder the host triple *is* `x86_64-unknown-linux-musl`, and with no `--target` cargo applied `.cargo/config.toml`'s `+crt-static` rustflags to proc-macros too, which cannot be static (`cannot produce proc-macro for async-trait`). The build now passes `--target` set to the builder's own host triple (still never a cross-compile) and copies from `target/<triple>/dist/`; the Dockerfile's header comment explains it. It was the last reason: the next run, `33647189156`, built both images and booted the stack. **Built for the first time by CI's `e2e (compose)` job on run `33647189156` (2026-09-02), and the resulting `scratch` image booted, found its baked config, connected to Postgres, ran the migrations and answered `/healthz` 200.** Never built on an authoring machine — see below |
| `frontends/Dockerfile` | ✅ | Last rewritten 2026-08-09. **Never built anywhere yet**: not on an authoring machine, and CI's `e2e (compose)` job, which will build it, has never reached its Docker step. **Built for the first time by CI's `e2e (compose)` job on run `33647189156` (2026-09-02); the standalone Next server answered 200 on `/`.** Its build context had been checked beforehand the same way as the backend one — `pnpm install --frozen-lockfile --filter @vpay/dashboard...` against a reconstruction of exactly what it copies passes the lockfile consistency check with `examples/` and `sdks/nodejs` absent — see below |
| `deny.toml` | ✅ | `cargo deny check` passes clean: `advisories ok, bans ok, licenses ok, sources ok`. The three advisories that failed before were fixed by **upgrading dependencies, not by suppressing them** — see below. One advisory is explicitly ignored: **RUSTSEC-2023-0071** (Marvin Attack in `rsa`, no patched release, an unconditional dependency of `authkestra-engine` per [ADR-0009](adr/0009-dashboard-oidc-provider.md)), accepted deliberately with the reasoning recorded inline in `deny.toml`. **This entry was preemptive when added and now genuinely fires — and this pass found that the previous pass's own note on *how* it fires was already stale, before this note could even be written once.** The last pass said `authkestra-op`/`authkestra-engine` reached `rsa` only via `vpay-tests-integration`'s dev-dependencies, so "the exposure itself is still narrower than 'in production' ... no shipping binary pulls it in." **That is no longer true, independently re-run and confirmed for this update:** `vpay-db` added `authkestra-op` as a genuine, non-dev dependency this pass (for `SqlClientAssertionStore`, OP-2), and both `vpay-server` and `vpay-worker-bin` depend on `vpay-db`. `cargo tree -i rsa` now shows `rsa v0.9.10 ← authkestra-engine ← authkestra-op ← vpay-db ← vpay-api/vpay-server/vpay-worker-bin`, with no `(dev)` marker anywhere on that specific path (the pre-existing `vpay-tests-integration` dev-only path still exists too, unchanged, in parallel). `cargo deny -L info check advisories` still reports the same `note[advisory-ignored]`/`note[vulnerability]` pair it did before — nothing about the ignore mechanism changed, and `cargo deny check` still exits 0 with 0 errors, so this is **not a CI regression**. What changed is the honesty of this row's own claim about scope: `rsa`'s Marvin-Attack timing side-channel is now reachable from both shipping binaries' production dependency graph, not merely from a test-only crate, even though nothing in either binary calls into `rsa` yet (no shipping code path constructs anything from `authkestra-engine`/`authkestra-op` — see "Merchant auth"/"Dashboard auth" above). The original `deny.toml` comment's own reasoning for accepting the advisory (no patched release exists; RS256 has no alternative in this stack; `/dash/v1` is staff-only, not the merchant payment path) does not depend on which dependency edge is dev-only, so the acceptance itself still stands — only the "no shipping binary pulls it in" line needs correcting, which this row now does. Also bans `aws-lc-rs`/`aws-lc-sys` so a second rustls crypto provider cannot reappear. **New this pass:** `CDLA-Permissive-2.0` was added to the allow list, with its justification recorded inline — it covers `webpki-roots` (Mozilla's CA bundle, data not code), pulled in through `sqlx`'s `tls-rustls-ring` feature now that `vpay-db` is a non-dev dependency using it (root `Cargo.toml`'s own comment: previously latent in the workspace's pins, now actually reachable). `tls-rustls-ring` (vendored roots) was chosen deliberately over `tls-rustls-ring-native-roots`: the runtime image is `FROM scratch` ([ADR-0004](adr/0004-musl-mimalloc.md)) with no OS trust store for `rustls-native-certs` to read, so native roots would fail TLS to Postgres in the shipped image only, while passing locally and in CI where a trust store exists — exactly the kind of gap that would not be caught until a real deployment. `rustls-native-certs` does still appear in the dependency graph (via `bollard → testcontainers → vpay-testkit`), but only as a `[dev-dependencies]` chain — `cargo tree -i rustls-native-certs` shows every path terminating in a dev-dependency of `vpay-testkit`/`vpay-db`/`vpay-tests-integration`, never a shipping binary, independently confirmed for this update |
| GitHub Actions | ✅ | **Correcting this row, which said "never executed": by 2026-09-02 the `ci` workflow had run 13 times (2026-08-09 → 2026-09-02, every one on a pull request) and failed all 13** (`gh run list --workflow ci`; per-job conclusions from `gh run view`). Job by job: `self-checks` passed 13/13; `rust` passed 10/13 (failed on `31317876404`, `31319267218`, `33618568372`) — on the latest run, `33626567174`, it ran `cargo nextest run --workspace` on `ubuntu-latest` with a working Docker daemon, container suites included, and reported `320 passed, 3 skipped`, which is the evidence for every container-backed row on this page; `supply chain` passed 11/13; `web` passed only the last 2 (the `pnpm -r test` Cypress-script bug fixed in the SDK pass); **`e2e (compose)` failed 13/13** — the first eleven at `pnpm/action-setup@v4` (the `packageManager` conflict `bf9811d` fixed), the last two at `pnpm exec cypress install`, because the workflow set `CYPRESS_INSTALL_BINARY: 0` for *all* jobs and then asked Cypress to install. The Docker steps after that never ran once. Two more defects: `on.push.branches` said `main` (the default branch is `master`, so nothing ever ran on a merge), and the `rust` job's flow-style `{ components: rustfmt, clippy }` parsed `clippy` as a stray key. **All fixed 2026-09-02**: the e2e job downloads Cypress normally and verifies it, builds both images, polls `/healthz` for a 200 and the dashboard for a 200 before running the spec; the compiler version is read from `rust-toolchain.toml` (now pinned to `1.95.0`, matching `backends/Dockerfile`) in every Rust job of `ci.yml` and `docs.yml`; and a new `just verify-ignored` step fails the `rust` job if the ignored-test count is not exactly 3, the number of test binaries is not exactly 30 (the check that actually catches a binary dropping out — 18 of the 30 hold eight tests or fewer), or the suite shrinks below 320 tests. **`expected_suites` moved 30 → 32 on 2026-09-02 (Step 1)**, for the two new `vpay-tests-integration` binaries `client_store` and `merchant_token_flow`; the raised value has not yet been exercised by a CI run. **Run 14 of the workflow — `33647189156`, on this fix's own pull request (#14) — is the first green `ci` run in this repository's history**: all five jobs passed; the `rust` job reported `329 tests run: 329 passed, 3 skipped` and `verify-ignored: 3 ignored (expected 3), 30 test binaries (expected 30), 332 total`; the `e2e (compose)` job built both images (5 min 21 s), got `/healthz answered 200 after 1s` and `dashboard: / answered 200 after 2s`, and Cypress ran the one spec (`dashboard.cy.ts`, 3 passing). The run before it, `33646048616`, was the first ever to reach the Docker step and failed there — see "Docker / compose" below for the proc-macro finding it produced. **✅ as of run `33650294682` (2026-09-02), the first push-triggered run on `master` in the repository's history, triggered by the merge of #14 and green on all five jobs.** The claim this row makes — the workflow runs on the default branch and on pull requests, builds the images, boots the stack and runs every suite — would fail visibly if it broke, which is the bar for ✅ here |
| Local demo (`just demo`, `examples/merchant-demo`, `compose.demo.yml`) | 🟡 | New 2026-09-02. `just demo` generates a throwaway server signing key and a demo merchant keypair (`just gen-demo-keys`: `cargo xtask gen-signing-key` for the merchant, its public JWK written into a git-ignored `demo` profile overlay `.e2e/application-demo.yml` that `compose.demo.yml` bind-mounts beside the baked base config), brings up `compose.yml` + `compose.e2e.yml` + `compose.demo.yml`, waits for `/healthz`, and runs `cargo run -p merchant-demo` — a Rust binary using `vpay-sdk` that prints one line per step: discovery and JWKS, an access token's decoded claims (never the token), the 401 envelope without a bearer, and the authenticated 404 `unknown_route` for `payment_intents().retrieve(..)` with the sentence "payment intents are not built yet — this is where the next step lands". `compose.demo.yml` publishes no host port for Postgres (`ports: !reset []`) because 5432 is the most commonly occupied port on a developer machine and the demo never reaches Postgres from the host. **🟡, not ✅:** the demo is an assertion harness a human reads, not a test CI runs — nothing fails a build if it regresses; and it demonstrates authentication only, because no `/v1` resource exists yet. Its first run found the runtime-image panic recorded in the "Resource-server JWT validation" row, which is the kind of thing it exists to find |
| `schemas/*.cstack` | 🟡 | **Syntax verified against real CrateStack 0.10.1** (and 0.7.10 / 0.7.8 before it); content remains a design sketch, excluded from the build graph — see below. **The migrations are now the authoritative schema, and this file has diverged from them on two constraints**: raw SQL in `backends/migrations/0002_create-providers.sql` and `0003_create-payment-intents.sql` expresses two `CHECK` constraints (`partial_refunds_imply_refunds`, `no_over_refund`) that CrateStack's grammar cannot — no `@@check(expr)` exists in 0.7.8, 0.7.10 or 0.10.1
(0.10.1's parser adds `@@sql`/`@@embedded_sql`/`@@server_sql` — for views, not
constraints — plus `@@paged`, `@@subscribe`, `@@audit` and `@@soft_delete`, none
of which is a cross-column constraint, and `cratestack-migrate` still gates
CHECK emission on a single field's validator). The `.cstack` file's own `GAP` comments on those two models now point at the migrations that implement them |

### Docker / compose — made bootable, proven by CI run `33647189156`

Both Dockerfiles and `compose.e2e.yml` date from 2026-08-09 (musl host
target, non-root UID 65532, `.dockerignore`). Two days later `--config` /
`VPAY_CONFIG` became mandatory in both binaries, and nothing in the image
or the compose file supplied it — so from 2026-08-11 to 2026-09-02 the
"never built" stack was also a stack that could not have booted if it had
been. A previous version of this section said the files were "rewritten
this pass"; `git log` says otherwise, and this section now says what the
files actually are.

What changed on 2026-09-02, and what each change is proven by:

- `backends/Dockerfile` bakes `config/` into both runtime stages and sets
  `VPAY_CONFIG`. Proven by CI's `e2e (compose)` job on run `33647189156`.
- `compose.e2e.yml` sets `VPAY_CONFIG` and the three rail `${VAR}`
  placeholders on both services. `docker compose config` renders it, and
  the same CI run proves the processes boot behind it.
- `config/application.yml` names the real WireMock service hosts. Proven by
  `vpay-config`'s `a_valid_config_loads_and_produces_the_expected_typed_values`,
  which loads the real file.
- No `HEALTHCHECK` was added to the `scratch` image — there is still no
  executable in it that could run one, and the `--healthcheck` self-check
  mode `compose.e2e.yml` describes was not built. CI observes readiness
  from outside by polling `/healthz` instead.

**None of it was built on an authoring machine.** Docker Hub is unreachable
from one; on the other the rootless Docker daemon cannot start any
container (and, separately, `just build-dist` fails there at `ring`'s C
build for want of an `x86_64-linux-musl-gcc` cross compiler — the exact
cross-linker problem the Dockerfile's header describes, which does not
arise on the alpine builder because its `cc` is musl-native) (a containerd shim fault — the same one that fails the
container-backed test suites locally). The rows above cite CI run
`33647189156` (the pull request) and `33650294682` (the first run on
`master`) as their evidence; ✅ means exactly "the images build, the stack
boots, `/healthz` answers 200 and the one Cypress spec passes," and nothing
about what the stack can do once up — it still serves only `/healthz`.

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
resource, and `Vpay-Signature` webhook verification. They landed on 2026-09-02
ahead of any server route; later the same day the merchant OP landed, so
**the Rust SDK has now completed the handshake against a real
`vpay_api::router` (in-process, one manual run — see the header paragraph),
and the Node SDK still has not spoken to a vpay at all.** No vpay serves any
`/v1` *resource*, so neither SDK has ever completed a resource call. Every
claim below is about what the tests prove against stubs and against the real
Authkestra verifier, and nothing more.

| SDK | Status | What is proven, and how strongly |
|---|---|---|
| Rust — `sdks/rust`, crate `vpay-sdk` (workspace member, `publish = false`) | 🟡 | **107 tests, 0 ignored**, run by `cargo nextest run --workspace` and therefore by `just ci`. **The assertion it mints is accepted by the real OP verifier** — `authkestra_op::client_assertion::verify_client_assertion` at the pinned `=0.7.1`, called directly in `tests/op_conformance.rs` against a `ClientRegistration` holding the matching public JWK, with `expected_audiences = [token_endpoint, issuer]` exactly as `handlers/token.rs` passes them — with and without a `kid`, and refused for a different keypair, a different `aud`, and a `kid` the key did not sign with. That is a **CI-gated** proof. Everything on the wire — token form fields, `Bearer` header, caching, single-flight refresh, the single 401 re-auth replaying the identical `Idempotency-Key` and body, each resource's exact path and form body, the Stripe-shaped error envelope, transport and timeout errors — is asserted byte-for-byte against a `wiremock` stub; the webhook verifier and form encoder are unit-tested. The README's Status section lists every source mutation that was run and which test each one fails. Cross-SDK parity is pinned by `src/form.rs` tests carrying the exact body string the Node encoder emits for the same parameters. TLS is built by the SDK itself (ring + vendored roots) and proven not to require or install a process-default provider (`tests/tls.rs`); **there is no live TLS test** — nothing here serves TLS, so certificate verification against the vendored roots is exercised by no test, and a merchant behind a private-CA proxy is not trusted. **2026-09-02 (Step 1): this SDK now drives a real vpay**, not only a stub — `backends/tests/integration/tests/merchant_token_flow.rs` uses it as the client for the whole handshake, and `vpay-api` takes it as a `[dev-dependency]` so `an_sdk_minted_assertion_verifies_against_the_registration_this_module_builds` checks the shipping SDK against the registration the server builds from YAML. Still 🟡, now for two narrower reasons: those integration tests have never run under Docker or in CI (header paragraph), and every one of the eight `/v1` resource methods still has no route to call |
| Node — `sdks/nodejs`, package `@vpay/sdk` (`private: true`, zero runtime dependencies, Node ≥ 22.11) | 🟡 | **126 tests, 0 skipped**, run by `pnpm -r test` and therefore by `just ci`; `pnpm --filter @vpay/sdk build` is a CI step too. The same wire assertions as Rust, against a real `node:http` server started by each test — never a mocked `fetch` — including the fake-timer expiry and short-TTL margin cases, five-way concurrent single-flight, the 401 retry replaying the same `Idempotency-Key` and body on a `POST`, path ids percent-encoded so `../../admin` or `pi_1#frag` cannot leave `/v1`, a stalled response body surfacing as `VpayTransportError` rather than a raw `DOMException`, amounts refused unless a non-negative safe integer, `exactOptionalPropertyTypes`-safe public types (a compile-time test), and every README code block type-checked against `dist/`. The assertion's RS256 signature is verified with `node:crypto` and its claim set pinned to exactly `aud, exp, iat, iss, jti, sub`. **Node cannot link the Rust verifier, so its real-OP proof is weaker than Rust's and must not be read as equivalent:** `just sdk-conformance-node` mints an assertion with the built Node SDK and pipes it into `sdks/rust/examples/verify_assertion.rs`, which runs the real `verify_client_assertion`. It is a manual recipe, **not part of `just ci`**. Last run 2026-09-02 09:19 UTC, on this tree: `verified: the pinned authkestra-op verifier accepted this assertion for client_id=merchant_a` (`jti=e6ff9a35-59a9-4663-bd0a-7316609e817e`, `exp=2026-09-02 09:20:37 UTC`), exit 0; the same recipe exits 1 for a wrong `client_id`, a wrong `aud`, and a single flipped signature byte, so it discriminates. Re-run it and update this line whenever `auth.ts` or the pinned `authkestra-op` changes |

**Decisions this work left to a maintainer — three of the four are now
settled by the server, 2026-09-02 (Step 1):**

- ~~**The token endpoint path.**~~ **Decided: `{public_base_url}/v1/oauth`
  is the issuer and `{issuer}/token` is the token endpoint** — exactly what
  both SDKs already defaulted to, so no SDK default changes.
  `vpay_api::op::issuer_for` is the single derivation in the workspace,
  `MerchantOp::new` and `vpay-server`'s `main` both call it, and
  `the_issuer_and_endpoints_are_what_the_sdk_derives_from_a_base_url` pins
  it. The discovery document is compared against what the SDK derived
  independently, over a booted server, by
  `the_jwks_and_discovery_documents_describe_this_process` — so a merchant
  who never fetches discovery lands on the same URLs as one who does.
- ~~**`audience=vpay:v1`.**~~ **Decided, and no longer "provisional in
  `resource_auth.rs`":** the string is `vpay_config::MERCHANT_AUDIENCE`,
  `Surface::Merchant.audience()` returns that constant, and a deployment
  whose merchant registration cannot target it refuses to boot
  (`ConfigError::MerchantMissingV1Audience`, proven by
  `a_merchant_client_that_cannot_target_the_v1_audience_is_rejected` and by
  `the_example_config_registers_its_merchant_for_the_v1_audience` on the
  real `config/application.yml`). The "keep the two constants equal"
  instruction this bullet used to give is structurally unnecessary now:
  there is one constant.
- **Array encoding — still open, and still untestable.** The SDKs send
  Stripe's indexed form (`k[0]=v`); the curl examples use `k[]=v`. The
  server must accept both, as Stripe does — and it decodes neither today,
  because no `/v1` resource route parses a body at all.
- ~~**Whether ADR-0010's YAML-only merchant registry still stands**~~ now
  that `authkestra-op` 0.7.1 persists `token_endpoint_auth_method`/`jwks`.
  **Decided: kept.** `vpay_api::op::clients::YamlClientStore` resolves
  identity from `merchant_clients` in YAML and consults the database only
  for `disabled_clients`, which can subtract access but never grant it.
  The cost of that decision is unchanged and still real: merchant
  onboarding is a PR-then-deploy (ADR-0003, no hot-reload), and a rolling
  deploy has a window where old and new pods disagree about the client list.

**Not done, stated plainly:** neither SDK has been exercised against a
*deployed* vpay. What changed on 2026-09-02 is that the server half of the
contract exists — `/v1/oauth/token`, discovery and JWKS are served — and the
**Rust** SDK completes the whole handshake and reaches `/v1` in
`backends/tests/integration/tests/merchant_token_flow.rs`, against a server
booted in-process by the test, in the implementer's single manual run
against a scratch database (header paragraph): never under Docker, never in
CI, never against a container. **`sdks/nodejs` has still never spoken to a
vpay of any kind** — all 126 of its tests run against its own `node:http`
stub, and no integration test uses it. Neither SDK is published anywhere
(`publish = false` / `private: true`) and should not be until a `/v1`
resource exists for it to call: the SDKs implement eight resource endpoints
and the server implements none of them, so `examples/merchant-node` and
`examples/merchant-curl` can now *authenticate* against a running vpay and
will then get a 404 from every call they make.

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
   authenticated. **The *authenticated* half of this item is now built; the
   resource half is not started.** As of 2026-09-02 (Step 1) a merchant can
   obtain an access token from `/v1/oauth/token` with `client_credentials` +
   `private_key_jwt` and carry it past the authentication boundary — see the
   "Merchant auth" and "Merchant OP" rows above for the six integration tests
   that cover it, and the header paragraph for the one thing that keeps those
   rows at 🟡 (they have run once, manually, never under Docker or in CI).
   **What an authenticated `/v1/payment_intents` request gets today is a 404**,
   and that is the honest answer rather than a placeholder: no payment-intent
   route, no handler, no idempotency implementation, no form-decoding of a
   create body. Nothing about creating or confirming a payment intent moved
   this pass. The credential-model history this item used to recount still
   holds: `merchant_api_keys` (migration `0008`) is dropped (migration
   `0009`) and the design it backed is reversed by
   [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md) — there is no opaque
   key of any kind on `/v1`. The two decisions that were open on the wire
   contract are now decided *by the server*, not merely proposed by the SDKs:
   the token endpoint is `{public_base_url}/v1/oauth/token` and the audience
   is `vpay:v1` (see the Merchant SDKs section). The one ADR-0010 premise
   that shifted at `authkestra-op = "=0.7.1"` — that `SqlxOpStore::find_client`
   can now persist `token_endpoint_auth_method`/`jwks`, making a
   database-backed merchant registry buildable — was **resolved by keeping
   YAML**: `YamlClientStore` reads `merchant_clients` from the config file and
   the database only ever subtracts access via `disabled_clients`. That is
   ADR-0010's original choice, now implemented rather than merely recorded.
5. Signed webhooks with the two-step outbox.
6. `just test-e2e` green against the compose stack.
7. `/dash/v1` login working end to end against a real database — issuing an
   access token, verifying it on a subsequent call, and rotating a signing
   key at least once. **Two of those three verbs now have an implementation,
   and it belongs to the *merchant* surface, not this one.** Tokens are
   issued and verified on `/v1` (item 4); a signing key is generated,
   loaded, announced in `oauth_signing_keys` and published at
   `/v1/oauth/jwks.json`. **Nothing here has ever performed a login**, and
   the gap is not a matter of wiring the same parts to a second router:
   there is no `/login` or `/authorize` route, no `SessionStore` (
   `authkestra-engine` is pinned without its `sql-postgres` feature, so no
   SQL-backed session store is compiled in at all), and
   `authkestra-op`'s authorization-code handler mints `aud = <client_id>`
   with no requested-audience path, which `Surface::Dashboard`'s
   `vpay:dash/v1` would reject on every call — a design question whoever
   builds this must answer first. See the "Dashboard auth" row. **And
   "rotating a signing key at least once" is still unmet in the sense this
   item means it:** `ensure_active_signing_key` will rotate the database
   record when a process boots with a different key, but `TokenManager`
   holds one key for the life of the process, so rotation is restart-based,
   nothing re-reads the key file, and no rotation has been observed on any
   deployment. Not part of "does this take payments"; listed here because it
   is the half of Phase 2 this pass deliberately did not build.

Until every one of those is ✅, this README's own claim is: **it does not take payments.**
