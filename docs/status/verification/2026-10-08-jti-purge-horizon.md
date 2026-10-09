# 2026-10-08 — A spent `jti` is kept past the validator's leeway (ADR-0028)

Branch `claude/skills-companion-adr-review-mxhohm`, on `a33aac6`. Production
code, tests and docs. **No migration, no wire type, no route, no gate, no
`NotImplemented` token moved.** The decision is
[ADR-0028](../../adr/0028-a-spent-jti-outlives-the-validators-leeway.md); this
page is what was run.

## What was wrong

`authkestra-op` 0.7.1 verifies a client assertion with `jsonwebtoken`'s default
60 s leeway, so the API accepts it while `api_now_secs <= exp + 60`: up to 61 s
of real time after `exp`. The worker's hourly `sweep_expired` deleted
`oauth_client_assertion_jtis` rows at `expires_at < now()`, where `expires_at`
is the raw `exp`. A sweep inside that 61 s deleted the `jti` of an assertion
the API still accepted, and the next presentation was a second access token
from one already-spent assertion.

## Reproduced before the fix

The fix was written first, then the production constant
`vpay_worker::CLIENT_ASSERTION_JTI_RETENTION` was set to `Duration::ZERO`
(exactly the old `expires_at < now()`), which is the pre-fix behaviour, and
the new tests were run against it:

| Command                                                                                             | Result with the horizon at zero                                                                                                                                                      |
| --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `cargo test -p vpay-tests-integration --test merchant_token_flow a_spent_assertion` (after reorder) | **1 failed**: the replay of an assertion with `exp` 30 s past, after the real worker sweep, answered **`200`** where `401` is required (`left: 200, right: 401`). The exploit works. |
| the same binary, whole file                                                                         | 10 passed, **2 failed**: the replay case, and `an_assertion_older_than_the_sweep_horizon_is_refused_by_the_validator` (an assertion 1 s past a zero horizon verifies, `200`)         |
| `cargo test -p vpay-tests-integration --test client_store`                                          | 3 passed, **1 failed**: `a_spent_jti_survives_…` deleted 3 rows where 1 is correct                                                                                                   |

The constant was then restored to `Duration::from_secs(5 * 60)`. (An earlier
run of the replay case failed one assertion earlier, on "the spent `jti` row
survived"; the assertions were reordered so the failure shows the replay
itself, which is the property.)

## After the fix

Counts are from this branch, Linux, Docker daemon running,
`postgres:16-alpine`. The shell had `RUST_BACKTRACE=1` set; see the one
failure below.

