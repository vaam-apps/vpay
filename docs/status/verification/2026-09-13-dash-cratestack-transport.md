# Verification — mounting CrateStack's transport (nav plan Lane C, 2026-09-13)

Branch `claude/dash-cratestack-transport`, based on lane B's
`claude/dash-admin-role-review` at `f14e31fc5bdfe1ac46d2e5cc97980a48438eb0f7`
(ADR-0018 and `DashboardTenancy`). Brief:
`docs/plans/2026-09-13-dashboard-nav-notes/transport.md` and plan.md's Lane C.

## What changed

- `backends/crates/vpay-db/src/schema.rs` — `dashboard_procedure_router`, a
  `pub(crate)` fn calling `cratestack_schema::axum::procedure_router` (never
  `router()`); the `#[cfg_attr(not(test), allow(dead_code))]` on `mod
search_payment_intents` is deleted.
- `backends/crates/vpay-db/src/dashboard_transport.rs` (new) —
  `DashboardAuthFn` (a plain `Extensions -> Option<String>` closure type) and
  `ExtensionAuthProvider`, the `cratestack::AuthProvider` impl that turns a
  resolved tenant into a `CratestackContext` via `with_principal`, never via
  `persistence::system_context()`.
- `backends/crates/vpay-db/src/repository.rs` — `Repositories` gains
  `dashboard_procedure_router(&self, auth: DashboardAuthFn) -> cratestack::axum::Router`,
  implemented on `PgRepositories` by reusing `self.cs` (the same `Cratestack`
  handle every other repository method already runs its queries through),
  not `op_store_pool` (that accessor's own doc: "not a general escape hatch").
- `backends/crates/vpay-db/Cargo.toml` — `cratestack-codec-json` added as a
  direct, `default-features = false` dependency (already resolved
  transitively via `cratestack-axum`; no new package in `Cargo.lock`) and
  `tower` as a dev-dependency for the routing-table test below.
- `backends/crates/vpay-api/src/lib.rs` — `resolve_dashboard_tenancy`,
  factored out of `require_dashboard_token`'s body (token validate, merchant
  claim, scope, staff re-read, ADR-0018 tenancy resolution) with no change to
  its own behaviour; a new `require_dashboard_procedure_token` middleware
  (`POST`-only) that shares it; `router()` mounts a second, `nest_service`d
  sub-router (`state.repositories.dashboard_procedure_router(...)`, layered
  with the new middleware) at `dash::DASH_NEST`, alongside the existing
  `dash` nest — `nest_service`, not `merge`/`nest`, because CrateStack's
  router is already `Router<()>` while the rest of `router()` is still
  `Router<AppState>` at that point.
- `backends/tests/integration/tests/dashboard_procedure_transport.rs` (new)
  — the end-to-end proof, below.
- `docs/status/backend.md`, `docs/status/cratestack.md`,
  `docs/status/cratestack/2026-09-11-search-payment-intents.md` (corrected in
  place), `docs/status/cratestack/2026-09-13-dashboard-procedure-transport.md`
  (new), `docs/status/README.md`,
  `docs/flows/dashboard/status-styling-and-demo.md` (corrected in place).

## The five "done" criteria

1. **The procedure answers over HTTP with the same rows its existing tests
   assert.** `dashboard_procedure_transport.rs::the_procedure_answers_the_callers_own_merchant_over_http`
   boots the real `vpay_api::router` on a real socket over a real Postgres,
   seeds two merchants' intents, calls `POST
