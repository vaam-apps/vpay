# exp12b (opus arm) — sqlx 0.9 **landed**, by replacing the OP stores that pinned 0.8

Date: 2026-09-05. Branch `claude/exp12b-sqlx09-opus`, base `d086084`
(master: Rust 1.98.0, ten gates, 1270 tests in 42 binaries, 0 ignored).
**Rebased onto `8d907f9` the same day** — PR #44's CrateStack drift test
landed on `master` in between — and every count below that names `d086084` is
superseded by §11, which re-measures on the new base rather than adjusting the
old numbers.
Host: the authoring machine, rootless Docker
(`DOCKER_HOST=unix:///run/user/1000/docker.sock`), `CARGO_BUILD_JOBS=4`,
Node 22.23.2 via nvm, other agents running container tests concurrently.

Every command in this file was run here and every figure is pasted, not
recalled. The sequel to sample 12 (`docs/plans/exp12-notes/opus.md` on branch
`claude/exp12-sqlx09-opus`), which measured this upgrade and deliberately did
not land it because completing it needed a decision.

## 0. The answer, up front

**Landed.** `[workspace.dependencies] sqlx = "0.9"`, one sqlx major in the
graph, ten gates green, `just ci` green end to end, and the image builds and
runs.

The blocker sample 12 identified was `authkestra_op::sqlx_store::SqlxOpStore`,
reachable only through `authkestra-op`'s optional `sqlx-postgres` feature,
which pins `sqlx ^0.8`. vpay used it for three `CompositeOpStore` slots that
no `/v1` grant can reach. Those slots now hold fail-closed stores written
here; the feature is off in both manifests; the last reverse dependency of
sqlx 0.8 is gone.

## 1. The decision, and who took it

Taken by the coordinator under the maintainer's "upgrade the repo" direction,
and stated rather than re-opened: **replace the three `SqlxOpStore` slots with
fail-closed stores**, in the pattern `authkestra-op` uses for its own optional
seams. Not option B of sample 12 (vpay implements OP storage — 900-1400 lines
of SQL for grants nothing serves, three of whose methods are an
authorization-code replay if written wrongly), and not option C (two sqlx
majors, which fails the one property the upgrade exists for).

**First I checked whether authkestra already ships such types**, as the brief
required, by extracting the crate rather than reading a changelog:

```
$ curl -sL https://static.crates.io/crates/authkestra-op/authkestra-op-0.7.1.crate | tar xz
$ grep -rn "No[A-Za-z]*Store" authkestra-op-0.7.1/src/lib.rs
28:    ClientAssertionStore, MemoryClientAssertionStore, NoClientAssertionStore,
43: pub use dpop::{DpopJtiRecord, DpopReplayStore, NoDpopReplayStore, …};
$ grep -rn "impl.*\(AuthorizationCodeStore\|RefreshTokenStore\|DeviceCodeStore\) for" authkestra-op-0.7.1/src
src/code.rs:116        impl<S> AuthorizationCodeStore for S where S: KvStore + AtomicConsume
src/refresh.rs:87      impl<S> RefreshTokenStore for S      where S: KvStore + AtomicConsume
src/device.rs:101      impl<S> DeviceCodeStore for S        where S: IndexedKvStore + AtomicConsume
src/sqlx_store.rs:277,317,414   (the three for SqlxOpStore)
src/builder.rs, src/handlers/token.rs           (the crate's own test doubles)
```

It ships fail-closed types for the **client-assertion** and **DPoP** seams and
for **neither** of the three this task needs. So they are written here, and
`NoClientAssertionStore`/`NoDpopReplayStore` are the pattern they follow.

(A note for whoever reads `code.rs:116` and worries about coherence: the
blanket impl is over `S: KvStore<..> + AtomicConsume<..>`, and a *local* unit
struct that implements neither is a disjoint impl rustc accepts. This was the
one thing I expected to fight and did not — `cargo check` produced exactly one
error on first compile, an arity mismatch on `RefreshToken::new`, which gained
a `jkt` parameter at 0.7.1.)

## 2. What was written

`backends/crates/vpay-api/src/op/refusing_stores.rs`:

* `UnservedGrant` — a three-variant enum whose `grant_type()` returns the wire
  spelling. An enum, not three `&'static str`s, so a fourth grant is a compile
  error at every match rather than a fourth string.
* `UnservedGrantError` — `#[derive(thiserror::Error)]`, `Display` names the
  grant *and* the trait method (`store_code` reaching this is a different
  defect from `consume_code` reaching it), `impl Classify` →
  `Category::NotImplemented`. Not `Internal`: `Internal` claims vpay broke,
  and what happened is that a capability vpay never offered was asked for.
  `public_message()` is the category's generic sentence and leaks neither the
  grant nor the method (ADR-0011), which is its own test.
* `RefusingAuthorizationCodeStore`, `RefusingRefreshTokenStore`,
  `RefusingDeviceCodeStore` — twelve async methods, no SQL, each returning
  `Err(OpError::GrantTypeNotPermitted)` after a `tracing::error!` carrying the
  whole `UnservedGrantError`. `GrantTypeNotPermitted` rather than `Storage`
  because nothing is wrong with any storage and a log reader who sees "storage
  error" starts checking Postgres; it makes no difference to the caller, since
  every grant handler maps *any* store error to `server_error` without
  inspecting it.
* Four doctests and four unit tests.

**Why this is not the stub AGENTS.md rule 1 forbids**, spelled out because it
is the one judgement call in the change: an "always empty" store answers
`Ok(None)`, `authkestra_op` renders that as `invalid_grant` — "your code was
wrong" — and the day another grant is mounted it becomes a silent lie. These
answer `Err` from all twelve methods, so that day fails loudly with a message
naming the grant. `verify-no-mocks` is unchanged and green.

## 3. Why nothing can reach them, measured on both sides of the change

Two independent arguments, then the measurement.

* Each of the three grant handlers checks `client.allows_grant_type(..)` as
  its **first statement** (`default_handle_authorization_code`,
  `default_handle_refresh_token`, `handle_device_code`, all in
  `authkestra-op-0.7.1/src/handlers/token.rs`), and a merchant registration
  can only ever declare `client_credentials`
  (`vpay_config::ConfigError::DisallowedMerchantGrant`).
* `handle_client_credentials` — the one handler that does run — **is not
  passed the store at all**: its signature takes `req, client_id, client,
  config, tokens, client_cert_der` and no `op_store`. No configuration change
  can route the served grant to a refusing store.

The test is `merchant_token_flow.rs` case (i),
`the_three_grants_vpay_does_not_serve_are_refused_before_any_store`: a real
router on a real socket over a real Postgres, a real freshly minted
`private_key_jwt` assertion per grant, every parameter the grant's own handler
would need (so a refusal cannot be "you left out `code`"), asserting **not
500, not `server_error`, and exactly `unauthorized_client`/400**.

**The baseline was measured first, on the unmodified tree**, with
`SqlxOpStore` still wired — because "the same refusal shape as on master" is
only worth asserting if somebody looked:

```
$ git stash push --include-untracked      # my work shelved; tree == d086084
$ cargo nextest run -p vpay-tests-integration --test merchant_token_flow \
    -E 'test(the_three_grants…)'
    Summary [  14.076s] 1 test run: 1 passed, 9 skipped
```

and after the swap, in the full file:

```
    Summary [  29.302s] 10 tests run: 10 passed, 0 skipped
```

**The brief predicted `unsupported_grant_type` and the measurement says
`unauthorized_client`.** The test asserts what was measured. The reason is the
`allows_grant_type` check above: authkestra dispatches on the grant string
first and consults `grant_types_supported` never, so a grant it *knows* but
this client may not use is `unauthorized_client`, not `unsupported_grant_type`.

## 4. The sqlx bump

`sqlx = "0.9"` in `[workspace.dependencies]`, `cargo update -p sqlx`, and:

```
$ cargo metadata | (every package whose name starts with "sqlx")
sqlx 0.9.0  sqlx-core 0.9.0  sqlx-macros 0.9.0  sqlx-macros-core 0.9.0
sqlx-mysql 0.9.0  sqlx-postgres 0.9.0  sqlx-sqlite 0.9.0
$ cargo tree -i aws-lc-rs      → did not match any packages
$ cargo tree -i aws-lc-sys     → did not match any packages
$ cargo tree -i openssl-sys    → did not match any packages
$ cargo tree -i native-tls     → did not match any packages
$ cargo deny check
advisories ok, bans ok, licenses ok, sources ok
```