| Command                                                                                                                       | Result                                                                                                                                                                                                                            |
| ----------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo test -p vpay-tests-integration --test merchant_token_flow --test client_store --no-fail-fast`                          | `client_store`: **4 passed**, 0 failed, 0 ignored. `merchant_token_flow`: **12 passed**, 0 failed, 0 ignored (10 existing + 2 new).                                                                                               |
| `cargo nextest run --workspace --no-fail-fast` (`CARGO_INCREMENTAL=0`, `*_DEBUG=line-tables-only`, `RUST_BACKTRACE=1` in env) | **2100 run: 2099 passed, 1 failed, 0 skipped** (so 0 ignored), 2578 s. Of those: `vpay-db` 249 passed, `vpay-worker` 75, `vpay-api` 403, `vpay-tests-integration` 356 of 357, `vpay-tests-conformance` 67. See the failure below. |
| the failing case alone, `RUST_BACKTRACE=0 cargo nextest run -p vpay-tests-integration -E 'test(the_cstack_schema_drifts_…)'`  | **1 passed**                                                                                                                                                                                                                      |
| `just test-doc` (`cargo test --doc --workspace`)                                                                              | **124 passed, 0 failed, 1 ignored** (`sdks/rust/src/lib.rs - ReadmeDoctests (line 503)`, untouched). `vpay-db`'s own doctest run: 8 passed, 0 ignored.                                                                            |
| `just clippy` (`cargo clippy --workspace --all-targets -- -D warnings`)                                                       | clean. (A first run failed on an `expect` in `client_store.rs`; fixed by returning `anyhow::Result`.)                                                                                                                             |
| `just fmt-check-rust`                                                                                                         | ok                                                                                                                                                                                                                                |
| `just verify`                                                                                                                 | `verify: ok — the fifteen gates above passed`; `verify-doc-counts: 13 documented count(s) in 11 of 278 markdown file(s) agree`; `verify-migrations: 49 migration file(s) … all match`                                             |
| `just verify-ignored`                                                                                                         | `verify-ignored: 0 ignored (expected 0), 48 test binaries (expected 48), 2100 total (minimum 1080)`                                                                                                                               |
| `prettier 3.9.6 --check` on the changed `*.md`                                                                                | ok                                                                                                                                                                                                                                |

The workspace run includes the cases that call the sweep:
`client_store::{expired_client_assertion_jtis_are_swept_and_live_ones_are_kept,
the_sweep_horizon_is_the_callers_duration_on_the_databases_clock,
a_spent_jti_survives_the_sweep_while_its_assertion_could_still_verify}`,
`merchant_token_flow::{a_spent_assertion_past_its_exp_but_inside_the_leeway_cannot_be_replayed_after_a_sweep,
an_assertion_older_than_the_sweep_horizon_is_refused_by_the_validator}`,
`checkout_sessions::the_housekeeping_sweep_expires_a_stale_session_and_spares_a_paying_one`
and `worker_recovery::the_housekeeping_jobs_are_seeded_once_and_reschedule_themselves`,
all passed.

### The one failure was the environment, not this change

`postgres_smoke::the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`
failed in the workspace run, in two stages. The first run had no `cratestack`
CLI on `PATH` (its error says so and names the pinned version; installed with
`cargo +1.98.0 install cratestack-cli --locked --version 0.15.0`). The second
failed with the CLI present: the test compares the CLI's strict-refusal message
exactly, and the message carried an `anyhow` backtrace because the shell had
`RUST_BACKTRACE=1`. With `RUST_BACKTRACE=0` it passes (row above). It does not
touch this change.

## What each test proves

- `merchant_token_flow::a_spent_assertion_past_its_exp_but_inside_the_leeway_cannot_be_replayed_after_a_sweep`
  — an assertion signed by hand with `exp` 30 s in the past is accepted once;
  a control row long past its `exp` is staged; the shipping `seed_singletons`
  and `run_once` run `sweep_expired`; the control row is gone (so the sweep
  ran its `jti` statement), the spent row is still there, and the replay is
  `401 invalid_client`. Decisive: horizon at zero → `200`.
- `merchant_token_flow::an_assertion_older_than_the_sweep_horizon_is_refused_by_the_validator`
  — a control at `exp` − 30 s is accepted (so the refusal below is about age),
  then an assertion one second past `CLIENT_ASSERTION_JTI_RETENTION` is `401`
  and its `jti` was never recorded. This is what fails when an `authkestra-op`
  or `jsonwebtoken` bump widens the leeway past the horizon.
- `client_store::a_spent_jti_survives_the_sweep_while_its_assertion_could_still_verify`
  — on the database's clock (`support::db_now`): rows 30 s and 61 s past `exp`
  survive the production horizon, one a second beyond it is deleted, and a
  survivor is still a spent `jti`.
- `client_store::the_sweep_horizon_is_the_callers_duration_on_the_databases_clock`
  — a 120 s horizon deletes a row 140 s old and keeps one 100 s old; a
  sub-microsecond duration is accepted.
- `client_store::expired_client_assertion_jtis_are_swept_and_live_ones_are_kept`
  — the existing case, updated for the new signature and for `db_now`.

The `merchant_token_flow` harness gained a `pool` field and two helpers
(`assertion_with_exp`, `run_the_housekeeping_sweep`). The tests use
`vpay_worker::CLIENT_ASSERTION_JTI_RETENTION`, never a copy of its value.

## What this does not do

- **It does not run a skewed host.** No test here sets a testcontainer's clock
  apart from this machine's. The skew budget (about four minutes) rests on the
  arithmetic in the ADR and on the horizon being applied by the database, not
  on a measurement.
- **It does not change `record_jti` or `expires_at`**, which stays the client's
  raw `exp` (ADR-0026 D6). It does not edit migration `0011`, whose header
  comment still says no cleanup job exists and is stale; migrations are
  checksummed (`verify-migrations`).
- **It does not resolve ADR-0026 D8's other bullet**
  (`oauth_signing_keys.expires_at`), or move any D6 fact.
- **The confirm gate in `vpay-api/src/v1/return_trip.rs` is unchanged**; the
  ADR records only that ADR-0026 D6's list omitted it.
- **`just ci` was not run as a whole.** `fmt-check`, `clippy`, `verify`,
  `test-rust`, `test-doc` were run as separate recipes, with the results above.
  `lint-web`, `test-web`, `audit-web` and `deny` were not run (no change under
  `frontends/`, `sdks/` or a manifest; there is no `node_modules` and
  `cargo-deny` is not installed here), and nothing in this change touches what
  they check. `just docs-check-citations` was not run (no
  network token); this change cites no new CI run. The `just fmt` prettier half
  was not run tree-wide: `prettier --check` ran on the changed Markdown only.
- **No `vpay-skills` PR was opened.** `ClientAssertions::delete_expired_client_assertion_jtis`
  changed signature and `vpay_worker` gained a public constant; whether a skill
  cites either was not checked, and the instruction for this change was not to
  touch that repository.
- The disk filled twice during the workspace build (`No space left on device`)
  and `target/` was cleaned and rebuilt with `line-tables-only` debug info; the
  first of those runs left no result that is quoted here.