/dash/v1/$procs/searchPaymentIntents` with a real dashboard token, and
   asserts the page is exactly the caller's own two rows, newest first, and
   never the other merchant's — `ok`.
2. **An unauthenticated caller is refused.**
   `dashboard_procedure_transport.rs::an_unauthenticated_caller_is_refused`
   — the same call with no bearer token is `401` — `ok`.
3. **A tenant mismatch is refused.** `vpay-db`'s own mutation, run for this
   lane: `search_payment_intents.rs`'s `SEARCH_SQL` had
   `WHERE merchant_id = $1` replaced with `WHERE $1::TEXT = $1::TEXT` (a
   tautology that keeps the bind-parameter count unchanged), and
   `against_postgres::the_page_is_the_tenants_own_rows_filtered_and_bounded`
   was re-run against a real Postgres container. Output:

   ```
   thread '...the_page_is_the_tenants_own_rows_filtered_and_bounded' panicked at
   backends/crates/vpay-db/src/schema/search_payment_intents.rs:1079:13:
   assertion `left == right` failed: newest first, and merchant_b's row is not merchant_a's business
     left: ["pi_b_only", "pi_a_new", "pi_a_old"]
    right: ["pi_a_new", "pi_a_old"]
   ```

   The mutation was reverted immediately after; the test is green again
   (confirmed by a second run).

4. **No write is reachable.** `schema::tests::no_generated_model_route_is_mounted_only_the_one_procedure_is`
   walks every one of the eighteen models' generated `list`/`create` path
   (`/{plural}`) and `get`/`update`/`delete` path (`/{plural}/{id}`), across
   `GET`/`POST`/`PATCH`/`DELETE` (seventy-two requests), against the router
   `dashboard_procedure_router` actually builds, and asserts each is axum's
   own `404` — no route matched. A seventy-third request, `GET` on the one
   real path, asserts `405` (matched, wrong method) rather than `404`, so the
   test is not merely proving that everything 404s regardless of what is
   mounted. Decisive: temporarily changing the call from
   `cratestack_schema::axum::procedure_router(...)` to
   `cratestack_schema::axum::router(...)` (with `DEFAULT_BODY_LIMIT_BYTES`)
   reddened it at once:

   ```
   thread '...no_generated_model_route_is_mounted_only_the_one_procedure_is' panicked at
   backends/crates/vpay-db/src/schema.rs:359:21:
   assertion `left == right` failed: GET /currencies answered something other than this
   crate's honest 404 — a generated model CRUD route may be mounted
     left: 500
    right: 404
   ```

   (The `500` rather than another status is because the lazy, never-connected
   pool this test uses fails the statement `list_currencies` would run once a
   route admits the request — the point is that it is not `404`.) Reverted
   immediately after; confirmed green again.

5. **`schema.rs`'s `#[cfg_attr(not(test), allow(dead_code))]` on `Payments`
   is deleted.** Done in the same commit as the transport; `docs/status.md`'s
   own machine-checked area (`just verify-docs`) is unaffected (advisory,
   never a gate) — see "verify-docs" below for the exact count.

## Executed and ignored counts

- `cargo test -p vpay-db --lib` (container tests included, `DOCKER_HOST` set)
  — **81 passed, 0 failed, 0 ignored.**
- `cargo test -p vpay-api --lib` — **366 passed, 0 failed, 0 ignored** (all
  pre-existing; `resolve_dashboard_tenancy`'s extraction changed no
  behaviour).
- `cargo test -p vpay-tests-integration --test dashboard_procedure_transport`
  — **2 passed, 0 failed, 0 ignored** (new).
- `cargo test -p vpay-tests-integration --test dashboard_read_surface` — **21
  passed, 0 failed, 0 ignored** (pre-existing, unchanged — including the
  ADR-0018 admin/cross-tenant cases, which is what proves the
  `resolve_dashboard_tenancy` extraction is behaviour-preserving).
- Container tests initially failed with a plain connection error before
  `DOCKER_HOST=unix:///run/user/1000/docker.sock` was set (this host's
  rootless Docker) — a skipped/errored container test was never reported as
  passing; both suites were re-run to completion once the socket was
  correct.

## Gates

`just verify` (twelve gates) failed once on `verify-links`, because two new
doc pages and one new page reference were still untracked
(`cargo xtask verify-links` reads `git ls-files`, not the filesystem) —
`git add -A` fixed it; not a defect in the pages themselves.

`just verify` exit code: **0**, after staging (`verify: ok — the twelve gates
above passed; the verify-docs report is advisory`). One environment note,
not a regression this lane caused: `check-schema` printed `WARNING —
cratestack 0.11.1 on PATH, this repository pins 0.12.0` and ran the check
under 0.11.1 regardless — `schemas/vpay.cstack` still type-checked. The
library the workspace actually compiles against (`Cargo.lock`) is `0.12.0`,
as `root Cargo.toml` pins.

`just verify-docs`'s advisory count of `#[allow]`/`#[expect]` sites in
production code is **6** after this change (it was **7** on the base commit —
`schema.rs`'s deleted `#[cfg_attr(not(test), allow(dead_code))]` on `mod
search_payment_intents` was one of them). `verify-repositories` reports
**4** concrete implementations in `vpay-db` (`Handles`, `PendingTransaction`,
`PgRepositories`, `SqlClientAssertionStore`) named by none of 83 source files
outside it, and confirms no generated schema module is exported — the
gate this lane was most likely to trip, and it passed cleanly.

`cargo clippy --workspace --all-targets -- -D warnings` (the same recipe
`just clippy` runs) over `vpay-db`, `vpay-api` and `vpay-tests-integration`
after every change in this lane: clean. One warning was found and fixed
along the way — `clippy::result_large_err` on `resolve_dashboard_tenancy`'s
`Result<_, Response>`, resolved by boxing the error
(`Result<_, Box<Response>>`) rather than by an `#[allow]`.

`cargo fmt --check` over the same three crates: clean after one `cargo fmt`
pass (import ordering and line wrapping the tool applies automatically;
no hand-formatting was fought).

`just ci`'s exit code, read from a file rather than the harness banner: see
this session's final report for the number — the recipe takes over an hour
and this page was drafted from the gate outputs gathered while it was still
running its slow (container-backed) legs, per this lane's own brief.
