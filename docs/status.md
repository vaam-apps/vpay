# STATUS

**What actually works today.** This page is the contract behind the repo's second
rule: *never advertise a feature as done when it clearly is not.*

It is machine-checked, and **since 2026-09-03 the check runs in both
directions**. `cargo xtask verify-status` scans the workspace for every
`ProviderError::NotImplemented("…")` token and fails the build if one is
missing from this file — *and* fails if this file declares a token that no
shipping code carries any more, so a section that outlives the code it
described cannot sit here unnoticed. The scanner is now comment-aware: a
`NotImplemented("…")` written inside a doc comment while explaining
something is no longer counted as a token (that blind spot is described in
the Step 2 note below, where it was found). `cargo xtask verify-no-mocks`
no longer greps the two app manifests: it walks `cargo metadata`'s
dependency graph from each shipping binary along non-dev edges only, so a
test double reachable through *any* intermediate crate is caught, and it
additionally refuses a test-only crate listed under any workspace member's
`[dependencies]` — with one narrow, documented allowlist
(`vpay-testkit` → `testcontainers`/`testcontainers-modules`, because
starting a real container is what that crate exists to do and ADR-0006 says
a stub rail **is** a WireMock host reached over HTTP). The Rust `wiremock`
crate — an *in-process* double — is allowlisted for nobody, which is how
`vpay-testkit`'s unused runtime dependency on it was found and dropped.
AGENTS.md's claim that `verify-status` "fails in both directions" is,
as of this pass, true. Since 2026-09-02 `cargo xtask
verify-errors` likewise fails the build if an error type in
`backends/crates` is not classified per [ADR-0011](adr/0011-error-modelling.md),
or if `anyhow` leaks into a library crate.

Last verified: 2026-09-03, on branch `claude/step3-rails` (the "Step 3" rails
pass) against `master` at `036b30c` — the merge of #16, which is Step 2's
`06a4280` on top of `0ac2a7f`. **Everything in this paragraph was measured
directly for this note, on the authoring machine, with the toolchain pinned
to `1.95.0` and `DOCKER_HOST=unix:///run/user/1000/docker.sock`.** `just
verify`: ok — `verify-no-mocks` clean, `verify-status` `1 unimplemented
item(s), all declared in docs/status.md and all still in shipping code`,
`verify-errors` `12 error type(s), all classified; anyhow confined to
binaries`. `just verify-ignored`: `0 ignored (expected 0), 35 test binaries
(expected 35), 718 total (minimum 640)`. `cargo nextest run -p vpay-provider
-p vpay-adapter-mtn-momo -p vpay-adapter-orange-money -p vpay-core`:
**156 tests run, 156 passed, 0 skipped** (11 `vpay-provider`, 48
`vpay-adapter-mtn-momo`, 53 `vpay-adapter-orange-money`, 44 `vpay-core`),
none of which needs Docker. `cargo nextest run -p vpay-config`: **70 run, 70
passed, 0 skipped**.

**The container-backed runs, which are what every 🟡 in the rail and confirm
rows below rests on.** `cargo nextest run -p vpay-tests-integration -p
vpay-db -p vpay-tests-conformance`: **115 tests run, 115 passed, 0 skipped**,
in 199 s, against real `postgres:16-alpine` and `wiremock/wiremock`
containers — 26 `vpay-tests-conformance` (4 capability cases plus 11 port
cases parameterised over both rails, each against a container started by
`vpay_testkit::containers::start_wiremock`), 37 `vpay-db`, and 52
`vpay-tests-integration`, of which **7** are the new `confirm_rails` suite
and 17 `payment_intents`.

`cargo nextest run -p vpay-api`: **165 run, 165 passed, 0 skipped**.

**What this note does *not* claim.** It measured 506 of the 718 listed tests.
`vpay-server`'s and `vpay-worker-bin`'s subprocess CLI suites, the Rust
SDK's, `xtask`, `vpay-worker`, `vpay-testkit` and `vpay-ledger` were
**listed, not run** for this note; the counts for them below are reported
from the pass's own gate run, not re-measured here. `cargo fmt`, `cargo
clippy`, `cargo deny`, `pnpm -r test` and `cargo doc` were likewise not
re-run here — `just ci` on this branch is the thing that would refute the
previous pass's results for them. **No CI run exists for this branch**, and
`just demo` was not run for this note; the demo row below says what was and
was not observed there.

**What this pass adds is the rails — and the thing to be clear about is what
"the rails" means here.** Both adapters make real HTTP calls: MTN's
`requesttopay` and status query, Orange's `webpayment` and
`transactionstatus`, each with a token cache, a documented failure mapping
and a callback parser. `POST /v1/payment_intents/{id}/confirm` now reaches
one of them over the network and moves the intent — to `processing` on a
push rail, to `requires_action` with a `next_action.redirect_to_url` on a
redirect rail, or back to `requires_payment_method` with a
`last_payment_error` and a `409` when the rail declines. **Every one of
those observations is against a WireMock host. Neither MTN's nor Orange's
real sandbox has ever been called by this code**, so what is proven is that
the adapters speak the protocol these documents describe — not that the
documents are right about the rails. Nothing polls a `submitted` charge, so
a confirmed intent stops at `processing`/`requires_action` forever and
**`succeeded` has still never happened**. The rows below say which is which.
The "Step 2" pass's own note follows, unchanged: on branch
`claude/step2-payment-intents` (the
"Step 2" payment-intents pass) against `master` at `0ac2a7f`. **Everything in
this paragraph was measured directly for this note, on the authoring machine,
with the toolchain pinned to `1.95.0`; nothing in it is reported from
elsewhere.** `just verify`: ok — `verify-no-mocks` clean, `verify-status`
`8 unimplemented item(s), all declared in docs/status.md`, `verify-errors`
`11 error type(s), all classified; anyhow confined to binaries`. **A scanner
blind spot found on this pass, stated rather than hidden:** for a while the
count read `9`, because `scan_not_implemented` in `.xtask/src/main.rs` does
not distinguish a doc comment from code and a doc comment in
`backends/crates/vpay-api/src/v1/boot.rs` spelled out `NotImplemented("…")`
while explaining why a different error is used there — and the check still
passed only because this page's own prose happens to contain an ellipsis.
The comment was reworded (the eight adapter tokens are unchanged — this pass
added none and removed none); making the scanner comment-aware, and making it
refuse a "declaration" that is not in the token list, is a follow-up.
`just verify-ignored`: `3 ignored (expected 3), 34 test
binaries (expected 34), 546 total (minimum 500)` at 01:20 UTC+2 and `548
total` at 01:33 — **and that drift is worth stating rather than smoothing
over.** This note was written while a concurrent remediation pass was still
editing `vpay-db::idempotency`, migration `0015` and the two
`payment_intents` suites on the same branch (it is adding a `claim_id`
column so an expired-then-reclaimed idempotency row cannot be overwritten by
the stale claim — an ABA fix), so every count in this paragraph is a
measurement of a moving tree. **The number to trust is whatever `just
verify-ignored` and `just ci` print on the commit**, not the ones written
here; they are recorded so a later reader can tell whether anything
*shrank*. `expected_suites` moved 33 → 34 for the new `payment_intents`
integration binary and `min_tests` 320 → 500 in the same change. `cargo nextest run -p vpay-api -p vpay-core -p
vpay-config`: **258 tests run, 258 passed, 0 skipped** (160 `vpay-api`, 41
`vpay-core`, 57 `vpay-config`), none of which needs Docker.

**And, for the first time in this repository's history, the container-backed
payment tests were observed passing on the authoring machine.** With
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, `cargo nextest run -p
vpay-db -p vpay-tests-integration`: **74 tests run, 74 passed, 0 failed, 0
skipped**, in 125 s, against real `postgres:16-alpine` containers — measured
at 01:20, i.e. *before* the concurrent ABA fix described above; that fix
touches exactly these two packages, so this run must be repeated on the
commit. That is 32
in `vpay-db` (24 of them in `tests/repositories.rs`) and 42 in
`vpay-tests-integration` — 16 `payment_intents` (new this pass), 14
`postgres_smoke`, 7 `merchant_token_flow`, 3 `authkestra_op_smoke`, 2
`client_store`. Every 🟡 and ✅ in the payment-intent, idempotency and
reconciliation rows below rests on that run.

**What this note does *not* claim.** It measured 332 of the 546 listed tests.
The other 214 — `vpay-server`'s and `vpay-worker-bin`'s subprocess CLI
suites, the Rust SDK's 107, `xtask`, `vpay-worker`, `vpay-provider`,
`vpay-ledger`, and the adapter-conformance suite that holds all 3 `#[ignore]`s
— were **listed, not run** for this note. `cargo fmt`, `cargo clippy`,
`cargo deny`, `pnpm -r test` and `cargo doc` were likewise not re-run here;
the previous pass's results for them stand and `just ci` on this branch is
the thing that would refute them. **No CI run exists for this branch**, and
`just demo` was not run for this note either — the demo row below says what
was and was not observed there.