`cargo tree -d` lists no sqlx *version* duplicate. It does print
`sqlx-core v0.9.0 (*)` twice — the same version twice, which it also does for
`base64 v0.22.1` and `log v0.4.33` on this workspace; that is a
feature-resolution artefact of `-d` on a multi-member workspace, not two
majors.

The 36 `AssertSqlSafe` sites are exactly sample 12's table, re-derived from a
fresh `cargo check` on this branch and re-audited site by site here rather
than taken on trust. **Two things sample 12 could not have found**, because
`vpay-api` did not compile there and so neither did the integration crate:

* `postgres_smoke.rs` has two more dynamic statements (`{table}`,
  `{expires_at_clause}`) — an identifier and a SQL expression, neither of
  which can be a bind parameter. Wrapped, each with its own audit comment.
* `payment_intents.rs`'s `count(pool, sql, bind)` helper took `sql: &str`,
  which no longer satisfies `SqlSafeStr`. Every caller passes a literal, so
  the parameter became `&'static str` — the compiler keeps doing the checking
  rather than the check moving into a comment. This is the one place the bump
  made the code *stronger* rather than merely different.

Feature list copied across unchanged (checked against both releases' own
manifests); `webpki-roots` 0.26.11 → 1.0.9, still CDLA-Permissive-2.0, so
`deny.toml` is untouched. No `query!` macros and no `.sqlx/`, so
`cargo sqlx prepare` is not applicable rather than skipped.

**Two claims about sqlx internals were re-read at 0.9.0** rather than assumed
to carry over, because a major bump is exactly what invalidates them:
`sqlx-core`'s TLS handshake still hands `ring::default_provider()` to
`builder_with_provider` and never calls `CryptoProvider::get_default()`; and
`PoolOptions` still exposes `acquire_timeout` as its only connect-path
timeout. Both doc comments now say they were re-checked and when.

**`rust-version` moved 1.88 → 1.94.** sqlx 0.9.0 declares `1.94.0`; 0.8.6
declared none. Re-derived over all 469 packages (112 declare nothing). The
toolchain pin (1.98.0) is unaffected.

## 5. The audit is a test

`AssertSqlSafe`'s contract is "the caller audited this string", and a contract
discharged by a comment is discharged by whoever last read the comment. So the
invariant is enforced: `vpay_db::sql_audit` (test-only) reads the crate's own
sources and fails if a `format!` bound to `sql` interpolates anything but a
`const … : &str` declared in the crate, or one of two exceptions that are
themselves checked (`direction` must still be the two-literal `if`; `columns`
must still be `crate::charges::COLUMNS`).

**Proven to fire by three mutations, each reverted:**

| mutation | failing test |
|---|---|
| `format!("… WHERE payment_intent_id = '{payment_intent_id}'")` in `charges::get_for_intent` | `every_interpolation_into_a_statement_is_a_crate_constant` — *"charges.rs: a statement interpolates `{payment_intent_id}`, which is neither a `const …: &str` in this crate nor one of the two audited exceptions"* |
| `let direction = if backwards { "ASC" } else { "DESC".to_owned().leak() };` | `the_audited_non_constants_are_still_what_the_audit_says_they_are` — *"events.rs interpolates `{direction}` but no longer contains …"* |
| `AssertSqlSafe(format!("UPDATE jobs SET locked_by = '{worker_id}'"))` in `jobs::claim` | `every_assert_sql_safe_wraps_the_variable_the_audit_covers` |

The third is the one that matters: without it, the audit is bypassed by not
using the variable the audit looks at. Two further tests are controls on the
scanners themselves (balanced parens through `count(*)`, `{{}}` as an escape
rather than a capture), driven over synthetic text so they cannot be satisfied
by the real sources happening to be clean.

## 6. The decisive negative, re-measured here

A scratch crate outside the workspace on `cratestack-sqlx = "0.11.1"` **and**
`vpay-db` by path. Both directions, on this branch, minutes apart:

```
# workspace pin 0.8                    # workspace pin 0.9
sqlx 0.8.6                             sqlx 0.9.0
sqlx-core 0.8.6                        sqlx-core 0.9.0
sqlx-core 0.9.0     <-- two majors     sqlx-macros 0.9.0
sqlx-macros 0.8.6                      sqlx-macros-core 0.9.0
sqlx-macros-core 0.8.6                 sqlx-mysql 0.9.0
sqlx-mysql 0.8.6                       sqlx-postgres 0.9.0
sqlx-postgres 0.8.6                    sqlx-sqlite 0.9.0
sqlx-postgres 0.9.0 <-- two majors
sqlx-sqlite 0.8.6                      $ cargo check
                                           Checking cratestack-sqlx v0.11.1
                                           Checking vpay-db v0.1.0
                                           Finished in 56.14s
```

The scratch crate was deleted. **No CrateStack crate was added to this
workspace**, and `schemas/vpay.cstack` is still outside the build graph.

## 7. What was given up, and the honest cost

* **`backends/tests/integration/tests/authkestra_op_smoke.rs` (3 tests) is
  deleted.** It drove `SqlxOpStore`'s hand-built SQL against migrations
  0006/0013 to prove that transcription faithful, and it cannot compile
  without the feature. It proved a property of a type this system no longer
  constructs. `postgres_smoke.rs` still asserts all four `authkestra.*` tables
  exist and that `oauth_codes.client_id`'s foreign key fires — a schema check,
  not a store-compatibility check, and the difference is recorded in
  `docs/status.md` against the row that used to claim the stronger thing.
* **The four `authkestra.*` tables are now unread and unwritten by any code
  path.** Dropping them needs a new migration and is left to the maintainer.
* **Migrations 0006 and 0013 still cite the deleted test in their header
  comments, and were deliberately left wrong.** `sqlx::migrate!` checksums a
  migration's entire file content, comments included, so editing an applied
  one turns the next boot into a version mismatch. The correction is in
  `docs/status.md`.
* **`expected_suites` moved 42 → 41.** This is the first entry in that
  justfile comment's long history to record a binary being *removed*, and the
  reasoning is written there rather than only here.

## 8. Reserved for the maintainer

* **Ask authkestra upstream to move `authkestra-store-sqlx` to sqlx 0.9.** An
  external issue or PR for the maintainer to file, not for an agent. It is
  what would make a real SQL-backed store available again if `/v1` ever mounts
  one of the three grants. (`authkestra-op` 0.8.1, published 2026-09-05,
  deletes `src/sqlx_store.rs` and moves it to `authkestra-store-sqlx`, which
  is still `sqlx ^0.8` — so bumping the authkestra family does not help.)
* **Whether to drop the four `authkestra.*` tables**, now that nothing reads
  them.
* **Whether to adopt `cratestack-sqlx` at all**, now that it resolves. This
  pass made it possible and started nothing.

## 9. The gate

```
$ just ci                                                     # exit 0
verify-no-mocks: ok — no test double reachable from a shipping binary
verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md …
verify-errors: ok — 16 error type(s), all classified; 14 `#[from]` variant(s) …
verify-sdk-parity: ok — 342 proving test(s) …, 26 dated gap(s)
verify-links: ok — 715 repository link(s) in 126 tracked markdown file(s) …
verify-npm-scope: ok — 2 publishable package(s) …
check-schema: ok — schemas/vpay.cstack type-checks under cratestack 0.11.1
verify-serde: ok — 49 serialisable type(s) …
verify-repositories: ok — 3 concrete implementation(s) in vpay-db …
verify-toolchain: ok — rust-toolchain.toml pins 1.98.0 …
verify: ok — the ten gates above passed; the verify-docs report is advisory
    Starting 1277 tests across 41 binaries
     Summary [1029.968s] 1277 tests run: 1277 passed, 0 skipped
verify-ignored: 0 ignored (expected 0), 41 test binaries (expected 41), 1277 total (minimum 1080)
advisories ok, bans ok, licenses ok, sources ok
```

`verify-errors` reads 16 error types where master reads 15 — the new one is
`UnservedGrantError`. Doctests: **90 passed, 0 failed, 1 ignored** (the
ignored one is `sdks/rust`'s README block, pre-existing). Web: vitest across
8 packages — checkout 302, nodejs SDK 172, stripe-js 119, config 63, shop 57,
api-client 4, tokens 3, ui 3.

By name, from that run: `merchant_token_flow` **10/10** including case (i);
`postgres_smoke` **15/15** including `one_charge_per_intent_is_enforced_by_
the_database`, `an_authkestra_oauth_code_referencing_a_nonexistent_client_is_
rejected_by_the_database` and `schema_migrates_cleanly_on_an_empty_database`;
`worker_kill9` **2/2**; `worker_recovery` **23/23**; the `jobs` claim cases
and the idempotency cases in `vpay-db::repositories` all green (that suite is
**91/91** on its own).

**An earlier attempt at this gate failed, and it is recorded rather than
dropped.** `vpay-db::postgres an_abandoned_transaction_survives_a_rollback_it_
cannot_send` hit `failed to create a container: Timeout error` after 120 s.
At that moment the host had 35 containers up and a load average of 28, and a
bare `docker run postgres:16-alpine` took **19.5 s** to create. nextest
cancels on the first failure, so **no container test ran at all** in that
run — it is evidence about the host, not about the code. `cargo nextest run
-p vpay-db` once the load dropped: **91 tests run: 91 passed, 0 skipped**,
that case included. Then the full `just ci` above.

Image: `docker buildx build -f backends/Dockerfile --target server .` on a
private builder `vpay-exp12b-opus` (removed afterwards; the shared default
builder was never touched or pruned) — **exit 0**, `docker run --rm … --version`
prints `vpay-server 0.1.0`, image **16 MB** (`FROM scratch`, musl static).

## 10. What I did not do

* **Did not implement real OP storage.** The three stores refuse; they store
  nothing. If `/v1` ever mounts one of those grants, this is where the work
  starts.
* **Did not bump the authkestra family** to 0.8.1. It does not unblock
  anything (§8) and is a second dependency migration with its own breaking
  surface.
* **Did not drop the four unused `authkestra.*` tables**, or touch any
  migration file.
* **Did not adopt CrateStack.** The scratch crate that proves the bump unlocks
  it was deleted; no CrateStack crate is in this workspace.
* **Did not verify the 1.94 MSRV by compiling** with a 1.94 toolchain. It is a
  metadata floor, as the comment in `Cargo.toml` says.
* **Did not run `just test-e2e`, `just helm-check` or `just demo`.** None is
  part of `just ci`; the first needs Cypress's CDN and the second needs the
  network.
* **Did not push, and did not open a PR.**
## 11. Rebased onto `8d907f9` (2026-09-05)

`master` moved while this branch sat: [PR
#44](https://github.com/vaam-apps/vpay/pull/44) merged the exp13 drift work —
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` in
`backends/tests/integration/tests/postgres_smoke.rs`, a CI step installing the
pinned CrateStack CLI in the `rust` job, the `docs/status.md` "The measured
drift" section, a `schemas/vpay.cstack` header and a `justfile` `check-schema`
comment. `git rebase origin/master` replayed all ten commits of this branch
with **one conflict**.

**`justfile` — the only conflict, resolved by keeping both sides.** Both
branches appended a dated entry to the comment block above
`expected_ignored`/`expected_suites`: #44's recording *1271 total, 42 test
binaries*, and this branch's recording *42 → 41* when
`authkestra_op_smoke.rs` was deleted. Neither block was dropped — each names
the base it was measured on, and deleting either would have erased a measured
fact to make an arithmetic chain look tidy. `expected_suites` is **41**: #44's
case joined `postgres_smoke`, a binary that already existed, so it added a
test and not a suite, while this branch removed a whole binary. Verified
rather than reasoned — `cargo nextest list --workspace` on the rebased tree
prints **1279 total, 41 test binaries, 0 ignored** — and a third entry was
added to that comment saying so, because the 1272/1277/1278 in the entries
above it were all measured on `d086084` and none of them had seen #44.