**What this pass adds is the first `/v1` business resource, and it stops at
the rail on purpose.** `POST/GET /v1/payment_intents`,
`GET /v1/payment_intents/{id}`, `POST …/confirm` and `POST …/cancel` are
served; create, retrieve, list and cancel return real objects from real rows,
and **`confirm` reaches the rail adapter and answers `501 not_implemented`**,
because no adapter implements `submit`. Nothing here has taken a payment,
nothing has called a rail, and no payment intent has ever reached
`processing`, `requires_action` or `succeeded`. The rows below say which is
which. The "Step 1" pass's own note follows, unchanged: on branch
`claude/step1-merchant-tokens` (the
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
subprocess tests (`a_missing_config_is_exit_78_naming_the_problem`,
`a_bad_config_is_exit_78_naming_the_problem`,
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
> cannot take a payment. Do not deploy it.
>
> *2026-09-03 (Step 2): `/v1/payment_intents` now exists and answers real
> requests with real rows.*
>
> *2026-09-03 (Step 3): the sentence "no HTTP call to any rail has ever been
> made by this code" is **retired** — both adapters now make real HTTP calls,
> and `confirm` moves an intent to `processing` or `requires_action` on the
> strength of one. **Its replacement is narrower and just as load-bearing:
> no HTTP call to a real rail has ever been made.** Every call has gone to a
> `wiremock/wiremock` host. No payer has been prompted, no money has moved,
> nothing polls a submitted charge, and no intent has ever reached
> `succeeded`. Do not deploy it.*

---

## Backend

| Area | Status | Notes |
|---|---|---|
| Workspace, edition 2024, resolver 3 | ✅ | `cargo check --workspace --all-targets` clean |
| Lint policy (no `unwrap`/`expect`/`panic`/float in prod) | ✅ | `cargo clippy -- -D warnings` clean; tests exempted via `clippy.toml` |
| Error classification seam (`vpay_core::error`, [ADR-0011](adr/0011-error-modelling.md), [docs/flows/errors.md](flows/errors.md)) | ✅ | `Category` (12 variants), `Retry`, `Severity`, the `Classify` trait and `find_in_chain`. The whole policy table — HTTP status, Stripe `type`, default `code`, retry, severity, public message, exit code — is one set of exhaustive `match`es on `Category`, pinned two ways: invariant tests over every category (caller categories are 4xx and system categories 5xx, only `Rail`/`Storage`/`RateLimited` retry after backoff, only `Internal` pages, every `type` is in Stripe's closed vocabulary, no generic message names anything internal, exit codes follow `sysexits`) **and** a literal transcription of [docs/flows/errors.md](flows/errors.md)'s twelve-row table, so the document and the code fail together; `Category::ALL` is proven complete by an exhaustive index function, so a thirteenth variant fails to compile there and fails the test if left out of `ALL`. 28 test functions in `vpay-core`. ✅ for what this row claims — the seam exists and its policy is proven — not for any request being answered through it (see the `ApiError` row) |
| Leaf errors classified (`MoneyError`, `UnknownCurrency`, `LedgerError`, `ConfigError`, `DbError`, `ProviderError`, `AuthRejection`) | ✅ | Each has an `impl Classify` next to its definition, with a comment per non-obvious choice (`DbError::Migrate` is `Configuration` not `Storage`; `ProviderError::Rejected` is `Conflict` with `Retry::NewAttempt`; `LedgerError::Unbalanced` pages). `ProviderError::Rejected` is the one that carries policy of its own: its envelope `code` is the constant `charge_declined` (an earlier draft reused the `FailureCode` string, which collided with `Transport`'s `provider_unavailable` at a different status), the `FailureCode` is in the public message, and its severity follows [docs/flows/failures.md](flows/failures.md)'s own table — `provider_account_blocked` pages, `provider_unavailable`/`provider_error` warn — proven exhaustively over all eleven codes. **Machine-checked**: `cargo xtask verify-errors` fails `just verify` and CI if any `pub` type under `backends/crates` that derives `thiserror::Error` **or** is named `*Error`/`*Rejection` lacks an `impl Classify` outside test code, or if a library crate lists `anyhow` under `[dependencies]` — proven live by deleting `UnknownCurrency`'s impl, by moving an impl into a `#[cfg(test)]` module, and by injecting `anyhow`, all three refused; it currently counts 9 types. Scope, stated plainly: `pub` types only, `backends/crates` only, `tests/` directories and `#[cfg(test)]` blocks excluded, line and block comments stripped; the SDKs and `backends/apps` are outside it by design |
| `vpay_api::ApiError` (HTTP composite) | 🟡 | `#[from]` every leaf the HTTP layer can meet (`DbError`, `ProviderError`, `MoneyError`, `UnknownCurrency`, `LedgerError`, `ConfigError`, `AuthRejection`) plus `UnknownRoute`/`InvalidParam`/`IdempotencyKeyReused`/`Internal`; axum's `Form`/`Json`/`Path`/`Query` rejections convert into `InvalidParam` with a curated sentence (a real `Form` extractor failing over a router yields the 400 envelope with `param: "body"`, never axum's plain text); no blanket `From<serde_json::Error>`, by design. `Classify` delegates all five methods to the leaf, pinned by a test asserting a wrapped leaf answers exactly as the bare leaf for `retry`/`severity`/`public_message` too. `IntoResponse` derives status, `type`, `code`, `message` and optional `param` from the classification and logs the full `Display` **and** source chain at the mapped `tracing` level (`alert = true` on `Page`) — a `DbError` carrying `host-secret-xyz` reaches the log and never the body (tested). `InvalidParam.param` must look like a field name (else `request`) and `message` is capped at 200 chars at render time; a 1 MB input yields a body under 1 KiB (tested). **The two envelope renderers are `pub(crate)`**, so a handler cannot build one by hand — one renderer is structural; `error_envelope` itself is now test-only (its pinned-shape test remains), and `IntoResponse` calls `error_envelope_with_param`. `AuthRejection` is classified and rendered through it; the 404 fallback is an `ApiError`; the pre-existing 404 and 401 envelope bytes are pinned unchanged. 29 test functions in `vpay-api`, 0 ignored. **Changed 2026-09-02: the 401 envelope is now reachable in a running `vpay-server`.** `AuthenticatedMerchant` is mounted in front of the `/v1` nest (see "HTTP surface"), so `ApiError::Auth` is produced by real traffic, not only by this module's own tests: `an_unauthenticated_v1_request_is_401_not_404` and `the_unauthenticated_v1_401_is_the_stripe_shaped_envelope` (`lib.rs`) drive it over the real router, and `a_v1_request_with_no_bearer_token_is_the_401_envelope` (`backends/tests/integration/tests/merchant_token_flow.rs`) does it over a socket against a booted server. Two reachable envelopes now, then: the 401 and the 404. `the_404_fallback_is_byte_for_byte_what_it_was_before_api_error` had to move its URI off `/v1` to keep testing the fallback at all — a `/v1` path with no token is a 401 now and never reaches it — and the pinned bytes are unchanged, because the envelope never echoed the path. **Still 🟡, for what is left rather than for what was**: every other variant (`DbError`, `ProviderError`, `MoneyError`, `UnknownCurrency`, `LedgerError`, `IdempotencyKeyReused`, `InvalidParam` from a body extractor) is still produced by no shipping handler, because no `/v1` business resource exists to produce one. `vpay-api` gained `vpay-config` and `vpay-ledger` as runtime dependencies for variants no handler can produce today. `vpay-config` was already in both binaries' graphs; `vpay-ledger` is a workspace crate that **neither binary linked before** and now both do (via `vpay-api` and `vpay-worker`). No third-party package is new to either binary, but `vpay-api`'s own graph now includes `clap`/`figment`/`garde`/`serde_yaml_ng`; `cargo deny check` still clean. **Changed 2026-09-03 (Step 2), and the "produced by no shipping handler" clause above is now false for most of them.** Four variants are new — `NotFound { resource, id }`, `Conflict { message }`, `Forbidden`, and `IdempotencyKeyInFlight { key_hint }` — and `/v1/payment_intents`'s handlers return them, along with `InvalidParam`, `Db`, `Provider`, `Currency` and `Money`, from a shipping request path. `IdempotencyKeyInFlight` is its own variant rather than a `Conflict` because the two are different advice ("your intent moved on" versus "your own earlier call is still running"), and it classifies as `Category::Idempotency` → `400` `idempotency_error`/`idempotency_key_in_flight`; the `409`-versus-`400` question is in the Idempotency row and is a maintainer decision. `NotFound` is what a *foreign* id answers as well as a missing one, byte for byte (`a_foreign_object_and_a_missing_object_are_byte_identical`), so the API cannot be used to discover which ids exist under another tenant — that is why `Forbidden` is reserved for a missing *scope*. `error.rs` now holds 20 tests, including `the_step_2_variants_say_what_they_should_and_no_more`, `every_variant_answers_with_the_classification_its_leaf_chose`, `every_variant_renders_that_classification_over_a_real_router`, `a_key_still_in_flight_is_a_different_code_from_a_key_reused_and_from_a_conflict`, `an_idempotency_key_is_never_echoed_past_its_hint` and `a_storage_errors_leaf_text_reaches_the_log_and_never_the_body`. **Also new: `vpay_api::form`'s `VpayForm`/`VpayQuery` replaced axum's `Form`/`Query` on `/v1`.** axum's own `FormRejection` renders plain text, which would have put a non-envelope body on the one surface whose error contract is the product; the replacements render the Stripe envelope and name the part of the request the rejection came from (`a_form_rejection_is_answered_with_the_envelope_not_axums_plain_text`, `every_extractor_rejection_names_the_part_of_the_request_it_came_from`, `a_json_body_is_told_to_send_a_form`, `a_missing_required_field_is_a_400_naming_the_body`). **Still 🟡:** `LedgerError` and `ProviderError::Rejected` are still produced by nothing, and `Category::Rail`'s `502` has never been produced by an actual rail, because no rail has ever been called |
| `vpay_worker::JobError` (job-loop composite) | 🟡 | `Db`/`Provider`/`Money`/`Ledger` wrapped with all-five-method delegation, plus `Poisoned` (`Internal`) and `Exhausted` — the reconciler's `unresolved` state, which [docs/flows/reconciler.md](flows/reconciler.md) defines as "still polled, once an hour, and now raising an alert": `Rail`, `Retry::AfterBackoff`, severity `Error`, code `charge_unresolved`. `decision(attempt)` is a wildcard-free `match` on `Classify::retry` alone: `AfterBackoff → RetryAfter { delay, alert }` with `delay = poll_delay(attempt)` (or the documented hour, `UNRESOLVED_POLL_INTERVAL`, for `Exhausted`) and `alert = severity ≥ Error`; `NewAttempt → Terminal`; `Never → DeadLetter`. 12 test functions: a declined charge is `Terminal`, `NotImplemented` dead-letters, `Db::Connect` rides the ladder *and* alerts (Storage is severity `Error`), `Transport` rides it and wakes nobody, `Exhausted` retries hourly with `alert: true` at every attempt and is never a `DeadLetter`. **🟡 because nothing calls `decision()`**: the job loop is ⛔ (Poll ladder row); `JobError` has no consumer anywhere in the workspace and is the contract Phase 5 consumes |
| Binary exit codes (`vpay-server`, `vpay-worker-bin`) | ✅ | `main` returns `ExitCode`: on a startup error the full `anyhow` chain is printed to stderr and the code comes from the first classifiable leaf in that chain (`ConfigError` looked up before `DbError`, since a config naming a dead database is still a config problem), `Internal`/1 if nothing matched. Proven by subprocess tests that need no Docker: missing `--config` → 78, invalid config → 78, a closed Postgres port → 69 (the `sqlx` acquire timeout makes that test take ~5 s, documented on the constant). A mutation forcing `1` fails all six. The drain-timeout `exit(1)` on `vpay-server` is unchanged |
| `Money` — integer minor units, XAF zero-decimal | ✅ | 6 tests incl. cross-currency and over-refund rejection |
| Canonical failure taxonomy | ✅ | 3 tests |
| Charge / intent state + `ProviderFlow` | ✅ | 3 tests incl. live-xor-terminal exhaustiveness. **Extended 2026-09-03 (Step 2):** `vpay_core::state` gained `Transition` (`Confirm(ProviderFlow)`, `Cancel`, …) and `next_status`, the single answer to "is this move legal, and where does it land". The table is proven *total* rather than spot-checked — `the_transition_table_covers_every_status_and_verb` and `next_status_answers_the_lifecycle_diagram_for_every_pair` enumerate every (status, verb) pair against [docs/flows/payment-lifecycle.md](flows/payment-lifecycle.md)'s diagram, with `cancel_is_legal_only_from_requires_payment_method`, `confirm_routes_through_the_flows_own_answer` and `a_new_intent_starts_where_the_diagram_says` naming the individual rules. `/v1`'s handlers ask this module rather than testing a status literal, which is why `confirm_legality_does_not_depend_on_the_rails_flow` can hold. 41 tests in `vpay-core`, of which the new `ids` module contributes 6: `pi_`/`ch_`/`re_`/`evt_` prefixes plus 24 Crockford base-32 characters, `is_well_formed`, and `percent_encoding_an_id_is_the_identity` — so an id can go in a URL path unescaped |
| Ledger balancing invariant | 🟡 | Types and `validate()` done + 3 tests. **Persistence not started** |
| Config guard rails (stub host, literal secret) | 🟡 | The two rules (`validate_host`, `validate_secret`) are unchanged and still directly unit-tested (the original 5 tests). They are now also exercised through real YAML loading: `a_livemode_config_with_an_http_host_is_rejected` and `a_livemode_config_with_a_literal_secret_is_rejected` in `vpay-config`'s `config.rs` drive them through `Config::load_with_env` against fixture files, not just as bare function calls. **Changed 2026-09-03 (Step 2): DB reconciliation — boot-sequence step 4 — is now started.** `vpay_db::config_reconcile::reconcile` makes `currencies` and `providers` match the deployment's configuration in one transaction whose first statement takes `pg_advisory_xact_lock(lock_keys::CONFIG_RECONCILE)`, and both binaries call it at boot. A rail absent from the seed is set `enabled = false`, never deleted, because a rail that has ever taken money must stay nameable. Proven against a real Postgres by `reconcile_is_idempotent_and_disables_a_dropped_provider_code`, `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released` (which is what proves the lock is actually taken, rather than merely written down) and `two_concurrent_reconciles_with_the_seeds_in_opposite_orders_both_succeed_and_converge`. `lock_keys` is a new module holding both advisory-lock constants with `every_advisory_lock_key_is_distinct_and_positive` and `each_key_decodes_to_its_documented_mnemonic`. **Changed 2026-09-03 (Step 3), and one item is a bug this pass found rather than a feature it added.** (1) **Livemode had never been bootable.** `validate_secret`'s rule — "a credential must be *written* as a `${VAR}` placeholder, not a literal" — is a question about the file's text, and it was being asked of the *resolved* values, where a correctly written `${MTN_API_KEY}` and a literal `hunter2` are the same string. It therefore enforced nothing and refused every correct livemode config; the literal fixture passed only because a literal is also not a placeholder. The pre-resolution text of each `providers[].credentials` value is now captured before resolution and checked against that (`RawProviderSecrets`, private to the module; a credential the map cannot account for fails closed). The "an unresolved placeholder is fatal" rule stays where it was, so the two now answer the two different questions they were always meant to (`a_livemode_config_with_a_literal_secret_is_rejected`, `a_livemode_config_whose_placeholders_resolve_loads`, `a_livemode_placeholder_that_does_not_resolve_is_still_the_unresolved_error`, `a_sandbox_config_with_a_literal_secret_loads`). (2) **`REQUIRED_RAIL_KEYS`** refuses to boot a rail missing a key its adapter cannot work without — MTN `settings.{target_environment, api_user}` + `credentials.{subscription_key, api_key}`, Orange `credentials.{merchant_key, client_id, client_secret}` — with a present-but-empty value counting as missing (`a_rail_missing_a_required_setting_is_rejected`, `a_rail_missing_a_required_credential_is_rejected`, `a_required_key_present_but_empty_is_treated_as_missing`, `a_rail_this_crate_has_no_key_table_for_is_not_refused_here`). It is a provider-code match outside an adapter crate, which ADR-0002 forbids; it is a deliberate, recorded interim — [ADR-0012](adr/0012-rail-configuration-requirements-in-config.md) — that selects a *refusal to start*, never behaviour, and moves behind the port the day the port grows a `required_settings()` hook. (3) **`callback_url` and `currency`** are new on `ProviderHost`: `currency` is required and must be in the canonical table (`a_rail_currency_outside_the_canonical_table_is_rejected`); `callback_url` defaults to `{public_base_url}/provider/{code}/callback`, and the *effective* value — derived or overridden — goes through `validate_host`, so a livemode deployment cannot hand a live rail a plaintext or stub callback host (`a_livemode_callback_url_that_is_not_https_is_rejected`, `a_livemode_deployment_cannot_derive_a_plaintext_callback_url`, `a_derived_callback_url_survives_a_trailing_slash_and_an_override_wins`). (4) **`ProviderHost::to_provider_config(&Deployment)`** is the single place a `vpay_provider::ProviderConfig` is built from YAML, so server and worker cannot disagree about a rail's callback URL, currency or deadlines (`to_provider_config_projects_the_example_config_onto_the_port`, which asserts the whole projected value rather than field by field, and `to_provider_config_names_a_currency_it_cannot_parse`). **Still 🟡 for this row's own claim, and step 4 is still not complete: nothing records or compares a config hash**, so nothing detects a replica booted from a different config file — see "YAML config loading" below and [docs/flows/configuration.md](flows/configuration.md) |
| YAML config loading (`vpay-config::Config::load`) | ✅ | Figment layers `application.yml` with an optional `application-{profile}.yml` overlay (same directory, `<stem>-<profile>.<ext>`); `${VAR}` placeholders are resolved by hand-rolled string scanning (figment's own `Env` provider does not interpolate inside YAML scalars) before typed deserialization, so an unresolved placeholder is a named, fatal error, never an empty string; validation runs `garde`'s structural derive, then the existing `validate_host`/`validate_secret` guard rules over every provider, then a currency-exponent-vs-canonical-table check, duplicate-code checks, and (new this pass) the OAuth-client rules below. 23 dedicated tests in `vpay-config/src/config.rs` cover all of that plus (see "Secret redaction" below) that neither `ProviderHost`'s nor the whole `Config`'s `Debug` output ever contains a credential value. **Upgraded from 🟡 to ✅ this pass, for the two reasons the previous note gave for withholding it — both are now closed and both are proven by an end-to-end subprocess test, not just a library-level one:** (1) **now wired into both binaries.** `vpay-server` and `vpay-worker-bin` both call `Config::load` before opening a database connection, and `--config`/`VPAY_CONFIG` is now required at the binary level (still `Option<PathBuf>` at the `clap` type level, exactly like `--database-url`) — proven by three subprocess tests per binary in each `tests/cli.rs`: a missing config is a non-zero exit naming `--config`/`VPAY_CONFIG` (`a_missing_config_is_exit_78_naming_the_problem`), a config that fails validation is a non-zero exit (`a_bad_config_is_exit_78_naming_the_problem`), and a valid config lets the process boot and (for `vpay-server`) actually serve `/healthz` (`a_valid_config_lets_the_server_boot_and_serve_healthz` / `a_valid_config_lets_the_worker_boot`). (2) **merchant and dashboard OAuth clients are now modelled** — see `crate::oauth` (new this pass: `MerchantClient`, `DashboardClient`) and the new "Merchant OAuth clients" notes folded into this row below. **What is still explicitly out of scope, unchanged from before and stated in the module's own doc comment:** two boot-guard rules from [docs/flows/configuration.md](flows/configuration.md)'s table remain unimplemented on purpose, because they need a *payment-routing* `merchants` concept this config shape does not have — "every merchant's rail host is in the allowlist" and "every referenced provider exists and is enabled." An OAuth `MerchantClient`'s `client_id` is not that merchant concept and has no rail host to check. Boot-sequence step 4 (reconciling into the database in one transaction) is also still out of scope here. Neither gap weakens the claim this row actually makes — that `Config::load` loads, validates, and is used — so ✅ stands for that claim specifically. **Updated 2026-09-03 (Step 2): 57 tests (up from 53), and two new rules that refuse to boot.** `MerchantClient::merchant_id` is now **required and unique** across `merchant_clients` — required rather than defaulted to `client_id` because a default would let a config that forgot it boot and silently invent the one boundary `/v1` has no second line of defence for, and unique because two credentials sharing a tenant could read each other's objects (`a_merchant_client_without_a_merchant_id_does_not_load`, `two_merchant_clients_sharing_a_merchant_id_are_rejected`, fixture `oauth-duplicate-merchant-id.yml`; `ConfigError::DuplicateMerchantId`). `ProviderHost::enabled` is new and defaults to enabled when the line is absent (`a_provider_with_no_enabled_line_is_enabled`, `an_explicitly_disabled_provider_stays_disabled`). **One of the two "structurally impossible" gaps this row records is now half-closed:** "every referenced provider exists and is enabled" is enforced at boot in the direction that *is* expressible — a YAML rail with no linked adapter is `ConfigError::ProviderWithoutAdapter` and exit `78`, before the port is bound (`a_provider_code_with_no_linked_adapter_is_exit_78` in `backends/apps/vpay-server/tests/cli.rs`, with `the_repositorys_own_configuration_passes_the_adapter_join` asserting the shipped `config/application.yml` satisfies it). The *merchant*-facing half is still impossible for the same reason as before: an OAuth `MerchantClient` names no rails, and there is no merchant→rail routing concept to check. **Updated 2026-09-03 (Step 3): 70 tests (up from 57), measured** (`cargo nextest run -p vpay-config`: 70 run, 70 passed, 0 skipped). The new rules are in the "Config guard rails" row above; the shipped `config/application.yml` is itself loaded by `a_valid_config_loads_and_produces_the_expected_typed_values` and projected onto the port by `to_provider_config_projects_the_example_config_onto_the_port`, so a `${VAR}` added to that file without a matching entry in `compose.e2e.yml` is an exit-`78` boot failure and not a silent empty string — six rail variables are now referenced (`MTN_SUBSCRIPTION_KEY`, `MTN_API_KEY`, `MTN_API_USER`, `ORANGE_MERCHANT_KEY`, `ORANGE_CLIENT_ID`, `ORANGE_CLIENT_SECRET`) |
| Merchant/dashboard OAuth client modelling (`vpay-config::oauth`, ADR-0010) | ✅ | New this pass, folded into the row above operationally but broken out here because it is a distinct piece of new modelling: `MerchantClient` (public JWK set, `client_credentials` only) and `DashboardClient` (redirect URIs, a single `scope` — enforced by the type being a `String`, not a `Vec<String>`), plus a closed local `GrantType` enum whose serde wire form matches `authkestra_op::client::GrantType`'s. Both carry a `client_secret: Option<String>` trap field that must always be `None`, with hand-written redacting `Debug` impls (5 tests in `oauth.rs`, including one proving a populated `client_secret` never appears in `{:?}` output). Seven boot-time validation rules run from `Config::validate_all`, each with a dedicated fixture-driven test asserting the *specific* `ConfigError` variant: duplicate `client_id` across merchants and the dashboard, an empty/keyless merchant JWKS, a merchant declaring a grant other than `client_credentials`, a dashboard client with no redirect URI, a non-`https` livemode dashboard redirect URI (reusing `validate_host`), and a client secret present anywhere (merchant or dashboard, tested separately). **This is authentication-client modelling only, not merchant *payment routing*** — see the row above's "still out of scope" note for exactly what that distinction means and does not cover |
| Secret redaction (`ProviderHost`/`CommonArgs` hand-written `Debug`) | ✅ | `ProviderHost` (rail credentials) and `CommonArgs` (`--database-url`, which routinely embeds a plaintext password) both hand-write `fmt::Debug` to redact secret values while keeping every other field, and credential *keys*, visible. `Config`, `ServerArgs` and `WorkerArgs` keep `#[derive(Debug)]` — safe because a derive formats each field via *that field's own* `Debug` impl, so the redaction composes upward without a second hand-written impl at every level. Six dedicated tests prove this holds, not just for the leaf types but through the composition: `provider_host_debug_output_never_contains_a_credential_value`, `a_whole_config_debug_output_never_contains_a_credential_value` (`vpay-config/src/config.rs`), `common_args_debug_output_never_contains_the_database_password`, `server_and_worker_args_debug_output_never_contains_the_database_password` (`vpay-config/src/cli.rs`), plus two more asserting the non-secret fields (rail code, host, `[redacted]` marker itself, `database_url: None` when unset) stay visible so the redaction does not silently swallow useful debugging signal. Marked ✅ because a re-derived `Debug` on either type — the exact regression these tests exist to catch — fails the build. **Residual risk stated, not hidden:** `ProviderHost::settings` and `::credentials` are both plain `BTreeMap<String, String>` and only `credentials` is redacted; a value accidentally placed in `settings` instead would leak in plaintext, and no test (and no type) can catch a value merely misclassified between the two maps — the boundary is enforced by convention, not by the type system. **That risk got concrete on 2026-09-03 (Step 3), and the classification was made deliberately:** `ProviderHost` gained `callback_url` and `currency` (both printed, both non-secret), and the rails' new `settings` keys — MTN's `target_environment` and `api_user`, Orange's `env` and `lang` — **print in full** in `ProviderHost`'s `Debug`. That is intended: `api_user` is a UUID identifier, not a bearer secret, and knowing which target environment loaded is exactly the debugging need after a boot failure. Orange's `merchant_key` was considered for `settings` on the same reasoning and **kept in `credentials`** (Step 3, decision 4), so it stays redacted. Every actual secret — `subscription_key`, `api_key`, `client_id`, `client_secret`, `merchant_key` — is in `credentials` and prints as `[redacted]` with its key still visible (`provider_host_debug_output_never_contains_a_credential_value`, `provider_host_debug_output_still_contains_the_non_secret_fields`). The adapters carry the same discipline into their own types: `debugging_the_adapter_does_not_print_the_token`, `debugging_credentials_does_not_print_them`, `debugging_a_token_does_not_print_it` (MTN), `the_adapters_debug_carries_no_credentials`, `debug_never_prints_the_bearer` (Orange) |
| CLI / env configuration (`vpay-config::cli`) | 🟡 | `--version` reports `0.1.0`. Every option auto-resolves from an env var with an explicit flag winning, shared between both binaries via a flattened `CommonArgs`, covered by unit tests on the built `clap::Command` plus subprocess tests that set real env vars on a child process. **`--database-url` is no longer inert** — both binaries now treat it as required at runtime and use it to open a real connection pool and run migrations before serving (see "Database connectivity" below); it stays `Option<String>` at the clap type level, so the CLI itself does not enforce presence, only the two binaries' own startup logic does. **`--config` is no longer inert either, as of this pass** — both binaries now treat it as required at runtime too (same `Option<PathBuf>`-at-the-clap-level, required-in-`main.rs` pattern as `--database-url`), calling `vpay_config::Config::load` and refusing to start on a missing or invalid file; see the "YAML config loading" row above for the three subprocess tests per binary that prove this. **New 2026-09-02 (Step 1): `--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE`**, on `vpay-server` only — the worker issues no tokens, so mounting the Secret into it would widen its blast radius for no capability. Same `Option<PathBuf>`-at-clap, required-in-`main` pattern as `--config`, and required *before* the database connection, so all three failure modes exit `78` with no Docker needed: `a_missing_signing_key_flag_is_exit_78_naming_the_problem`, `a_signing_key_file_that_does_not_exist_is_exit_78_naming_the_path`, `a_signing_key_file_that_is_not_a_key_is_exit_78_without_echoing_its_contents`. The *path* is deliberately not redacted from `Debug` (a path is not a secret, and "which file did it try" is the first thing an operator needs); the file's contents never enter `ServerArgs` at all. Three unit tests in `cli.rs` pin that shape: `the_signing_key_file_flag_parses_to_the_path_it_was_given`, `the_signing_key_path_stays_visible_in_debug_output`, and `the_worker_is_not_handed_the_signing_key`. **`--public-base-url` remains the one flag still accepted and parsed but consumed by nothing, and Step 1 did not change that** — it is easy to assume otherwise now that `/v1/oauth` publishes an issuer, so to be exact: the issuer is `vpay_api::op::issuer_for(&config)`, which reads **`deployment.public_base_url` from the YAML config**, never the CLI flag. Grepping the workspace for `public_base_url` outside `vpay-config` finds only `vpay-api`'s uses of the config field. Two sources of the same idea, one of them inert, is a trap worth closing; nothing in this pass closes it. **A pre-existing gap this pass looked at and left alone:** a missing `--database-url` still exits `1`, not `78`, because `main` produces a bare `anyhow` error there with nothing for `exit_code_for` to classify — the `StartupError` introduced for the signing key covers only the signing key. Out of scope, and stated so nobody reads the new `78`s as meaning every missing flag is classified. **New 2026-09-03 (Step 2): one more `78`, and it is a *configuration* failure rather than a flag one.** Both binaries now derive their reconcile seeds at boot from `vpay_api::v1::boot::boot_seeds` — the same function in both, so they cannot disagree about which rails exist — and a YAML provider code with no linked adapter in *that* binary is `ConfigError::ProviderWithoutAdapter`, which classifies as `Category::Configuration` and exits `78` before Postgres is contacted. `a_provider_code_with_no_linked_adapter_is_exit_78` is a subprocess test against the fixture `backends/apps/vpay-server/tests/fixtures/provider-without-adapter.yml`, so it needs no Docker |
| Database connectivity (`vpay-db`: pool, migrations, healthcheck) | 🟡 | New crate this pass: `connect()` (a `PgPoolOptions` pool, max 10 connections, 5s acquire/connect timeout, eager — it does not return until at least one connection succeeds or the timeout elapses), `run_migrations()` (`sqlx::migrate!` against `backends/migrations`, idempotent by construction), and `check_connection()` (`SELECT 1`). All three are tested against a real `postgres:16-alpine` via testcontainers in `vpay-db/tests/postgres.rs`: `run_migrations_applies_cleanly_and_is_idempotent`, `check_connection_succeeds_against_a_live_database`, and `check_connection_fails_against_a_dead_database` (the container is stopped mid-test to prove the failure path, not just asserted by reading the code). Both `vpay-server` and `vpay-worker-bin` now call `connect()` then `run_migrations()` before doing anything else observable, and this happy path is proven end-to-end, not just at the crate level: `backends/apps/vpay-server/tests/cli.rs` spawns the real binary against a real testcontainers Postgres and polls `GET /healthz` until it returns **200** (`bind_and_log_format_env_vars_are_actually_applied` and others); `vpay-worker-bin`'s equivalent tests prove the same connect-then-migrate sequence via its startup log lines. **Marked 🟡, not ✅, because two specific claims this pass makes are implemented but not proven by any test:** (1) **"a missing `--database-url` is a hard startup failure"** — true by reading `main.rs` in both binaries (`args.common.database_url.as_deref().context(...)?`), but every subprocess test in both `tests/cli.rs` files always supplies `DATABASE_URL`; no test spawns either binary without it and asserts a non-zero exit. (2) **"`/healthz` returns 503 when the database is unreachable"** — true by reading `vpay-api/src/lib.rs`'s `healthz` handler, which maps a `check_connection` error to `StatusCode::SERVICE_UNAVAILABLE`, and `check_connection`'s own failure path is unit-tested in `vpay-db` (above) — but nothing kills the database mid-request and polls the real HTTP endpoint to observe a 503; the handler's status-code mapping itself is unexercised by any test. **Extended 2026-09-03 (Step 2): `vpay-db` is no longer only a pool.** Five repository modules landed — `payment_intents` (insert, merchant-scoped get, keyset `list_page`, `transition` as a compare-and-swap on the expected status, `cancel` with its second `NOT EXISTS` guard), `charges` (`insert_for_intent`, taking a `PgConnection` so the *caller* decides the commit point, which is what lets `confirm` commit before the network), `idempotency` (`claim`/`store`/`release`/reclaim-expired/`sweep_expired`), `provider_requests` (`insert_pending`/`record_response`), and `config_reconcile` — plus `lock_keys` for the advisory-lock constants. `DbError` gained `UniqueViolation` (SQLSTATE `23505` → `Category::Conflict`, code `resource_conflict`, deliberately *not* `invalid_state`: the object is not in a forbidden state, it already exists) and `ForeignKeyViolation` (`23503` → `InvalidRequest`), classified by `integrity_violations_are_the_callers_problem_not_a_storage_outage`. **`vpay-db` now runs 32 tests, 24 of them container-backed in `tests/repositories.rs`, and all 32 passed on 2026-09-03** — including `a_transition_from_a_stale_expected_status_changes_nothing` and `an_intent_in_an_unseeded_currency_is_a_named_foreign_key_violation`. Still 🟡 for this row's own claim, unchanged: nothing observes `/healthz` returning a real 503 |
| Provider port trait (`vpay-provider`) | ✅ | **Changed 2026-09-03 (Step 3): the trait is now `#[async_trait]`** — `submit`, `query_status` and `refund` are `async`; `parse_callback` stays synchronous on purpose, so an adapter cannot make a network call while "parsing" an unauthenticated hint. `async_trait` rather than a native `async fn` because a trait with one is not dyn-safe and this port is only ever held as `Box<dyn ProviderAdapter>` — which is what makes `if provider == "mtn_momo"` structurally impossible outside an adapter crate (ADR-0002). `refund`'s default is `ProviderError::Unsupported`, a permanent capability answer, **not** `NotImplemented`. `ProviderConfig` gained `connect_timeout`/`request_timeout` (5 s / 20 s constants, on the config rather than the client because one `reqwest::Client` is shared by every rail). **The vendored-roots HTTP client moved here from `vpay-api` as `vpay_provider::http`** — `vpay_api::http_client` is a re-export, so no call site changed. It refuses redirects, ignores `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`, and caps any rail body at `MAX_RAIL_BODY_BYTES` (256 KiB) via `bounded_body` rather than reading to end of stream. **Decision 2's cost, stated rather than hidden: `vpay-provider` is no longer a pure interface crate** — it links reqwest, rustls and webpki-roots, so a future non-HTTP rail (a USSD gateway, a file drop) compiles a TLS stack it never uses; no *binary* grew, because both already resolved all three. 11 tests in the crate, measured 2026-09-03 — including `a_redirect_is_returned_rather_than_followed`, `a_request_timeout_actually_fires_against_a_silent_peer` and the `Classify` table for `ProviderError`. See [docs/flows/provider-port.md](flows/provider-port.md) |
| Process lifecycle (SIGINT/SIGTERM) | ✅ | `vpay-server` shuts down via `axum::serve(...).with_graceful_shutdown(...)` on SIGINT or SIGTERM instead of requiring `docker compose down` to SIGKILL it. `vpay-worker-bin` no longer exits immediately on boot — it stays up, answers the same signals, and logs a startup WARN banner plus a 60-second WARN heartbeat stating the job loop is not implemented and no jobs are being processed. **Startup race fixed this pass:** both binaries used to construct their shutdown-signal future late (inside `with_graceful_shutdown`'s argument, or just before the worker's select loop) — `tokio::signal::unix::signal(..)` and `tokio::signal::ctrl_c()` both install their OS-level handler on first *poll*, not at construction, so a SIGTERM delivered before that first poll (CLI parsing, tracing init, adapter-registry logging, `TcpListener::bind` all had to complete first) kept its default disposition and killed the process outright, skipping graceful shutdown and dropping any in-flight request. Confirmed by reproduction (`kill -TERM` sent tens of milliseconds after spawn reliably produced exit 143 with no shutdown log line) and by reading `tokio`'s own source (`signal_hook_registry::register` runs synchronously inside `tokio::signal::unix::signal`'s function body, not inside the future it returns). Fixed by `vpay_config::signal::ShutdownSignals`, a new type in `backends/crates/vpay-config/src/signal.rs` shared by both binaries (precedented by `CommonArgs` living in the same crate): `ShutdownSignals::install()` is now the first thing either binary's `main` does, before tracing init, registering SIGTERM/SIGINT handlers before any slower startup work can run. On Unix, SIGINT is now handled via `signal(SignalKind::interrupt())` rather than `tokio::signal::ctrl_c()` specifically because `ctrl_c()` is an `async fn` and would reintroduce the same late-installation race; non-Unix platforms still fall back to `ctrl_c()` inside `ShutdownSignals::wait()`, unchanged from before. A failure to install a handler is now a **hard startup failure** (`main` returns `Err`), not a logged warning that lets the process run its whole life with no graceful-shutdown path — deliberately stricter than before, since silently continuing would reintroduce the exact bug for the entire process lifetime rather than a brief window. Both binaries are exercised by subprocess tests that send a real `SIGTERM` and assert a clean exit (`backends/apps/vpay-server/tests/cli.rs`, `backends/apps/vpay-worker-bin/tests/cli.rs`), including a new regression test per binary (`sigterm_immediately_after_startup_still_triggers_graceful_shutdown`) that sends SIGTERM almost immediately after spawn and asserts both exit 0 and the graceful-shutdown log line. **That regression test's own limits, stated plainly:** it is a statistical majority-vote test (`ATTEMPTS`/`MIN_SUCCESSES` spawn-signal-wait trials), not a deterministic one, because the actual race window on modern hardware is on the order of a millisecond once other confounds (binary cold-start, CPU frequency ramp-up) are controlled for — verified in isolation to reliably fail against the pre-fix code and pass against the fix (repeated hundreds of times across macOS and a Linux container). But `cargo nextest run --workspace`'s real contention from ~20 concurrently running test binaries widens that window for *both* fixed and unfixed code enough that no single delay was both safe against the fixed binary and sensitive to the bug under full-suite load; the delay actually shipped (`DELAY = 50ms`) was chosen to never fail the full suite on correctly fixed code, at the cost of not reliably catching the bug when run as part of the full suite — its demonstrated sensitivity is strongest when run scoped/alone. This is disclosed in the test's own doc comment, not hidden. |
| `--shutdown-grace-seconds` bounded drain | 🟡 | On `vpay-server` this is now wired in: `serve_with_bounded_drain` in `backends/apps/vpay-server/src/main.rs` races the axum drain against a `shutdown_grace_seconds`-long clock and exits non-zero if the clock wins, logging that in-flight work was cut off. **No test exercises the timeout path itself** — the existing SIGTERM tests never have in-flight work to drain, so they would pass identically with the grace clock deleted; nothing here proves the bound actually holds under load. On `vpay-worker-bin` the flag is accepted and logged ("has no effect yet") but genuinely does nothing — there is no drain to bound because there is no job loop |
| Poll ladder | 🟡 | `poll_delay` done + 3 tests. **Job loop not started.** *Unchanged by Step 3, deliberately: nothing polls a `submitted` charge, so a confirmed intent stays in `processing`/`requires_action` forever. What Step 3 supplied is the adapter half the loop cannot exist without — `query_status` on both rails, returning a canonical `ChargeStatus::NotFound` rather than a failure (`not_found_is_never_on_its_own_a_failure`, both rails).* |
| HTTP surface | 🟡 | **Changed 2026-09-02 (Step 1): the surface is no longer `/healthz` plus a 404.** `router(RouterDeps)` now serves, unauthenticated by necessity rather than omission, `GET /healthz`, `POST /v1/oauth/token`, `GET /v1/oauth/.well-known/openid-configuration` and `GET /v1/oauth/jwks.json` (`the_oauth_routes_are_reachable_without_a_token`); **every other path under `/v1` is nested behind the `AuthenticatedMerchant` extractor, and that nest's only route is the honest 404.** That pair of answers is the whole observable boundary: `GET /v1/payment_intents/pi_x` with no bearer token is a 401 envelope (`an_unauthenticated_v1_request_is_401_not_404`, `the_unauthenticated_v1_401_is_the_stripe_shaped_envelope`), and the same request with a valid merchant token is a 404 `unknown_route` (`an_sdk_client_authenticates_and_reaches_the_honest_404`, integration). **A 200 there would mean someone invented a resource, which is the failure this repo's `CLAUDE.md` names first — so `/v1` still implements no business resource at all, and this row stays 🟡 for exactly that reason.** The auth layer is `Router::layer`, not `route_layer`: `route_layer` does not wrap a fallback, and since this nest's only route *is* its fallback, axum refuses the build — verified by making the swap and watching every router test panic. That protection disappears the moment a real `/v1` route lands, which is why the choice is written down in `router`'s own doc comment. **Two known edges, both documented in code and neither fixed:** `GET /v1/oauth/token` (right path, wrong method) gets axum's bare `405` with an empty body instead of the Stripe envelope — the status is correct and turning it into the 404 envelope would be worse, so it waits for a `method_not_allowed` renderer for the whole surface; and `GET /v1/` (the bare trailing-slash form) falls through to the *outer* 404 rather than the nest's 401 (`the_bare_trailing_slash_form_of_v1_falls_through_to_the_outer_404` pins this as the current behaviour, not as desirable). What the previous pass changed, unchanged since: `/healthz` is no longer a static `"ok"` string. It runs `vpay_db::check_connection` (a real `SELECT 1`) and returns `200`/`"ok"` or `503`/`"database unreachable"` depending on the result — see "Database connectivity" above for exactly what is and is not tested about that mapping. The router's constructor argument changed with Step 1 from a bare `PgPool` to `RouterDeps { pool, merchant_op, merchant_validator }` — so a router cannot exist without a database connection *or* without the OP and the validator that guard `/v1`; there is no way to build a partially-wired one. **New 2026-09-02: request ids and a per-request span.** `router()` mounts, in this order, a guard that drops a caller-supplied `x-request-id` unless it is 1–64 bytes of ASCII `[A-Za-z0-9._-]` (removed, never rejected: a bad diagnostic header must not block a payment request — the caller merely loses the right to choose the id; one unusable value drops the whole header), tower-http's `SetRequestIdLayer` (mints a v4 UUID `x-request-id` unless the caller sent one the guard kept), a `TraceLayer` whose span records `method`, `path` and `request_id`, and `PropagateRequestIdLayer` (copies the id onto the response). Seven of `lib.rs`'s fourteen tests (the other seven cover the route tree above): a request with no id gets a UUID back, a caller-supplied id comes back unchanged, the `api error` line `ApiError` logs while serving a 404 carries the request id, a 4 KB id / an id with a space, `/`, `"` or a non-ASCII byte / a 65-byte id are each replaced by a minted UUID while a 64-byte one survives, and one unusable value among several drops the header — each proven decisive by disabling the guard, the span field or the propagate layer and watching the relevant tests fail. This is what makes `Category::Internal`'s "Contact support with the request id" a sentence a merchant can act on; `error.rs`'s "No `request_id` field here" section now points at the mounted layers instead of saying they are not mounted. `/healthz`'s plain-text body and the 404 envelope bytes are unchanged (pinned). **Security review the same day:** the `/v1/oauth` nest has its own explicit 404 fallback — measured, an unmatched path under it used to fall into the *authenticated* `/v1/{*rest}` route and answer 401, telling an integrator who mistyped an OP path to present a bearer token on the one subtree that issues them; `the_oauth_nest_answers_its_own_404` pins the 404. **Changed again 2026-09-03 (Step 2): the nest is no longer one 404.** `vpay_api::v1::V1_ROUTES` is now the router's *source* — a `&[V1Route]` of path, methods and mount function that `routes()` folds into the `Router`, so a route cannot exist without appearing in the table a test can walk. Four paths, five methods: `POST`/`GET /payment_intents`, `GET /payment_intents/{id}`, `POST /payment_intents/{id}/confirm`, `POST /payment_intents/{id}/cancel`. `every_registered_v1_path_answers_401_without_a_token` (integration) walks `V1_ROUTES` itself rather than a hand-kept copy, so a new route is covered the day it lands. `/v1/balance`, `/v1/events` and `/v1/refunds` are deliberately **not** mounted and still answer the nest's 404 — both SDKs can call all three and vpay implements none of them. The auth layer is now a middleware (D3) that validates the bearer token **once**, resolves the token's `client_id` to a `MerchantScope` through the YAML, puts that scope in request extensions, and checks the method's required scope before axum matches a route: `payments:write` for anything that is not a read, `payments:read` or `payments:write` for `GET`/`HEAD`, `403` otherwise (`only_a_write_scope_authorises_a_method_that_is_not_a_read`, `a_clients_tenant_comes_from_config_not_from_the_client_id`, `a_token_without_the_required_scope_is_403_not_401`, and `a_client_registered_for_no_scopes_is_forbidden_while_a_scoped_one_is_not` over real HTTP). `MerchantScope`'s field is `pub(crate)` and the extractor is the only public way to obtain one, so a handler cannot invent a tenant; there is no `merchants` table and therefore no foreign key that would catch a query missing its filter, which is the whole reason it is built this way. A `RequestBodyLimitLayer` of **64 KiB** is mounted on the nest (`a_body_over_the_limit_is_refused_by_the_layer`). The `route_layer`-versus-`layer` note above no longer applies the way it did: real routes now exist beside the fallback |
| Payment intents (`/v1/payment_intents`) — `vpay_api::v1::payment_intents` | 🟡 | **New 2026-09-03 (Step 2). The first `/v1` business resource, and it stops at the rail on purpose.** Evidence for every claim below is `backends/tests/integration/tests/payment_intents.rs` (16 tests) unless another file is named, and all 16 passed against a real `postgres:16-alpine` on the authoring machine on 2026-09-03 (header). **✅ create**: `POST /v1/payment_intents` decodes a Stripe-bracket form body, validates `amount` (1 … 2^53−1, the same bound both SDKs enforce client-side), `currency` against the deployment's configured set, each `payment_method_types[]` against the *enabled* rails, `metadata` (50 keys / 40-char keys / 500-char values) and `description` (1000), writes a row in `requires_payment_method`, and renders the wire object (`create_then_retrieve_round_trips_through_the_sdk`, plus the unit rules `a_currency_the_deployment_did_not_configure_is_refused_by_name`, `a_disabled_rail_cannot_be_named_on_a_new_intent`, `the_amount_bounds_are_the_ones_both_sdks_enforce`, `metadata_and_description_bounds_name_their_own_parameter`). **✅ retrieve**, including the tenancy property that matters: another merchant's id answers the *identical* 404 (`merchant_b_cannot_read_merchant_as_intent`). **✅ list**: keyset pagination over `seq`, newest first, `limit` defaulting to 10 and capped at 100; both cursors together is a `400`, a malformed cursor is a `400`, and a well-formed cursor belonging to another merchant is an **empty page** rather than an error, because saying "no such id" would leak (`list_pages_forward_and_backward_with_cursors`, `a_list_refuses_two_cursors_and_a_malformed_one`, `a_cursor_is_checked_for_shape_and_not_for_existence`, and `list_page_walks_forward_and_backward_over_twenty_five_intents` in `vpay-db`). **✅ cancel**: a compare-and-swap carrying *two* guards — the expected status and `NOT EXISTS` a live charge — so an intent whose confirm already handed a charge to a rail cannot be cancelled out from under the payer; the ambiguous zero-row answer is disambiguated by one re-read into a 404 or one of two distinct 409s (`cancel_is_legal_only_from_requires_payment_method`, `a_confirmed_intent_cannot_be_canceled`, `cancel_refuses_an_intent_with_a_live_charge_and_allows_one_with_a_terminal_charge` in `vpay-db`). **🟡 confirm — see the "Charge submission" row directly below, which is where this pass's work landed.** A second confirm is a `409` before any insert, and the `one_charge_per_intent` unique index catches the race the read cannot (`a_second_confirm_cannot_produce_a_second_charge`, `a_second_charge_for_one_intent_is_refused_as_a_named_unique_violation` in `vpay-db`). The rail is chosen by `payment_method_data[type]` and branched on by `Capabilities::flow` only, never by code (ADR-0002) — `confirm_legality_does_not_depend_on_the_rails_flow`, `an_intent_only_allows_the_rails_it_was_created_with`. **Still 🟡, and for a reason that moved rather than closed:** the intent now reaches `processing` and `requires_action`, but **never `succeeded`** — nothing polls a `submitted` charge — and `next_action` is only ever a `redirect_to_url`. The integration suite is now **17 tests** (`payment_intents.rs`), all passing against a real Postgres on 2026-09-03 |
| Charge submission (`confirm` → rail) — `vpay_api::v1::payment_intents::confirm` | 🟡 | **New 2026-09-03 (Step 3). `confirm` reaches a rail over HTTP and moves the intent.** Evidence is `backends/tests/integration/tests/confirm_rails.rs` (7 tests, Postgres **and** WireMock containers), measured passing on 2026-09-03. The write-first ordering [docs/flows/crash-safety.md](flows/crash-safety.md) requires is unchanged — mint the reference, **commit** the charge in `submitting` (with the merchant's `return_url` on a redirect rail), insert a `provider_requests` row with no status, `await` `adapter.submit(..)`, record what came back — and step 6 now has four shapes, chosen by the *error's* own classification rather than by anything the handler knows about rails: **(1) push accepted** → charge `submitted`, intent **`processing`**, `200`, `next_action: null` (`a_push_confirm_the_rail_accepts_moves_the_intent_to_processing`); **(2) redirect accepted** → charge `submitted` with the rail's `pay_token` and `redirect_url`, intent **`requires_action`**, `200` with `next_action.redirect_to_url`. **The commit is the gate on the redirect, by construction:** the rail's material, the `return_url` and both statuses move in **one** transaction, and `next_action` is then built **only from the committed charge row**, never from the adapter's return value — so no code path can emit a URL the database does not already hold (`redirect_confirm_commits_the_rails_material_before_it_answers` asserts the URL handed to the merchant equals the one on the row). **(3) declined at submit** (`ProviderError::Rejected`) → charge `failed` with its `failure_code` + `failure_raw`, intent **keeps `requires_payment_method`** now carrying `last_payment_error` (the public message; the rail's raw words are stored and logged, never sent), merchant gets `409 charge_declined`. Two cases, on purpose: `a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read` steers the rail's refusal with the one field of the outgoing request a merchant controls (the MSISDN) and lands `invalid_payer`, and `credentials_the_rail_refuses_are_a_page_and_a_terminal_charge` lands `provider_account_blocked` — a different code, a different severity and a different on-call answer, asserted separately so the two cannot silently become one test. **(4) transport / malformed / misconfigured** → **nothing moves**: the charge stays `submitting`, the attempt keeps `status_code IS NULL`, and the merchant gets a `502`. Retrying under the same idempotency key then meets that live charge and is refused with a `409` whose text is **"A charge for this PaymentIntent is being resolved with the rail; poll GET /v1/payment_intents/{id} — do not create a new PaymentIntent."** — the advice follows `ChargeState::is_live`, and only a *terminal* charge gets the old "create a new payment intent" wording. That distinction came out of the Step 3 security review: telling a merchant whose submit timed out to open a second intent is telling them to prompt the payer's handset twice (`an_unreachable_rail_leaves_the_charge_where_recovery_expects_it`). **Two refusals happen before any charge exists**, so the intent stays confirmable on another rail: an intent whose currency is not the rail's settlement currency is a `400` naming `payment_method_data[type]` (`a_rail_that_settles_in_another_currency_is_refused_before_any_charge`), and a `return_url` that is not `http(s)` or is over 2048 characters is a `400` naming `return_url` (`a_return_url_that_is_not_a_bounded_web_url_is_refused_before_any_charge`) — the same bound the `charges.return_url` CHECK carries (migration `0019`), so a merchant gets a named `400` rather than the `503` an unguarded constraint violation becomes. **`provider_requests.status_code = 0` is a sentinel, documented by migration `0020`:** it means the rail *answered* but the port carries no HTTP status (`Submitted`, `Rejected`). `NULL` keeps its meaning — no answer received — paired with `NULL responded_at` by the `response_is_paired` CHECK, and that pair is what the recovery table reads. `error_kind` is `Classify::code(&ProviderError)`, so it is the same vocabulary the merchant's envelope uses (`the_recorded_error_kind_is_the_errors_own_code`). **Not done, plainly:** every rail response above came from a **WireMock** container, never from MTN or Orange; **nothing reads the rows a lost submit leaves** — there is no recovery pass and no worker, so a push confirm ends at `processing` forever; there is no callback route; and no intent has ever reached `succeeded` |
| Merchant OP (`/v1/oauth`) — `vpay_api::op` | 🟡 | New 2026-09-02. `MerchantOp` (`op/mod.rs`) assembles an `authkestra_op` provider whose issuer is `{deployment.public_base_url}/v1/oauth` (`issuer_for`, the single derivation in the workspace — `main` and `MerchantOp::new` both call it, so the `iss` a token is stamped with and the `iss` the validator pins cannot drift; `the_issuer_and_endpoints_are_what_the_sdk_derives_from_a_base_url` and `a_trailing_slash_on_the_public_base_url_does_not_change_the_issuer`). `grant_types_supported` is `["client_credentials"]` and nothing else. The store is a `CompositeOpStore` over `YamlClientStore` (row below), `SqlClientAssertionStore` for replay, and **three `SqlxOpStore<Postgres>` slots that serve no `/v1` grant** — they exist because `OpStore` is a supertrait of the code/refresh/device stores, and they are the real Postgres stores rather than an "always empty" stub because AGENTS.md rule 1 forbids a test double reachable from a shipping binary and a hand-written empty store would become a silent lie the day another grant is mounted. `op/token.rs` is thin: `token_handler` delegates to `authkestra_op::handlers::token::handle_token` and maps its RFC 6749 error JSON to a status (`invalid_client` → 401, everything else → 400); the discovery document is hand-built so it advertises only what this deployment serves (`discovery_publishes_the_endpoints_the_sdk_would_have_guessed`, `discovery_advertises_no_endpoint_this_deployment_does_not_serve`, `discovery_advertises_only_private_key_jwt`). **`authkestra-axum` is deliberately not a dependency** — its bundled router mounts `/authorize`, `/userinfo` and a one-key JWKS handler this deployment must not serve; see the JWKS row below. `op/jwks.rs` serves `/v1/oauth/jwks.json` from `vpay_db::publishable_signing_keys` (the whole rotation window, not just the active key), `Cache-Control: public, max-age=300` (`the_response_is_publicly_cacheable_for_the_documented_window`), skipping and warning about a row whose `public_jwk` is unusable rather than failing the whole document (`a_jwk_without_a_kid_is_skipped_and_warned_about`, `a_public_jwk_that_is_not_an_object_is_skipped_and_warned_about`, `a_kid_that_disagrees_with_its_row_is_published_but_warned_about`). 33 unit tests across `op::{mod,clients,keys,jwks,token}`. **Two numbers in here are defaults this pass chose, not decisions anyone recorded:** `ACCESS_TOKEN_TTL_SECS = 900` and `keys::ROTATION_OVERLAP = 24 h`. `docs/roadmap.md` lists both the access-token TTL and the rotation-overlap window as open questions and this pass does not close either; the only property that is *tested* is the relationship between them (`the_access_token_ttl_fits_inside_the_key_rotation_overlap`, `the_rotation_overlap_dwarfs_the_access_token_ttl_it_has_to_cover`), not that either value is right. **🟡 and not ✅** because the end-to-end proof — `the_jwks_and_discovery_documents_describe_this_process` and the rest of `merchant_token_flow` — has run exactly once, manually, against a scratch database, and never under Docker or in CI; see the header paragraph. **Not done here:** no rate limit on `/token` (left to ingress per [ADR-0009](adr/0009-dashboard-oidc-provider.md), and nothing in this repo checks that ingress actually does it), and no `/v1` resource for a minted token to reach.. **Known limitation, recorded not fixed:** no rate limit exists anywhere in this repository in front of `/v1/oauth/token` (one `disabled_clients` `SELECT` per request for any public `client_id`, before any signature check) — ADR-0009 leaves it to the ingress, which nothing here verifies. **Two changes 2026-09-03 (Step 2).** (1) **A token request that names no `scope` is now granted the client's registered `scopes:`** — RFC 6749 §3.3's "locally defined default", defined here as the registration itself. Both SDKs omit `scope`, so before this, every SDK-minted token carried none and would have been `403`ed by every `/v1` call; `handle_client_credentials` treats an absent `scope` as "grant none" and offers no seam, so the default is applied in `token_handler` before the grant runs. It only ever *fills in* an omitted value: a request naming a narrower scope keeps it, and anything outside the registration is still `invalid_scope` (`the_default_scope_is_the_clients_own_registration_and_nothing_wider`). An empty `scopes:` list is legal and means the client can mint a token and be `403`ed by everything, which is what `a_client_registered_for_no_scopes_is_forbidden_while_a_scoped_one_is_not` exercises over real HTTP. (2) **A minted token now has somewhere to go** — the "no `/v1` resource for a minted token to reach" clause above is no longer true; see the "Payment intents" row |
| Database schema / migrations (core) | ✅ | Five migrations exist in `backends/migrations/` (`0001_create-currencies.sql` … `0005_create-ledger.sql`), applied via `sqlx::migrate!` to a real `postgres:16-alpine` (testcontainers) and asserted against in `backends/tests/integration/tests/postgres_smoke.rs`: a clean migration run on an empty database, the `one_charge_per_intent` unique index, two cross-column `CHECK` constraints firing (`partial_refunds_imply_refunds` on `providers`, `no_over_refund` on `payment_intents`), a plain `amount >= 0` check, an FK violation, and an out-of-range currency exponent. Marked ✅ and not 🟡 because the claim this row makes — "the schema and migrations exist, apply cleanly, and their constraints actually fire" — is fully implemented and tested; a broken migration or a dead constraint would fail a real test. **This is narrower than "the database works."** No route reads or writes an application row through this schema yet — that gap is now tracked by "HTTP surface" and "Database connectivity" above (a connection pool and a migration runner now exist and are wired into both binaries, closing the exact gap this row used to describe — "there is no connection pool" is no longer true, see those rows for what is and is not proven), the same way "Provider port trait" being ✅ above does not imply the adapters' wire calls work. **This repository now has eighteen migrations in total** (`0001`–`0018`; it said twelve until 2026-09-02 and thirteen until 2026-09-03); this row covers only the first five — see the rows below for `0006`–`0013`, and this paragraph for `0014`–`0018`. **Five migrations landed on 2026-09-03 (Step 2), and all five are proven applied against a real Postgres by `migration_0014_replaces_last_payment_error_and_0015_to_0018_create_their_tables` in `backends/crates/vpay-db/tests/repositories.rs`** — which asserts the dropped column is gone, the new ones are present, and each new table exists. `0014_payment-intent-api-fields` adds to `payment_intents` a `seq BIGINT GENERATED ALWAYS AS IDENTITY` (the only pagination order; `created_at` cannot be it, because two intents in the same microsecond tie and a cursor over a non-unique ordering skips or repeats rows), `metadata JSONB`, `description`, `updated_at`, and the pair `last_payment_error_code failure_code` + `last_payment_error_message`, with `lpe_paired`, `lpe_message_length`, `description_length`, `metadata_is_object` and `pmt_is_array` CHECKs, a unique index on `seq`, a `(merchant_id, seq DESC)` index, `charges.updated_at`, and a partial index over the four live charge states. **It is a hard cutover, and the migration says so in its own header:** the free-text `last_payment_error` column added by `0003` is **DROPPED, not backfilled**. That is defensible only because nothing in this repository had ever written it — no SQLx query referenced `payment_intents` at all before this step — so a backfill would have been inventing values; a deployment that somehow held rows with a non-NULL `last_payment_error` loses them. `updated_at` is maintained by the repository layer, deliberately not by a trigger. `0015_create-idempotency-keys` creates the ledger the Idempotency row describes, including a `claim_id UUID DEFAULT gen_random_uuid()` that identifies which claim owns a row: the primary key alone is not enough once a row is reclaimable after expiry, and without it a stalled request waking up after its key was taken over would overwrite a live claim's stored response with its own. `0016_create-provider-requests` creates one row per *attempt* to call a rail (never one per charge), with `response_is_paired` keeping `status_code` and `responded_at` in step, so a NULL-status row is an unanswered attempt — exactly what a recovery sweep will look for (`provider_requests_record_attempts_and_keep_status_and_responded_at_in_step`). **`0017_create-refunds` and `0018_create-events` are schema only, and their own `COMMENT ON TABLE` says so:** no code in this repository reads or writes either table, there is no refunds repository, no `/v1/refunds` or `/v1/events` route, nothing emits an event and no fan-out loop exists — they are declared the way the ledger tables in `0005` were, and they carry the same warning |
| Authkestra OP tables (`0006_create-authkestra-op-tables.sql`, extended by `0013_add-authkestra-op-0-7-columns.sql`) | ✅ | `CREATE SCHEMA authkestra` plus `oauth_clients`, `oauth_codes`, `oauth_refresh_tokens`, `oauth_device_codes` — a byte-faithful transcription of the `CREATE TABLE` string literal hardcoded inside `authkestra-op` `=0.3.4`'s own `SqlxOpStore::migrate()` (not a vpay design; table/column names and types are not configurable — see the migration's header comment). **Upgraded to `authkestra-op = "=0.7.1"` this pass (from `=0.5.4`), and the re-diff the previous note demanded was done, not assumed:** `diff` over the extracted 0.3.4 and 0.7.1 crate sources shows the four tables 0006 creates are byte-identical, and 0.7.1's `migrate()` adds exactly one table (`authkestra.oauth_dpop_jti`, RFC 9449 DPoP replay tracking, authkestra#291) and three columns (`oauth_refresh_tokens.jkt`, `oauth_clients.token_endpoint_auth_method JSONB`, `oauth_clients.jwks JSONB`, authkestra#287). Migration `0013` transcribes those additions; it is **not optional** at this pin — `get_token`/`consume_token` now `SELECT … jkt` unconditionally and would fail at runtime against 0006's table alone. Proven compatible, not just transcribed correctly by eye: `backends/tests/integration/tests/authkestra_op_smoke.rs`'s `sqlx_op_store_round_trips_a_client_and_enforces_single_use_codes` drives the real `SqlxOpStore<Postgres>` against this schema end to end — inserts a client, `find_client` (JSONB columns decode through the store's own type, **now including `token_endpoint_auth_method` decoding to `TokenEndpointAuthMethod::PrivateKeyJwt` and `jwks` round-tripping as raw JSON**), `store_code`, `consume_code`, and asserts a second `consume_code` of the same code returns `None`, proving the crate's single-use `UPDATE … WHERE used = FALSE` actually fires here. Two new tests in the same file cover 0013's other additions through the store's own SQL: `sqlx_op_store_round_trips_a_refresh_token_with_its_jkt_column` (`store_token`/`get_token` round-trip `jkt`) and `sqlx_op_store_records_a_dpop_jti_once_against_migration_0013s_table` (`check_and_record_dpop_jti` accepts a fresh `jti` and refuses its unexpired replay). Neither refresh tokens nor DPoP are features vpay offers — see `docs/flows/dashboard-auth.md` — these prove schema compatibility with the pinned crate, nothing more. **Two API breaks absorbed in the same test file:** `AuthorizationCode` is `#[non_exhaustive]` since 0.6.0 (constructed via `AuthorizationCode::new` now), and `ClientRegistration::require_pkce` is deprecated since 0.7.0 because PKCE is unconditional on the authorization-code grant (authkestra#273) — the test no longer asserts on a field nothing reads. A second test in `postgres_smoke.rs` proves the `oauth_codes → oauth_clients` FK fires. `oauth_device_codes` is created even though vpay's login flow (PKCE only) never uses the device grant, because `SqlxOpStore` implements `DeviceCodeStore` unconditionally. **Marked ✅ for what this row claims — the DDL exists, matches the pinned crate, and is proven compatible against a real store — not for dashboard auth working.** No shipping binary constructs a `SqlxOpStore` or uses these tables — see "Dashboard auth" below. **Correcting a claim this row used to make, which this pass's dependency-graph check found stale:** it used to say `authkestra-op`/`authkestra-engine` were dev-dependencies of `vpay-tests-integration` only, with neither `vpay-server` nor `vpay-worker-bin` depending on `authkestra*` at all. That second half is no longer true — `vpay-db` added `authkestra-op` as a **production** dependency this pass (for `SqlClientAssertionStore`, OP-2), and both binaries depend on `vpay-db`, so `authkestra-op` (and, transitively, `authkestra-engine`) is now in both binaries' production dependency graph. `vpay-server`/`vpay-worker-bin` still do not name `authkestra*` directly in their own `Cargo.toml`s, but "depend on neither" is no longer an accurate description of the resolved graph — see the "cargo deny" infrastructure row for the concrete consequence (the `rsa` advisory's exposure is narrower than "dev-only" now claims). **Coupling risk:** this migration pair is pinned to `authkestra-op = "=0.7.1"` (root `Cargo.toml`) and must move in lockstep with it — the crate hand-builds SQL against these exact table/column names as string literals, so nothing type-checks a mismatch. Any future version bump of `authkestra-op` requires re-reading `sqlx_store.rs`'s `migrate()` block at the new version and re-diffing against this file before assuming compatibility still holds; the migration's own header comment says the same and this is not to be treated as a routine dependency bump |
| OAuth signing keys (`0007_create-oauth-signing-keys.sql`, reshaped by `0010_reshape-oauth-signing-keys.sql`) | 🟡 | vpay-owned table (authkestra ships no signing-key type, store, or rotation logic at any published version — confirmed by grepping `authkestra-op-0.3.4` and `authkestra-engine-0.3.4` source for `struct SigningKey`, `trait KeyStore` and `fn rotate`, with no hits). **Reshaped this pass: `private_key_pem TEXT` is dropped entirely and replaced with `public_jwk JSONB`; `id` is renamed to `kid`.** The decision (migration `0010`'s own header comment) is that the RS256 private key comes from a Kubernetes Secret via env at process boot and is parsed once by `authkestra_engine::TokenManager::new_asymmetric`, never persisted — so this table now stores only what `/jwks.json` needs to publish across a rotation window: the public half, its `kid`, and the validity window. **This corrects last pass's own note, which said the private key PEM was stored in plaintext and readable by anyone who could `SELECT` the column — that is no longer true; no private key material exists in this table or this repository at all.** The three constraints (partial unique index `one_active_signing_key`, `active_key_has_no_expiry`, `expiry_after_creation`, the last two renamed alongside the column) are proven to still fire *after* the reshape by the same dedicated tests in `postgres_smoke.rs`, updated to insert `kid`/`public_jwk` rather than `id`/`private_key_pem`. **New this pass: a Rust repository layer exists** (`vpay_db::signing_keys` — `publishable_signing_keys`, `active_signing_key_kid`, `rotate_signing_key`), tested against a real Postgres in `vpay-db/tests/repositories.rs` — `publishable_signing_keys_includes_active_and_unexpired_retired_but_excludes_expired` proves the `WHERE active OR expires_at > now()` overlap-window query keeps a just-retired key publishable and drops a long-expired one, and `rotate_signing_key_leaves_exactly_one_active_key` proves the one-transaction retire-then-insert both bootstraps cleanly (no prior active key) and rotates cleanly (an active key already exists), leaving `one_active_signing_key` intact either way. **Both of this row's previous reasons for 🟡 are now closed, 2026-09-02 (Step 1).** (1) **Key generation exists**: `cargo xtask gen-signing-key --out <dir>` writes a 3072-bit RSA PKCS#8 PEM, `0600`, refusing to overwrite — `a_generated_key_parses_back_off_disk_with_the_same_kid`, `the_key_file_is_only_readable_by_its_owner`, `it_refuses_to_overwrite_an_existing_key_file` (`.xtask`). `just gen-e2e-signing-key` is the openssl equivalent for the compose stack, so the CI e2e job needs no Rust toolchain. (2) **A shipping binary now calls this module.** `vpay_api::op::keys::LoadedSigningKey::from_file` parses the PEM into `authkestra_engine::TokenManager`, derives the `kid` as the RFC 7638 thumbprint of the public JWK — a function of the key, not of the file or the process (`the_kid_is_a_function_of_the_key_and_not_of_the_encoding_or_the_process`) — and cross-checks the JWK it publishes against `TokenManager::public_jwk`, so the key announced and the key signed with cannot diverge (`the_published_jwk_is_the_key_authkestra_signs_with`, `the_published_jwk_has_the_six_members_a_verifier_needs_and_a_self_consistent_kid`). Anything that is not an RSA private key, and anything under 2048 bits, is refused (`anything_that_is_not_an_rsa_private_key_is_refused`), and no error message or source chain echoes the PEM (`no_error_message_or_source_chain_echoes_the_pem`). `vpay-server` loads it **before** connecting to Postgres, so the three failure modes are testable without Docker and all exit `78`: `a_missing_signing_key_flag_is_exit_78_naming_the_problem`, `a_signing_key_file_that_does_not_exist_is_exit_78_naming_the_path`, `a_signing_key_file_that_is_not_a_key_is_exit_78_without_echoing_its_contents` (`backends/apps/vpay-server/tests/cli.rs`, subprocess). Activation goes through the new `vpay_db::ensure_active_signing_key`, which takes a Postgres advisory lock and does the whole read-decide-write in one transaction, so N replicas booting on the same Secret rotate once between them (`ensure_active_signing_key_bootstraps_is_idempotent_then_rotates_once`, `concurrent_ensure_active_signing_key_calls_with_the_same_kid_rotate_exactly_once`, `ensure_active_signing_key_refuses_to_reactivate_a_retired_kid` — a rollback to a retired `kid` is refused rather than silently resurrecting a key). **Still 🟡, for three new and smaller reasons, none of them "nothing calls it":** (a) **there is no rotation at runtime** — `TokenManager` holds exactly one key for the life of the process, so rotating means restarting with a new Secret; nothing re-reads the file, and no operator runbook describes the sequence; (b) the five `ensure_active_signing_key` tests are Docker-backed and **have not been run on any machine yet** (see the header paragraph) — the code is written and the tests exist, nothing has observed them pass; (c) the PEM is **not zeroized** — `LoadedSigningKey::from_file` reads it into a `String` that is dropped normally, so key bytes may linger in freed heap. That is a deliberate, stated limitation, not an oversight (`op/keys.rs`'s own module docs say so), and it is not fixed here. **Rollback to a retired key (security review 2026-09-02):** `ensure_active_signing_key` now refuses it with `DbError::SigningKeyRetired { kid, retired_at }` (`Category::Configuration`, so `vpay-server` exits 78 naming the kid and the retirement instant) instead of a raw duplicate-key SQL error — proven against a real Postgres by `ensure_active_signing_key_refuses_to_reactivate_a_retired_kid` and by `a_rollback_to_a_retired_signing_key_exits_78_and_a_dead_database_still_exits_69` in `vpay-server`'s `tests/cli.rs`. Re-activating a still-publishable retired key is deliberately *not* done — that is the rotation-policy decision [docs/roadmap.md](roadmap.md) leaves open; the operational consequence, that `kubectl rollout undo` after a rotation is a clean exit 78 rather than a degraded boot, is stated here on purpose. The `bootstraps_is_idempotent_then_rotates_once` test had never executed before this pass and failed on its first real run — it compared a nanosecond `OffsetDateTime` with the microsecond `TIMESTAMPTZ` read back; fixed by building the expected instant at microsecond precision, and all 12 `vpay-db` tests now pass on a real container on the authoring machine |
| Merchant API keys — dropped (`0008_create-merchant-api-keys.sql`, dropped by `0009_drop-merchant-api-keys.sql`) | ⛔ | The Stripe-shaped `sk_live_`/`sk_test_` bearer-key design this table backed is reversed by [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md): `authkestra_op::sqlx_store::SqlxOpStore::find_client` hardcoded `token_endpoint_auth_method: None`/`jwks: None` on every row at the then-pinned `authkestra-op = "=0.3.4"`, so an OP-backed client registry could not serve `private_key_jwt`. **That premise is no longer true at the current pin (`=0.7.1`): both columns are persisted and read back (authkestra#287), proven here by migration `0013` and the `find_client` assertions in `authkestra_op_smoke.rs`.** ADR-0010's *decision* — merchant clients in YAML, no database-stored merchant identity — is unchanged; an ADR is superseded, never edited, and whether the now-available OP-backed registry should replace YAML is a maintainer question this pass raises and does not answer. Per this repo's hard-cutover rule, `0009` is a straight `DROP TABLE`, not a deprecation — nothing had ever read or written a row here (last pass's own note said so), and the two tests that proved this table's constraints were deleted in the same migration rather than left passing against a table that no longer exists. **A reader must not infer from ADR-0010's continued reference to this migration number, or from this row remaining in the table for historical clarity, that `merchant_api_keys` still exists — it does not.** See "Merchant auth" below for the model that replaces it |
| Merchant auth (`/v1`: `client_credentials` + `private_key_jwt`, [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md)) | 🟡 | **The server half of this flow now exists.** A merchant is a statically registered OAuth2 client with a `client_id` and **public** JWK in YAML, authenticating with a signed `private_key_jwt` assertion; `vpay_api::op::clients::registration_for` is the conversion into `authkestra_op::client::ClientRegistration` this row spent two passes calling "the missing piece", and it is mechanical by design — `token_endpoint_auth_method: Some(PrivateKeyJwt)`, `client_secret_hash: None`, `redirect_uris: []`, and `grant_types` mapped from the config enum rather than hardcoded so `ConfigError::DisallowedMerchantGrant` stays observable (`the_conversion_maps_every_field_the_op_reads`, `the_conversion_maps_grants_it_is_given_rather_than_hardcoding_one`). **The registration is proven to be one the real verifier accepts, not one that merely type-checks:** `an_sdk_minted_assertion_verifies_against_the_registration_this_module_builds` mints an assertion with the shipping `vpay-sdk` (the merchant SDK itself, added as a `[dev-dependency]` — not a test double) and feeds it to `authkestra_op::client_assertion::verify_client_assertion` at the pinned `=0.7.1`; `an_assertion_signed_by_a_key_this_merchant_did_not_register_is_refused` is the negative control. The `vpay:v1` audience the three parties must agree on is now one constant, `vpay_config::MERCHANT_AUDIENCE`, returned by `Surface::Merchant.audience()`, and a deployment whose merchant cannot target it **refuses to boot** (`ConfigError::MerchantMissingV1Audience`, proven by `a_merchant_client_that_cannot_target_the_v1_audience_is_rejected` against a fixture that is verbatim what `config/application.yml` shipped until this pass, plus `the_example_config_registers_its_merchant_for_the_v1_audience` on the real file). End to end, over a booted server on a real database, `backends/tests/integration/tests/merchant_token_flow.rs` covers all six claims this row makes: a token is obtained by the SDK and reaches the authenticated 404 (`an_sdk_client_authenticates_and_reaches_the_honest_404`), no bearer is a 401 envelope (`a_v1_request_with_no_bearer_token_is_the_401_envelope`), a disabled client is `invalid_client`/401 with no restart (`a_disabled_client_is_refused_with_invalid_client_and_401`), a dashboard-audience token this same server signed is refused on `/v1` (`a_dashboard_audience_token_is_refused_on_v1`), JWKS lists exactly the active `kid` and discovery matches the URLs the SDK derived independently (`the_jwks_and_discovery_documents_describe_this_process`), and one assertion cannot be spent twice (`the_same_client_assertion_cannot_be_spent_twice`). **🟡 and not ✅, for one reason and it is about evidence, not code:** those six tests **have never run under Docker, here or in CI** — the only observation of them passing is the implementer's single manual run against a scratch database on an already-running Postgres (header paragraph). When the CI `rust` job runs them green, this row is ✅ and should say which run. **Separately not done, and not blocked on that:** there is no `/v1` business resource for a valid token to reach — an authenticated request gets the honest 404, deliberately — and no rate limit on `/token` (ADR-0009 leaves it to ingress; nothing here verifies ingress does it). One property that is real and easy to misread as a bug: **an access token already issued to a client stays valid for its remaining TTL after that client is disabled** — the kill switch acts on token *issuance*, which is what a stateless bearer token means, and `a_disabled_client_is_refused_with_invalid_client_and_401` builds a fresh SDK client precisely so it tests the endpoint rather than a cache. The *client* side of the flow — the two merchant SDKs, `sdks/rust` and `sdks/nodejs` — is unchanged this pass and described in the "Merchant SDKs" section below; what changed is that the contract they were written against is now served by something. **Updated 2026-09-03 (Step 2), on two points.** (1) **The evidence is no longer one manual run.** All 7 `merchant_token_flow` tests (a seventh, `a_token_minted_with_no_audience_is_addressed_to_the_client_and_refused_on_v1`, joined the six listed above) ran under testcontainers on the authoring machine on 2026-09-03 as part of a 74-test container-backed run that passed clean — see the header. They have still never run in CI. (2) **Authentication now carries a tenancy decision, not only an identity one.** The middleware resolves the token's `client_id` to the YAML registration's `merchant_id` and puts a `MerchantScope` in request extensions; every `/v1` query filters by it, and a merchant asking for another merchant's `pi_…` gets the same 404 as for one that never existed (`merchant_b_cannot_read_merchant_as_intent`, integration). The scope check (`payments:write` / `payments:read`) is part of the same single validation. Still 🟡: no CI run, and no deployed vpay has ever completed this handshake for a real merchant. |
| Client-assertion replay protection (`oauth_client_assertion_jtis`, `0011_create-oauth-client-assertion-jtis.sql`) | 🟡 | Backs `authkestra_op::client_assertion::ClientAssertionStore::record_jti`, which neither of `authkestra-op`'s two shipped implementations can satisfy for vpay's deployment: `NoClientAssertionStore` fails closed unconditionally, and `MemoryClientAssertionStore` is single-process only (its own doc comment names exactly vpay's situation — multiple replicas — as needing "something shared... instead"). This table's `jti TEXT PRIMARY KEY` is the atomic single-use guard, meant to be used as `INSERT ... ON CONFLICT (jti) DO NOTHING` read via `rows_affected()`, never check-then-insert (the migration's own header comment explains the TOCTOU race a separate SELECT would reintroduce). Two dedicated tests in `postgres_smoke.rs` prove the constraint at the database level (`a_duplicate_client_assertion_jti_is_rejected_by_the_database`, `on_conflict_do_nothing_reports_zero_rows_affected_for_a_replayed_jti`). **New this pass: a real Rust implementation exists — `vpay_db::SqlClientAssertionStore`**, implementing `authkestra_op::client_assertion::ClientAssertionStore::record_jti` with exactly that `INSERT ... ON CONFLICT DO NOTHING` pattern, converting `authkestra-op`'s `chrono::DateTime<Utc>` boundary type to vpay's own `time::OffsetDateTime` convention explicitly at the crossing (`chrono_to_offset_date_time`, `client_assertion.rs`). **Proven race-safe, not just correct when called sequentially**: `concurrent_record_jti_calls_for_the_same_jti_yield_exactly_one_fresh_result` fires 10 concurrent `record_jti` calls with the same `jti` against a real Postgres and asserts exactly 1 reports fresh and 9 report replayed — the same shape of proof `authkestra-op`'s own `sqlx_store` tests use for `consume_code`. **Wired 2026-09-02 (Step 1): `MerchantOp::new` passes a `SqlClientAssertionStore` to `CompositeOpStore::with_client_assertion_store`, so every `/v1` token request goes through it.** That it is genuinely wired — rather than merely constructed — is what `the_same_client_assertion_cannot_be_spent_twice` proves: one assertion is sent by hand twice (the SDK correctly mints a fresh one per request, which is exactly why the SDK cannot reach this case), the first exchange succeeds, and the second is refused `invalid_client`/401 while the assertion is still well inside its own lifetime and would verify perfectly on its own. Drop `with_client_assertion_store` and that test fails. **Still 🟡, for two reasons.** (1) That test is Docker-backed and has never run under Docker — one manual scratch-database run is all the evidence there is (header paragraph). (2) **There is still no cleanup job.** What landed instead is a stopgap and is labelled one in the code: `vpay_db::delete_expired_client_assertion_jtis` runs **once, at `vpay-server` boot**, non-fatally, which bounds the table at roughly "assertions since the last restart" rather than "assertions forever". It is not a timer, the worker job loop is still ⛔, and a long-lived process still grows this table monotonically. The sweep's own correctness is proven by reading rows back rather than trusting a count (`expired_client_assertion_jtis_are_swept_and_live_ones_are_kept`, `backends/tests/integration/tests/client_store.rs`) — **a test that has not been run anywhere yet**. **Known limitation, recorded not fixed (security review 2026-09-02):** the replay namespace is global — `jti` alone is the primary key and the upstream `record_jti` seam carries no `client_id` — so a merchant using low-entropy `jti`s could collide with or pre-spend another merchant's; [docs/flows/merchant-auth.md](flows/merchant-auth.md) now states `jti` MUST be a UUID v4 (both SDKs comply) and leaves the `(client_id, jti)` re-keying as a maintainer decision |
| Disabled-clients kill switch (`disabled_clients`, `0012_create-disabled-clients.sql`) | 🟡 | An operator revocation mechanism for an OAuth client (dashboard or merchant `client_credentials`) that takes effect without a deploy — `client_id` plus a disable flag/reason, no credential and no identity of its own (YAML stays authoritative for identity; this table only ever *subtracts* access). Its uniqueness is proven by two tests in `postgres_smoke.rs`: `disabled_clients_accepts_an_insert` and `a_duplicate_disabled_client_id_is_rejected_by_the_database` (rejected specifically on the `client_id` primary key). **New this pass: query functions exist — `vpay_db::is_client_disabled`/`disable_client`/`enable_client`** (`vpay-db/src/disabled_clients.rs`), deliberately uncached (the module's own doc comment argues a cache would reintroduce the revocation delay this table exists to remove). `disabled_client_lookup_reflects_disable_and_enable` in `vpay-db/tests/repositories.rs` proves all three functions observe the same underlying table consistently against a real Postgres, including that `disable_client` is idempotent (a second disable of an already-disabled client updates `reason` without erroring) and `enable_client` is a no-op on a client that was never disabled. **Enforced 2026-09-02 (Step 1), and in the one place where enforcing it is sufficient.** `vpay_api::op::clients::YamlClientStore::find_client` consults `is_client_disabled` — and `find_client` is step 1 of `authkestra_op`'s `handle_token_request`, the single point every token request passes through for every grant. That is not a convenience: reading the pinned `authkestra-op-0.7.1/src/handlers/token.rs`, `handle_client_credentials` takes the already-resolved registration and mints straight through `TokenManager`, consulting no store afterwards, so a kill switch enforced anywhere else would not be enforced at all on the one grant `/v1` uses. Three properties, three tests. A disabled client is reported as `Ok(None)` — "no such client" — so the token endpoint cannot be used as an oracle for whether a merchant exists but is suspended (`find_client_reflects_the_disabled_clients_kill_switch`, integration, which also proves disable and re-enable take effect on the next lookup with no restart). An unknown `client_id` — the shape every credential-stuffing attempt has — is answered from the in-memory YAML index and never reaches Postgres (`an_unknown_client_id_is_refused_without_touching_the_database`). **And a failed lookup fails closed:** a database error becomes `OpError::Storage`, which `handle_token_request` maps to `server_error`, so an outage produces no token rather than a token for a client that may have been revoked (`a_failed_kill_switch_lookup_refuses_a_known_client_rather_than_admitting_it` — returning `Ok(None)` there would have rendered as `invalid_client` and pointed an operator at the merchant instead of at Postgres). End to end: `a_disabled_client_is_refused_with_invalid_client_and_401`. **Still 🟡, for two reasons.** (1) Evidence: both integration tests are Docker-backed and neither has run under Docker (header paragraph). (2) The switch acts on **issuance only** — an already-issued token remains valid for the rest of its TTL, which is what a stateless bearer token means and what ADR-0009's revocation-gap open question is about; nothing in this repo shortens that window. `disable_client`/`enable_client` are still called by no shipping code — an operator flips the row by hand, and **no runbook documents the `disabled_clients`-plus-YAML dual authority yet** |
| Dashboard auth (`/dash/v1` as an Authkestra OP) | 🟡 | Decision recorded in [ADR-0009](adr/0009-dashboard-oidc-provider.md), design in [docs/flows/dashboard-auth.md](flows/dashboard-auth.md). **Upgraded from ⛔ this pass, on the strength of the same three prerequisites "Merchant auth" above lists** — the dashboard client is now modelled and validated in config (`vpay_config::oauth::DashboardClient`), and `vpay_api::resource_auth::JwtValidator`/`AuthenticatedDashboard` pinned to `Surface::Dashboard` is proven to validate a correctly-audienced token and reject a merchant-audienced one on this surface specifically (`a_dashboard_audience_token_is_accepted_by_the_dashboard_validator`, `a_merchant_audience_token_is_rejected_by_the_dashboard_validator`, in `resource_auth.rs`). **Still no `/dash/v1` route, and a reader must not conclude login works from any of this**: no login has ever been performed, no token has ever been issued by this code, and no key has ever been rotated — `rotate_signing_key` (OP-2, row above) rotates to a key it is handed, it does not generate one. `authkestra-op`/`authkestra-engine`/`authkestra-axum`/`authkestra-resource` are pinned in the root `Cargo.toml`; `authkestra-resource` is now a genuine production dependency of `vpay-api` (for `JwtValidator`), and `authkestra-op`/`authkestra-engine` are production dependencies of `vpay-db` (for `SqlClientAssertionStore`, OP-2) — **so, unlike what this row used to say, `authkestra-*` is no longer dev-dependency-only; it is in both shipping binaries' resolved graph** (see the "Authkestra OP tables" row above and the "cargo deny" infrastructure row for the concrete consequence). **Status unchanged 2026-09-02 (Step 1) — still 🟡, and the reader must not infer otherwise from the merchant rows above: no login has ever been performed and no `/dash/v1` route exists.** Two of this row's stated prerequisites did close, and they are worth naming precisely because they are the ones most easily mistaken for the feature. (1) **Signing keys and JWKS are real now**: a key is generated, loaded, announced in `oauth_signing_keys` and published at `/v1/oauth/jwks.json` across a rotation window — see the "OAuth signing keys" and "Merchant OP" rows. (2) A shipping binary does now construct `SqlxOpStore<Postgres>` — but as three slots the `OpStore` supertrait demands and **no `/v1` grant reaches**, not as anything serving `/dash/v1`. What is still missing, and it is the whole feature: **no `/login` route, no `/authorize`, no `/dash/v1` anything**; **no `SessionStore`** — `authkestra-engine` is pinned with `features = ["rustls-no-provider", "token", "session"]` and **without `sql-postgres`**, so no SQL-backed session store is even compiled in; and a design problem this pass surfaced but did not solve — `authkestra-op`'s `default_handle_authorization_code` mints the access token with `Some(client_id)` as the audience (`authkestra-op-0.7.1/src/handlers/token.rs`, step 7), with **no requested-audience path at all**, so a token from that grant would carry `aud = <client_id>` and `Surface::Dashboard.audience()` (`vpay:dash/v1`) would reject every one of them. Whoever builds `/dash/v1` has to resolve that first; the merchant surface does not hit it because `handle_client_credentials` *does* honour a requested audience. Rotation is also restart-based (the "OAuth signing keys" row), so "rotating a signing key at least once" — this flow's own definition of done — has still never happened |
| Resource-server JWT validation (`vpay-api::resource_auth`, OP-3) | 🟡 | New this pass: `JwtValidator`, pinned per `Surface` (`Merchant` or `Dashboard`, distinguished by required `aud`), backed by `authkestra_resource::jwt::JwksCache` — fetched once and cached for `jwks_refresh_interval`, not a network round trip per request (confirmed by reading `authkestra-resource-0.3.4`'s own source, cited in the module doc, and re-confirmed unchanged at `0.7.1`: `JwksCache::get_key` still refreshes only on a cache miss or once the TTL has elapsed). `AuthenticatedMerchant`/`AuthenticatedDashboard` are axum extractors that pull a bearer token, validate it, and hand a handler `ResourceClaims { client_id, scope }`. **A real vulnerability class found and fixed, not merely inherited from the library:** `jsonwebtoken::Validation::validate_aud` defaults to `true` but its own doc comment says the check "only happens if `aud` claim is present" — a token minted with no `aud` claim at all would sail through unchecked. Fixed with `set_required_spec_claims(&["exp", "aud", "iss"])`, which makes the claim's mere presence mandatory before the membership check runs, and proven by `a_token_with_no_audience_claim_at_all_is_rejected`. 11 tests in `resource_auth.rs` cover this plus: a validly-signed token round-trips its claims and scopes; a token signed by a different key (same advertised `kid`) is rejected; an expired token is rejected; a merchant-audience token is rejected by the dashboard validator and vice versa (both directions proven, not assumed from one); an unrecognized `kid` is rejected rather than falling back to any available key; and, over a real axum router, a missing/malformed `Authorization` header and a valid bearer token each produce the right status and Stripe-shaped envelope. Every failure mode collapses to the same generic `invalid_token` response (`AuthRejection::InvalidToken`), deliberately, so the endpoint cannot be used as an oracle for *which* check tripped. **Mounted 2026-09-02 (Step 1).** `AuthenticatedMerchant` is now the layer in front of the whole `/v1` nest (`vpay_api::router`, "HTTP surface" above), so this module is on the path of every merchant request, not only its own tests: `an_unauthenticated_v1_request_is_401_not_404` and `the_unauthenticated_v1_401_is_the_stripe_shaped_envelope` drive it over the real router, and `an_sdk_client_authenticates_and_reaches_the_honest_404` / `a_dashboard_audience_token_is_refused_on_v1` drive it over a socket against a booted `vpay-server`. The provisional `vpay:v1` string is gone: `Surface::Merchant.audience()` returns `vpay_config::MERCHANT_AUDIENCE`, so the validator and the config validation rule cannot disagree about the spelling. `AuthenticatedDashboard` remains mounted on nothing, because `/dash/v1` does not exist. **Still 🟡, for two reasons.** (1) The router-level tests that cover the mounted path are unit-level for the 401 and Docker-backed for everything past it, and the Docker-backed ones have not run under Docker (header paragraph). (2) **The validator fetches its JWKS over an HTTP round trip to this same process's own loopback port** — `vpay-server` binds first, then builds the validator with `loopback_jwks_url(bound)` (`the_validators_jwks_url_is_always_loopback_on_the_bound_port`, `the_validators_jwks_url_ends_at_the_route_the_router_mounts`, unit tests in the binary). It is always loopback, never the public URL, so no external dependency is introduced — but a process validating its own tokens by asking itself over TCP is a seam that exists because `authkestra_resource` offers no in-process key source, not because it is desirable. It also means the row below is no longer hypothetical. **Two findings from the security review, both fixed and pinned without Docker:** (1) an unauthenticated caller could force one loopback JWKS fetch (a Postgres `SELECT`) per request and hold the cache's write lock across it by presenting junk tokens with random `kid`s — `authkestra_resource`'s `JwksCache` refreshes on every miss; `JwtValidator` now decodes the header first (no `kid` → refused with zero cache access), delegates immediately for a `kid` already in the cached JWKS, and otherwise grants at most one refresh per `UNKNOWN_KID_REFRESH_INTERVAL` (30 s) per process — `a_hundred_unknown_kids_force_at_most_two_jwks_fetches` asserts wiremock saw ≤ 2 fetches for 100 junk tokens (101 with the throttle disabled), and `a_refused_token_does_not_spend_the_permit_for_a_good_one_on_the_same_key` pins that the predicate is membership of the published key set, not "validated before" — the stated cost is that a token signed by a key this replica has not yet fetched can be refused for up to 30 s during a junk burst; (2) a JWKS fetch failure (our own endpoint down because Postgres is down) was rendered as `401 invalid_token`, which the SDKs answer by re-authenticating — an outage amplifier; it is now `AuthRejection::KeysUnavailable`, `Category::Storage`, a 503 `service_unavailable` envelope with `Retry::AfterBackoff` (`a_jwks_that_cannot_be_fetched_is_keys_unavailable_not_invalid_token`, `a_jwks_outage_is_a_503_envelope_over_the_router`, with `a_bad_signature_is_still_a_401_over_the_router` as the control); every claim/signature/unknown-key failure still collapses to the oracle-free 401. Also pinned: a token the OP mints with no requested audience (`aud = client_id`) is refused on `/v1` (`a_token_whose_audience_is_the_client_id_is_refused_on_the_merchant_surface`, and the Docker-backed `a_token_minted_with_no_audience_is_addressed_to_the_client_and_refused_on_v1`) — the decisive mutation is *widening* `set_audience`, not deleting it, since `jsonwebtoken` 11 fails closed on a missing audience list. **Found by the first `just demo` run, not by any test (2026-09-02):** inside the `FROM scratch` image `vpay-server` panicked at boot — `JwksCache::new` builds `reqwest::Client::new()`, which on the workspace's reqwest 0.13 pin loads trust roots from the OS store the image does not have (`No CA certificates were loaded from the system`), exactly the failure the root `Cargo.toml`'s comment on that pin predicted. The prescribed fix (`JwksCache::with_client`) was **not sufficient**: `with_client` replaces a client `new` has already constructed, and 0.7.1 (the latest release) has no other constructor. So `vpay_api::jwks_cache` is a deliberate, narrowed port of `authkestra_resource::jwt::JwksCache` + `validate_jwt_generic` (~15 lines of refresh policy; every cryptographic step still calls authkestra's `Jwks::fetch_with`, `Jwk::to_decoding_key` and `jsonwebtoken::decode`), taking the client as a constructor argument, and `vpay_api::http_client::client()` builds that client on vendored `webpki-roots` + `ring` via `tls_backend_preconfigured` — the twin of `sdks/rust`'s `rustls_client_config`. The module doc lists the deviations and the re-diff obligation on an authkestra bump; the clean answer is an upstream constructor that takes a client, after which the port can be deleted. **Proven three ways:** `a_server_with_no_os_trust_store_boots_and_still_validates_tokens` in `vpay-server`'s `tests/cli.rs` spawns the real binary with `SSL_CERT_FILE`/`SSL_CERT_DIR` pointing at nothing and asserts `/healthz` 200 and a bogus-`kid` `/v1` request answering the 401 envelope (a 503 would mean the fetch failed) — it fails with the original panic when `http_client::client()` is replaced by `reqwest::Client::new()`; the real image booted and answered the same two requests under `docker compose` on the authoring machine; and the CI `e2e (compose)` job exercises the same path. **Latent, stated:** `authkestra-engine` still writes `reqwest::Client::new()` in its device-flow, client-credentials-flow and captcha modules — none reachable from vpay today; if one ever becomes reachable it panics in the image the same way, and `install_crypto_provider` does not prevent that. **Remediation review, later the same day:** the port's `get_jwks` refresh re-checks the entry under the write guard (`refresh_if_stale`), so waiters that queued behind the first refresh at a TTL boundary reuse its result instead of fetching again — a fifth deviation from upstream, documented in the module. Measured before claiming: with the re-check deleted, 32 callers released on a barrier produced 1 extra fetch on 17 of 20 boundaries and 2 on 3, never 32 — `tokio`'s write-preferring `RwLock` was already doing most of the coalescing — so this removes an occasional redundant `SELECT` taken while every validation is queued, not an N-fold amplification. The concurrent form of the test passes with the bug present most of the time and was deliberately **not** shipped; `a_caller_that_reaches_the_refresh_with_a_fresh_entry_does_not_fetch_again` pins the property deterministically (1 fetch with the re-check, 2 without). **Changed 2026-09-03 (Step 2): validation happens exactly once per request (D3).** The `/v1` boundary was an *extractor* (`AuthenticatedMerchant`), which meant every handler that wanted claims paid for a validation, and a handler that forgot to ask for it was unauthenticated by omission. It is now a **middleware** on the nest: it validates the bearer token once, checks the method's required scope, resolves the tenant, and puts a `MerchantScope` in request extensions. `MerchantScope`'s only public constructor is the middleware, and its `FromRequestParts` fails closed with a `500` rather than falling back to any tenant if the middleware is not mounted — reaching a handler with no scope means the layer is missing, and the safe answer to "whose rows may I read" is none of them. The extractor's own test suite is unchanged and still passes (`a_valid_bearer_token_reaches_the_handler_with_claims_attached`, `a_token_without_the_required_scope_is_403_not_401`, and the JWKS-throttle cases above); what is new is that a real resource path depends on it |
| rustls `CryptoProvider` process default, for `authkestra_resource::jwt::Jwks::fetch` | ✅ | **Closed 2026-09-02.** Both `vpay-server` and `vpay-worker-bin` now call `rustls::crypto::ring::default_provider().install_default()` (`install_crypto_provider()` in each `main.rs`) as the second thing in `run()`, after the signal handlers and before tracing init, so no client construction can precede it. The result is `.ok()`-dropped on purpose — `Err` means a default already exists, which is the wanted state — per the root `Cargo.toml`'s own note on the `authkestra-*` pins; no `unwrap`/`expect`. **What the ✅ rests on:** a unit test per binary (`installing_the_crypto_provider_leaves_a_process_default_and_is_idempotent`) asserts `CryptoProvider::get_default()` is `Some` afterwards and that a second call does not panic — emptying the function's body fails both. The existing exit-69 subprocess tests spawn the real binaries through this call and on to the database stage, so it is exercised on a real startup. **Updated 2026-09-02 (Step 1): the path that used to panic now runs in a shipping process.** `vpay-server` builds a `JwtValidator` at startup, and the first authenticated `/v1` request makes it fetch its own loopback JWKS — a real `Jwks::fetch`, not a test one. **What proves it, and how strongly:** `an_sdk_client_authenticates_and_reaches_the_honest_404` and the rest of `backends/tests/integration/tests/merchant_token_flow.rs` boot the real router in-process and complete that fetch; the test binary installs the provider itself at the top of `harness()` for exactly the reason this row exists, and its own comment says so. That is an in-process exercise of the fetch, and it has run once, manually, against a scratch database — **never under Docker or in CI** (header paragraph). It is therefore stronger evidence than this row had before and weaker than "a shipping `vpay-server` container has served an authenticated request": the CI `e2e (compose)` job boots `vpay-server` but its Cypress spec only touches the dashboard, so no containerised `/v1` request has ever been made. The rail adapters are still `NotImplemented`, so no rail client has ever been built. The previous row's analysis — that `vpay-db` never needed this because sqlx builds its own provider inline, and that `sdks/rust` sidesteps it with a pre-built `ClientConfig` — is unchanged and still correct. **Scope narrowed 2026-09-02:** the JWKS client this row was written for is now built by `vpay_api::http_client` from a pre-configured rustls `ClientConfig`, so it no longer consults the process default at all (see the row above); the `install_default()` call stays in both binaries because `authkestra-engine`'s own `reqwest::Client::new()` call sites would need it if ever reached, and because it costs nothing |
| Webhooks (signing, outbox, delivery) | ⛔ | |
| Idempotency | 🟡 | **New 2026-09-03 (Step 2); was ⛔.** `Idempotency-Key` is **required** on every `/v1` `POST` (stricter than Stripe, where it is optional; both SDKs already always send one), 1–255 printable-ASCII bytes, scoped to the merchant, stored 24 hours in `idempotency_keys` (migration `0015`). A request is identified by a SHA-256 over method, path and raw body, framed so the three cannot be shifted across each other; the stored digest is compared with `subtle::ConstantTimeEq` so the endpoint is not a hash oracle. The claim is one `INSERT … ON CONFLICT`, never check-then-insert. **The six behaviours and the test behind each, all of them run against a real Postgres on 2026-09-03:** a *replay* returns the stored body byte for byte and writes no second row (`a_replayed_idempotency_key_returns_the_same_object_and_no_second_row`, integration; `a_completed_idempotency_key_replays_its_stored_response`, `vpay-db`); a *mismatch* — same key, different body — is `400 idempotency_key_in_use` (`a_reused_key_with_a_different_body_is_the_400_envelope`; `reusing_an_idempotency_key_with_a_different_request_is_a_mismatch`); a key whose first request is *in flight* is `400 idempotency_key_in_flight`, its own code (`a_key_whose_first_request_is_still_running_is_answered_with_its_own_code`; `concurrent_claims_of_one_idempotency_key_yield_exactly_one_fresh`); a `5xx` *releases* the key so the retry re-executes (`a_5xx_releases_its_idempotency_key_so_the_retry_re_executes`; `release_hands_back_an_in_flight_key_and_never_a_completed_one`); an expired in-flight key is *reclaimable* while a live one is not (`an_expired_in_flight_key_is_reclaimable_and_a_live_one_is_not`), and a claim that was superseded by a reclaim can neither overwrite the new response nor delete the new claim — the ABA case, closed by a `claim_id` the database mints on every claim and that `store` and `release` both match on (`a_reclaimed_key_is_not_writable_by_the_claim_it_replaced`); and *sweep* deletes only rows past their window (`sweep_expired_removes_only_the_rows_past_their_window`). A missing header is the documented `400` naming `idempotency_key` (`a_post_without_an_idempotency_key_is_the_documented_400`), and a replay answers what the original answered even after the deployment changed underneath it (`a_replay_survives_the_rail_being_disabled`) — which is why `create` claims *before* validating and releases on a validation failure. **🟡, not ✅, for three named reasons.** (1) **The status code is `400` where Stripe answers `409`** for a key still in flight: [ADR-0011](adr/0011-error-modelling.md) derives the status from `Category`, `Category::Idempotency` is `400`/`idempotency_error`, and splitting one Stripe `type` across two statuses would be an ADR-level change. **That is a maintainer decision and this pass did not take it**; the `code` distinguishes the two cases either way, and `ApiError::IdempotencyKeyInFlight`'s doc comment records the trade. (2) **Nothing sweeps on a schedule.** `vpay-server` calls `sweep_expired` once at boot as a labelled stopgap and `vpay-worker-bin` calls it never, because there is no job loop; a long-lived deployment grows the table between restarts. (3) It covers `POST /v1/payment_intents` and its two sub-resources only — there is no other `POST` on `/v1` to cover |
| Reconciler | ⛔ | *Unchanged by Step 3 — no job loop, no escalation, no callback route. Said explicitly because Step 3 is the phase the roadmap used to put recovery in: see the Phase 4a/4b split in [docs/roadmap.md](roadmap.md).* |

### Unimplemented items tracked by `verify-status`

Every token below appears verbatim in shipping source. Removing an item here
without removing it from the code fails CI, and — since 2026-09-03 — so does
leaving an item here that no shipping code carries any more. The scanner is
comment-aware, so a token quoted in a doc comment neither declares anything
nor counts as one (`a_token_quoted_in_a_comment_is_not_a_shipping_claim` in
`xtask`).

**There is exactly one, down from eight on 2026-09-03 (Step 3), and the two
that left did so for opposite reasons — which is the distinction this list
exists to keep visible.** Six went because the code was *written*:
`{mtn_momo,orange_money}::{submit, query_status, parse_callback}` are real
HTTP calls now. `orange_money::refund` went because it was **never unbuilt
work in the first place** — Orange's Web Payment product documents no refund
API, so the adapter stops overriding the port and inherits the trait's
default `ProviderError::Unsupported`: a permanent capability answer the core
can branch on (`supports_refunds: false`), asserted by the conformance case
`a_rail_without_the_refund_capability_answers_unsupported`. A rail that will
never support an operation must not be described with the same token as work
someone still has to do.

- `mtn_momo::refund` — MTN refunds are a different product (Disbursements)
  with its own subscription key and its own token scope; nothing in
  `config/application.yml` or `ProviderHost` carries a disbursement key and
  no deployment has been issued one, so nothing honest can be built yet
  (Step 3 design, decision 3). `supports_refunds` stays **`true`** for
  `mtn_momo` on purpose: the *rail* refunds, and answering `Unsupported`
  would be a lie about MTN rather than an admission about us. `refund` is
  therefore the one operation on the one rail that still returns a token —
  reachable only through `POST /v1/refunds`, which is not routed, so no
  caller can currently provoke it
  (`refund_is_not_implemented_and_does_not_pretend`,
  `unimplemented_operations_never_fabricate_success` in the conformance
  suite).

### Adapters

Both rails' wire calls are implemented and **proven against a real
`wiremock/wiremock` container**, never against MTN or Orange. The column
split below is the whole point of this section: ✅ means a conformance case
would fail if the behaviour broke; 🟡 means the code is real and its only
witness is a stub; ⛔ means not built.

| Rail | Capabilities | Wire calls (vs. WireMock) | Real sandbox | Callbacks | Refunds |
|---|---|---|---|---|---|
| `mtn_momo` (push) | ✅ declared and tested | ✅ `submit` / `query_status` / `parse_callback` | ⛔ never called | 🟡 parsed, never received | ⛔ `NotImplemented("mtn_momo::refund")` |
| `orange_money` (redirect) | ✅ declared and tested | ✅ `submit` / `query_status` / `parse_callback` | ⛔ never called | 🟡 parsed, never received | ✅ `Unsupported` — permanent, capability-driven |

**What the ✅ in the wire-call column rests on, named.** The shared
conformance suite ran **26 tests, 26 passed, 0 skipped** on 2026-09-03 (measured for
this note, as part of the 115-test container run in the header): 4
capability cases plus 11 port cases parameterised over both rails, each
against a container started by `vpay_testkit::containers::start_wiremock`
and reached over HTTP exactly as a rail is (ADR-0006 — a stub rail is a
WireMock *host*, never a linked implementation). The 11:
`submit_returns_a_reference_and_a_flow_shaped_result` (push returns no
`redirect_url`; redirect returns a URL **and** `pay_token` in one value),
`duplicate_submit_reports_submitted_not_an_error`,
`not_found_is_never_on_its_own_a_failure`,
`a_declined_charge_maps_to_the_documented_failure_code`,
`an_unavailable_rail_is_a_transport_error_never_a_decline`,
`bad_credentials_are_not_reported_as_a_payer_problem`,
`a_callback_body_round_trips_to_identifiers_only`,
`a_rail_without_the_refund_capability_answers_unsupported`,
`pending_then_successful_walks_the_scenario`,
`redirects_are_refused_and_never_followed` and
`an_oversized_rail_body_is_refused_at_the_cap`. Under them sit 48 unit tests
in `vpay-adapter-mtn-momo` and 53 in `vpay-adapter-orange-money` (0 ignored,
measured the same day) covering the request bodies, the failure tables, the
token caches and every redaction. **The three `#[ignore]`d conformance cases
this section used to name are gone**, and `just verify-ignored` now pins
`expected_ignored := "0"`.

**What the ⛔ and 🟡 columns mean, plainly.**

- **Real sandbox — ⛔.** Neither adapter has ever exchanged a byte with MTN
  or Orange. Every assertion above would still pass if
  `docs/flows/adapter-mtn-momo.md` and `docs/flows/adapter-orange-money.md`
  were wrong about the rails, because the mappings were written from those
  documents. Both docs' "to confirm with the rail" lists stand in full.
- **The 401 → re-mint → retry path is unproven on both rails.** The logic
  exists and is bounded at one retry, but no mapping returns 401 from
  `requesttopay` / `webpayment` *after* a good token. What is proven is the
  401 on the token endpoint itself
  (`bad_credentials_are_not_reported_as_a_payer_problem`).
- **Callbacks — 🟡.** `parse_callback` is implemented and tested on both
  rails and returns identifiers only, never a status. **There is no callback
  route**: nothing in a running vpay ever calls it, nothing compares
  Orange's `notif_token` against a stored one, and MTN's callbacks are
  unsigned and unauthenticated in any case. Orange's parser fails closed
  without a `notif_token` (`a_callback_without_a_notif_token_is_refused`).
- **Orange's duplicate-submit semantics is an assumption.** The stub returns
  the same `pay_token` for a repeated `order_id`, and the port requires a
  duplicate to be `Submitted` rather than an error, but that is a property
  of the mapping, not an observation of Orange.
- **MTN's `externalId` carries the provider *reference*, not the charge id**
  — `ChargeRef` gives an adapter no charge id — and `payerMessage` is not
  sent.
- **Proxies are deliberately refused.** `vpay_provider::http` ignores
  `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`; a deployment behind a corporate
  egress proxy is not served by this client. The merchant SDK's twin
  (`sdks/rust/src/client.rs`) keeps proxy support on purpose, because a
  merchant's process runs on a merchant's network.
- **`vpay-worker-bin` builds an HTTP client and an adapter map, and calls no
  rail with either.** They are used only for the boot-time YAML↔adapter join
  (`boot_seeds`, which exits `78` for a configured rail with no linked
  adapter); there is no job loop to call `submit` or `query_status`.

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
| `compose.yml` (Postgres + 2 WireMock rails) | ✅ | Written; **never started as a stack on any authoring machine** (Docker Hub unreachable from one; the rootless daemon on another cannot start containers at all). **Started for the first time by the CI `e2e (compose)` job on run `33647189156` (2026-09-02)**, with both WireMock rails and Postgres up and `vpay-server` answering `/healthz` 200 against them — see "GitHub Actions" below. `config/application.yml` used to point both rails at a host named `wiremock`, which no compose file defines; fixed 2026-09-02 to `wiremock-mtn` / `wiremock-orange` (with Orange's `/orange-money-webpay/dev` path prefix, per its flow doc), proven by `vpay-config`'s own test that loads the real file. **Changed 2026-09-03 (Step 3): the mappings these two services bind-mount are now the ones the conformance suite drives.** `backends/tests/conformance/wiremock/{mtn,orange}/mappings/` gained the full failure vocabulary per rail, a WireMock *scenario* (pending → successful), a redirect mapping (`REF_REDIRECT`) and an oversize-body mapping (`REF_HUGE`); a mapping fixed for the suite is fixed for `just up` and for `just demo` in the same edit, because it is one directory. The suite does **not** use this compose stack — it starts its own `wiremock/wiremock` containers via testcontainers, so CI's `rust` job needs no compose services |
| `compose.e2e.yml` (full stack) | ✅ | **Could not have booted before 2026-09-02**: both binaries exit 78 without `--config`/`VPAY_CONFIG` (mandatory since 2026-08-11), and this file never set it. Now sets `VPAY_CONFIG` and the rail `${VAR}` placeholders (stub values for stub rails) on both services — **six of them as of 2026-09-03 (Step 3)**, up from three: `MTN_SUBSCRIPTION_KEY`, `MTN_API_KEY`, `MTN_API_USER`, `ORANGE_MERCHANT_KEY`, `ORANGE_CLIENT_ID`, `ORANGE_CLIENT_SECRET`, all present on **both** `vpay-server` and `vpay-worker` (miss one on either and the process exits `78` at boot — an unresolved placeholder is fatal by design, never an empty string). All six are listed in `.env.example`. **Run for the first time by CI's `e2e (compose)` job on run `33647189156` (2026-09-02)**: `vpay-server` answered `/healthz` 200 one second after the stack came up, the dashboard answered 200 on `/` a second later, and Cypress passed — see below. **Changed after that run, and therefore not covered by it (Step 1, same day):** `vpay-server` now exits `78` without an RS256 signing key, so this file sets `VPAY_OAUTH_SIGNING_KEY_FILE=/secrets/oauth-signing-key.pem` and read-only bind-mounts `.e2e/oauth-signing-key.pem` (git-ignored, `0644` because the scratch image runs as UID 65532, generated per stack by `just gen-e2e-signing-key` and thrown away with it; CI runs that recipe before `docker compose up`). **No CI run has yet booted the stack with that mount** — the ✅ above is evidence for the previous shape of this file, and the next `e2e (compose)` run is what proves the new one. **Step 3 added the three new rail variables to this file and to nothing else that runs it**, so the same caveat now covers them: they are correct by inspection and by `docker compose config`, and no run has booted the stack since |
| `backends/Dockerfile` (musl → scratch) | ✅ | Last rewritten 2026-08-09 (musl host target, UID 65532). **Could not have produced a bootable image before 2026-09-02**: it never copied `config/` in, so there was no file for `VPAY_CONFIG` to name. Now bakes `config/` at `/config` and sets `ENV VPAY_CONFIG=/config/application.yml` in both runtime stages (secrets stay `${VAR}`, so the layer holds none). **A second reason it could never have built, found in review the same day:** the builder stage copied every workspace member except `sdks/rust`, and cargo refuses to load a workspace whose `members` list names a missing directory — proven by reconstructing the build context outside Docker and running the Dockerfile's own `cargo build`, which failed at manifest load; `COPY sdks/rust` added. **A third, found by the first CI build that reached the Docker step (run `33646048616`, the fix's own PR):** on the alpine builder the host triple *is* `x86_64-unknown-linux-musl`, and with no `--target` cargo applied `.cargo/config.toml`'s `+crt-static` rustflags to proc-macros too, which cannot be static (`cannot produce proc-macro for async-trait`). The build now passes `--target` set to the builder's own host triple (still never a cross-compile) and copies from `target/<triple>/dist/`; the Dockerfile's header comment explains it. It was the last reason: the next run, `33647189156`, built both images and booted the stack. **Built for the first time by CI's `e2e (compose)` job on run `33647189156` (2026-09-02), and the resulting `scratch` image booted, found its baked config, connected to Postgres, ran the migrations and answered `/healthz` 200.** Never built on an authoring machine — see below |
| `frontends/Dockerfile` | ✅ | Last rewritten 2026-08-09. **Never built anywhere yet**: not on an authoring machine, and CI's `e2e (compose)` job, which will build it, has never reached its Docker step. **Built for the first time by CI's `e2e (compose)` job on run `33647189156` (2026-09-02); the standalone Next server answered 200 on `/`.** Its build context had been checked beforehand the same way as the backend one — `pnpm install --frozen-lockfile --filter @vpay/dashboard...` against a reconstruction of exactly what it copies passes the lockfile consistency check with `examples/` and `sdks/nodejs` absent — see below |
| `deny.toml` | ✅ | `cargo deny check` passes clean: `advisories ok, bans ok, licenses ok, sources ok`. The three advisories that failed before were fixed by **upgrading dependencies, not by suppressing them** — see below. One advisory is explicitly ignored: **RUSTSEC-2023-0071** (Marvin Attack in `rsa`, no patched release, an unconditional dependency of `authkestra-engine` per [ADR-0009](adr/0009-dashboard-oidc-provider.md)), accepted deliberately with the reasoning recorded inline in `deny.toml`. **This entry was preemptive when added and now genuinely fires — and this pass found that the previous pass's own note on *how* it fires was already stale, before this note could even be written once.** The last pass said `authkestra-op`/`authkestra-engine` reached `rsa` only via `vpay-tests-integration`'s dev-dependencies, so "the exposure itself is still narrower than 'in production' ... no shipping binary pulls it in." **That is no longer true, independently re-run and confirmed for this update:** `vpay-db` added `authkestra-op` as a genuine, non-dev dependency this pass (for `SqlClientAssertionStore`, OP-2), and both `vpay-server` and `vpay-worker-bin` depend on `vpay-db`. `cargo tree -i rsa` now shows `rsa v0.9.10 ← authkestra-engine ← authkestra-op ← vpay-db ← vpay-api/vpay-server/vpay-worker-bin`, with no `(dev)` marker anywhere on that specific path (the pre-existing `vpay-tests-integration` dev-only path still exists too, unchanged, in parallel). `cargo deny -L info check advisories` still reports the same `note[advisory-ignored]`/`note[vulnerability]` pair it did before — nothing about the ignore mechanism changed, and `cargo deny check` still exits 0 with 0 errors, so this is **not a CI regression**. What changed is the honesty of this row's own claim about scope: `rsa`'s Marvin-Attack timing side-channel is now reachable from both shipping binaries' production dependency graph, not merely from a test-only crate, even though nothing in either binary calls into `rsa` yet (no shipping code path constructs anything from `authkestra-engine`/`authkestra-op` — see "Merchant auth"/"Dashboard auth" above). The original `deny.toml` comment's own reasoning for accepting the advisory (no patched release exists; RS256 has no alternative in this stack; `/dash/v1` is staff-only, not the merchant payment path) does not depend on which dependency edge is dev-only, so the acceptance itself still stands — only the "no shipping binary pulls it in" line needs correcting, which this row now does. Also bans `aws-lc-rs`/`aws-lc-sys` so a second rustls crypto provider cannot reappear. **New this pass:** `CDLA-Permissive-2.0` was added to the allow list, with its justification recorded inline — it covers `webpki-roots` (Mozilla's CA bundle, data not code), pulled in through `sqlx`'s `tls-rustls-ring` feature now that `vpay-db` is a non-dev dependency using it (root `Cargo.toml`'s own comment: previously latent in the workspace's pins, now actually reachable). `tls-rustls-ring` (vendored roots) was chosen deliberately over `tls-rustls-ring-native-roots`: the runtime image is `FROM scratch` ([ADR-0004](adr/0004-musl-mimalloc.md)) with no OS trust store for `rustls-native-certs` to read, so native roots would fail TLS to Postgres in the shipped image only, while passing locally and in CI where a trust store exists — exactly the kind of gap that would not be caught until a real deployment. `rustls-native-certs` does still appear in the dependency graph (via `bollard → testcontainers → vpay-testkit`), but only as a `[dev-dependencies]` chain — `cargo tree -i rustls-native-certs` shows every path terminating in a dev-dependency of `vpay-testkit`/`vpay-db`/`vpay-tests-integration`, never a shipping binary, independently confirmed for this update |
| GitHub Actions | ✅ | **Correcting this row, which said "never executed": by 2026-09-02 the `ci` workflow had run 13 times (2026-08-09 → 2026-09-02, every one on a pull request) and failed all 13** (`gh run list --workflow ci`; per-job conclusions from `gh run view`). Job by job: `self-checks` passed 13/13; `rust` passed 10/13 (failed on `31317876404`, `31319267218`, `33618568372`) — on the latest run, `33626567174`, it ran `cargo nextest run --workspace` on `ubuntu-latest` with a working Docker daemon, container suites included, and reported `320 passed, 3 skipped`, which is the evidence for every container-backed row on this page; `supply chain` passed 11/13; `web` passed only the last 2 (the `pnpm -r test` Cypress-script bug fixed in the SDK pass); **`e2e (compose)` failed 13/13** — the first eleven at `pnpm/action-setup@v4` (the `packageManager` conflict `bf9811d` fixed), the last two at `pnpm exec cypress install`, because the workflow set `CYPRESS_INSTALL_BINARY: 0` for *all* jobs and then asked Cypress to install. The Docker steps after that never ran once. Two more defects: `on.push.branches` said `main` (the default branch is `master`, so nothing ever ran on a merge), and the `rust` job's flow-style `{ components: rustfmt, clippy }` parsed `clippy` as a stray key. **All fixed 2026-09-02**: the e2e job downloads Cypress normally and verifies it, builds both images, polls `/healthz` for a 200 and the dashboard for a 200 before running the spec; the compiler version is read from `rust-toolchain.toml` (now pinned to `1.95.0`, matching `backends/Dockerfile`) in every Rust job of `ci.yml` and `docs.yml`; and a new `just verify-ignored` step fails the `rust` job if the ignored-test count is not exactly 3, the number of test binaries is not exactly 30 (the check that actually catches a binary dropping out — 18 of the 30 hold eight tests or fewer), or the suite shrinks below 320 tests. **`expected_suites` moved 30 → 32 on 2026-09-02 (Step 1)**, for the two new `vpay-tests-integration` binaries `client_store` and `merchant_token_flow`; the raised value has not yet been exercised by a CI run. **Run 14 of the workflow — `33647189156`, on this fix's own pull request (#14) — is the first green `ci` run in this repository's history**: all five jobs passed; the `rust` job reported `329 tests run: 329 passed, 3 skipped` and `verify-ignored: 3 ignored (expected 3), 30 test binaries (expected 30), 332 total`; the `e2e (compose)` job built both images (5 min 21 s), got `/healthz answered 200 after 1s` and `dashboard: / answered 200 after 2s`, and Cypress ran the one spec (`dashboard.cy.ts`, 3 passing). The run before it, `33646048616`, was the first ever to reach the Docker step and failed there — see "Docker / compose" below for the proc-macro finding it produced. **✅ as of run `33650294682` (2026-09-02), the first push-triggered run on `master` in the repository's history, triggered by the merge of #14 and green on all five jobs.** The claim this row makes — the workflow runs on the default branch and on pull requests, builds the images, boots the stack and runs every suite — would fail visibly if it broke, which is the bar for ✅ here . **Nothing changed here in Step 3 (2026-09-03): no workflow file was touched, and no CI run exists for the `claude/step3-rails` branch.** The conformance suite starts its WireMock containers with testcontainers precisely so the `rust` job needs no new services; whether that works on GitHub's runners is unproven, and the counts in this pass's header were all measured on the authoring machine |
| Local demo (`just demo`, `examples/merchant-demo`, `compose.demo.yml`) | 🟡 | New 2026-09-02. `just demo` generates a throwaway server signing key and a demo merchant keypair (`just gen-demo-keys`: `cargo xtask gen-signing-key` for the merchant, its public JWK written into a git-ignored `demo` profile overlay `.e2e/application-demo.yml` that `compose.demo.yml` bind-mounts beside the baked base config), brings up `compose.yml` + `compose.e2e.yml` + `compose.demo.yml`, waits for `/healthz`, and runs `cargo run -p merchant-demo` — a Rust binary using `vpay-sdk` that prints one line per step: discovery and JWKS, an access token's decoded claims (never the token), the 401 envelope without a bearer, and the authenticated 404 `unknown_route` for `payment_intents().retrieve(..)` with the sentence "payment intents are not built yet — this is where the next step lands". `compose.demo.yml` publishes no host port for Postgres (`ports: !reset []`) because 5432 is the most commonly occupied port on a developer machine and the demo never reaches Postgres from the host. **🟡, not ✅:** the demo is an assertion harness a human reads, not a test CI runs — nothing fails a build if it regresses; and it demonstrates authentication only, because no `/v1` resource exists yet. Its first run found the runtime-image panic recorded in the "Resource-server JWT validation" row, which is the kind of thing it exists to find. **Updated 2026-09-03 (Step 2): four steps became five, and the fifth is the honest one.** The demo now runs discovery + JWKS, a token, the unauthenticated `401`, then **`payment_intents().create(…)` followed by `.retrieve(…)` through the shipping Rust SDK** — a real write to a real database, asserting the retrieve returns the object the create did — and finally **`.confirm(…)`, whose success condition is a `501 not_implemented`**. A printed payment intent that had been *confirmed* would mean something fabricated one. `just demo` is also port-configurable now: `just demo_port=18080 demo` propagates one number to the three places that must agree — the published port, the demo overlay's `deployment.public_base_url` (which becomes the OP's `issuer`, and a mismatch is an `invalid_client` whose message names no port), and `VPAY_BASE_URL` for the demo binary — and `gen-demo-keys` regenerates the overlay when either that URL or the newly-required `merchant_id` field is missing from it, a check added after a measured failure in which `just demo` spent its whole 120 s readiness budget on a crash loop while the recipe reported it had kept a file that no longer loads. **Updated 2026-09-03 (Step 3): still five steps, and step 5 now *succeeds*.** `payment_intents().confirm(…)` reaches the compose stack's WireMock MTN rail over HTTP and the demo asserts the intent came back **`processing`** with `next_action: null` — a push rail's one success state — then re-reads it and asserts the confirm's response and the later retrieve are the same object, so a status the handler rendered but did not commit would fail the run. It also asserts the *opposite* of what it used to: a confirm that does **not** reach `processing` is now the failure. The demo intent is **EUR**, because `config/application.yml` puts `mtn_momo` on EUR (MTN's sandbox rejects XAF) and `/v1` refuses a confirm whose intent currency is not the rail's — a property of the profile, expressed as config, never a code branch. **Still 🟡, and for one more reason than before:** the demo is an assertion harness a human reads, nothing fails a build if it regresses, **and this note did not run it** — `just demo` was last run green on 2026-09-02 in its four-step form; the five-step form's evidence is the integration suite (`confirm_rails.rs`), not the demo. And `processing` is not `succeeded`: nothing polls the charge the demo just created |
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
- ~~**Array encoding — still open, and still untestable.**~~ **Decided by
  the server, 2026-09-03: both spellings are accepted.** `vpay_api::form`
  decodes Stripe's indexed form (`k[0]=v`, what both SDKs send) and the
  unindexed form (`k[]=v`, what the curl examples use) into the same array,
  pinned by `both_array_spellings_produce_the_same_array`, with
  `array_indices_order_the_elements_and_holes_are_compacted` fixing what an
  index means. `create_payment_intent_body_decodes_to_the_params_the_sdk_encoded`
  and `confirm_body_decodes_to_the_params_the_sdk_encoded` carry the exact
  strings the SDK encoder emits. The decoder is hand-rolled rather than
  `serde_urlencoded`, because bracket nesting is not a shape that crate
  models; it refuses a duplicated scalar key rather than taking the last one,
  refuses a key used as both scalar and container, bounds nesting at 8, and
  is the reason `VpayForm`/`VpayQuery` exist at all.
- ~~**Whether ADR-0010's YAML-only merchant registry still stands**~~ now
  that `authkestra-op` 0.7.1 persists `token_endpoint_auth_method`/`jwks`.
  **Decided: kept.** `vpay_api::op::clients::YamlClientStore` resolves
  identity from `merchant_clients` in YAML and consults the database only
  for `disabled_clients`, which can subtract access but never grant it.
  The cost of that decision is unchanged and still real: merchant
  onboarding is a PR-then-deploy (ADR-0003, no hot-reload), and a rolling
  deploy has a window where old and new pods disagree about the client list.

**Updated 2026-09-03 (Step 2): the Rust SDK has now completed resource
calls, not only the handshake.** `payment_intents().create(…)`,
`.retrieve(…)`, `.list(…)`, `.confirm(…)` and `.cancel(…)` are exercised
against a real `vpay_api::router` and a real Postgres in
`backends/tests/integration/tests/payment_intents.rs`, and the wire object the
server renders is decoded by the shipping SDK's own type in
`the_merchant_sdk_deserialises_what_this_renders` — not by a copy of it
written for the test. **`.confirm(…)` succeeds only in the sense that it
receives the documented `501`**, which is what the demo's fifth step asserts
too. **The Node SDK's model is exercised against this renderer by nothing**:
`vpay_api::model`'s tests pin the object against the Rust SDK's fixture and
type only, and Node's parity rests on the SDK-to-SDK form-body tests
described above. Three of the SDKs' eight resource endpoints — refunds,
events, balance — still have no route at all.

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
(`publish = false` / `private: true`). *The reason given here used to be
"until a `/v1` resource exists for it to call"; five of the eight now exist,
so the reason is different and narrower:* nothing either SDK calls can
actually take a payment, `confirm` answers `501`, and refunds, events and
balance are still a `404` — so `examples/merchant-node` and
`examples/merchant-curl` can authenticate, create, retrieve, list and cancel
against a running vpay, and get a `501` or a `404` for everything else.

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
   with the `#[ignore]`s removed. **Updated 2026-09-03 (Step 3): the literal
   words of this item are met; the item is not.** Both adapters make real
   HTTP calls, and the shared suite passes with **no `#[ignore]`s left at all**
   — 26 tests, 26 passed, 0 skipped, measured on the authoring machine
   (`just verify-ignored` now pins `expected_ignored := "0"`). What every one
   of those 26 talks to is a `wiremock/wiremock` container. **Neither MTN's
   nor Orange's real sandbox has ever been called**, so this item stays open
   in the sense that matters for taking a payment: what is proven is that the
   adapters implement the protocol `docs/flows/adapter-*.md` describes, not
   that those documents are right. Also unproven: the 401-after-a-good-token
   re-mint path on both rails; Orange's duplicate-submit idempotency (an
   assumption about the rail, not an observation); and the callback path,
   because no callback route exists. `mtn_momo::refund` remains
   `NotImplemented` — see the token list above.
3. The worker's job loop, poll ladder and reconciler, with crash tests.
4. `/v1/payment_intents` create + confirm, form-encoded, with idempotency,
   authenticated. **Updated 2026-09-03 (Step 3): confirm now reaches a rail
   and moves the intent — `processing` on a push rail, `requires_action`
   with a `next_action.redirect_to_url` on a redirect rail, `409
   charge_declined` when the rail refuses, `502` when it cannot be reached.
   Seven integration tests in `confirm_rails.rs` drive all four outcomes
   against real Postgres and WireMock containers. The item is still not met,
   and now for exactly two reasons rather than one:** the rail behind every
   one of those observations is a stub, and **nothing polls the charge
   afterwards** — a confirmed intent stops at `processing` forever, so
   "create + confirm" produces a payment that is never resolved. *The Step 2
   note follows, and its account of where confirm stopped is now history.*
   **(Step 2): create is done; confirm reaches the rail boundary and stops
   there, so this item is not met.**
   `POST /v1/payment_intents` is form-encoded, idempotent, authenticated and
   merchant-scoped, and writes a real row — with `create_then_retrieve_round_trips_through_the_sdk`,
   `a_replayed_idempotency_key_returns_the_same_object_and_no_second_row` and
   `a_post_without_an_idempotency_key_is_the_documented_400` behind it, all
   run against a real Postgres on 2026-09-03. Retrieve, list and cancel are
   there too. **`confirm` is where this item stays open:** it performs the
   crash-safe write ordering and then gets `501 not_implemented` from the
   adapter, so no payment intent has ever reached `processing`,
   `requires_action` or `succeeded`, and no rail has been called. This item
   asks for create **and** confirm; half of it is now real and the other half
   is blocked on item 2. *The paragraph that follows was written when the
   resource half had not started; its account of the credential model still
   holds and is kept for that.* **The *authenticated* half of this item is
   built; the resource half is now built as far as the rail.** As of 2026-09-02 (Step 1) a merchant can
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