**No other file conflicted, which is the part that needed checking rather than
trusting.** `postgres_smoke.rs` was edited by both branches and git merged it
silently: #44 split `migrated_postgres` into a delegating wrapper over a new
`migrated_postgres_with_url` and appended ~530 lines of drift test at the end;
this branch edited the table list, the `AssertSqlSafe` wrapping and
`insert_signing_key` in the middle. Both sides were re-read in full against
their own diffs rather than accepted because the merge was quiet. Intact on
the rebased tree: #44's `migrated_postgres_with_url`, `OutDir`, `repo_root`,
`pinned_cratestack_version`, `parse_drift_header`,
`tables_missing_from_the_schema` and the drift test itself; this branch's
`authkestra.oauth_dpop_jti` row, its three migration-0013 column assertions
and both of its `sqlx::AssertSqlSafe(format!(…))` wrappings. Nothing was
half-merged and no brace went missing — `cargo build --workspace
--all-targets` is the check that would have caught it, and it was run before
anything else. `docs/status.md` merged cleanly too (this branch's 343-line
insertion sits ~130 lines above #44's edits), and both sets of rows are
present.

**The gate, end to end, on the rebased tree.**

```
verify: ok — the ten gates above passed; the verify-docs report is advisory
    Starting 1279 tests across 41 binaries
     Summary [ 676.578s] 1279 tests run: 1279 passed, 0 skipped
verify-ignored: 0 ignored (expected 0), 41 test binaries (expected 41), 1279 total (minimum 1080)
advisories ok, bans ok, licenses ok, sources ok
JUST_CI_EXIT=0
```

Ten gates by name: `verify-no-mocks`, `verify-status` (1 unimplemented item,
declared), `verify-errors` (16 error types), `verify-sdk-parity` (342 proving
tests, 26 dated gaps), `verify-links` (718 links in 130 files),
`verify-npm-scope`, `check-schema` (cratestack 0.11.1, 12 declarations),
`verify-serde`, `verify-repositories`, `verify-toolchain` (1.98.0 in both
places). `just test-doc`: **90 passed, 0 failed, 1 ignored** — unchanged, as
expected: #44 added no doctest. Web: vitest across 8 packages — checkout 302,
nodejs SDK 172, stripe-js 119, config 63, shop 57, api-client 4, tokens 3,
ui 3. By name from that run: `merchant_token_flow` **10/10** including
`the_three_grants_vpay_does_not_serve_are_refused_before_any_store`;
`vpay-api op::refusing_stores` **4/4**; `vpay-db sql_audit` **6/6**;
`postgres_smoke` **16/16** including
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` (1.893 s,
so the CLI really ran — that test fails rather than skips when `cratestack` is
off `PATH`); `worker_kill9` **2/2** (35.2 s and 6.0 s).

**Both decisive mutations re-run on the rebased tree**, because a rebase is
exactly when a gate quietly stops gating:

| # | Mutation | Observed |
|---|---|---|
| M4 | `authkestra-op = { workspace = true, features = ["sqlx-postgres"] }` in `vpay-api` | `cargo tree -d`: `sqlx v0.8.6` beside `sqlx v0.9.0`. `cargo deny check bans`: `error[banned]: crate 'sqlx = 0.8.6' is explicitly banned`, `error[banned]: crate 'sqlx-core = 0.8.6' …`, **bans FAILED, exit 2** |
| M1 | `charges::get_for_intent` interpolates `payment_intent_id` as a positional `{}` with the bind removed | **FAIL** `sql_audit::tests::every_interpolation_into_a_statement_is_a_crate_constant` — "charges.rs: a statement uses a positional `{}` capture, whose value comes from the argument list and cannot be checked here" |

Both reverted (`git checkout --` on the mutated files plus a `cargo metadata`
to restore `Cargo.lock`); `cargo deny check bans` is **bans ok** and
`sql_audit` is **6 passed** again, and `git status` shows only the
documentation edits this section belongs to.

**Image.** `docker buildx build -f backends/Dockerfile --target server .` on a
private builder `vpay-exp12b-land` (created for this, removed afterwards; the
shared default builder was never touched or pruned): **exit 0**, `docker run
--rm … --version` prints `vpay-server 0.1.0`, image **16 MB**.

**Nothing in §10 changed.** The rebase added no functionality: there is still
no real OP storage, the 1.94 MSRV has still never been compiled, and
`authkestra` upstream has still not been asked to move
`authkestra-store-sqlx` to sqlx 0.9.
