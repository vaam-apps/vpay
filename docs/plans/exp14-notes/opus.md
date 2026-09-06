# exp14 (opus) — the first CrateStack read

Branch `claude/exp14-cratestack-read-opus`, base `65a5952` (master).
Design: the data-layer plan's §0–§2 and §4 "S1". This page is the
measurement record; `docs/status.md` § "The first CrateStack read" and
`docs/reference/vpay-db.md` § CrateStack carry the conclusions.

Everything below was run on 2026-09-06 with `cratestack 0.11.1` on `PATH`,
rustc `1.98.0` (the pin), and `postgres:16-alpine` via testcontainers.

---

## 1. The dependency, and what the graph gained

`cratestack = { package = "cratestack-pg", version = "=0.11.1",
default-features = false, features = ["postgres"] }`.

**Why that feature set**, read out of `cratestack-pg-0.11.1/Cargo.toml`
rather than chosen: `default = ["postgres", "decimal-rust-decimal",
"codec-json"]`.

| Feature | Taken? | Why |
|---|---|---|
| `postgres` | yes | `dep:cratestack-sqlx` + `cratestack-macros/postgres` — the entire data layer. `include_server_schema!(…, db = Postgres)` has no backend without it. |
| `decimal-rust-decimal` | **no** | Exists to make `cratestack_core::Decimal` and the `sqlx-core/rust_decimal` bridge exist. `schemas/vpay.cstack` declares no `Decimal` field — vpay's money is integer minor units. |
| `codec-json` | **no** | `cratestack-client-rust/codec-json`, the *generated client's* JSON codec. vpay generates no client. |
| `crypto-aws-lc-rs` | **never** | `deny.toml` bans `aws-lc-rs`; and at 0.11.1 the feature is a `compile_error!`, not a working mode (`cratestack-pg-0.11.1/src/lib.rs:199-215`). |
| `pgvector`, `postgis`, `rate_limit`, `decimal-bigdecimal` | no | Nothing here uses them. |

**Graph delta, measured.** `Cargo.lock`: **469 → 497 packages (+28)**, plus
one version bump (`syn 3.0.3 → 3.0.5`). Cargo's own resolve line:
`Locking 29 packages to latest Rust 1.94 compatible versions`.

**Corrected 2026-09-06 by review:** those 28 are Cargo.lock *entries*, and
only **25 are new crate names**. The other three — `const-oid` 0.9.6,
`foldhash` 0.2.0 and `hashbrown` 0.15.5 — are additional *versions* of crates
the graph already carried, which is why they appear both in the table below
and in the duplicate-majors sentence after it. `comm -13` over the two
lockfiles' unique name sets is the measurement.

| Crate | Licence |
|---|---|
| `cratestack-{pg,axum,client-rust,codec-cbor,codec-json,core,exec,macros,parser,policy,sql,sqlx}` (12) | MIT |
| `ariadne`, `chumsky` | MIT |
| `const-oid`, `erased-serde`, `hashbrown`, `object`, `psm`, `stacker`, `typeid`, `unicode-segmentation`, `unicode-width`, `wasm-streams` | MIT OR Apache-2.0 |
| `ar_archive_writer` | Apache-2.0 WITH LLVM-exception |
| `foldhash` | Zlib |
| **`minicbor` 2.3.0, `minicbor-serde` 0.7.1** | **BlueOak-1.0.0** |

`cargo tree -i aws-lc-rs` → `error: package ID specification aws-lc-rs did
not match any packages` (still empty). `cargo tree -i aws-lc-sys` the same.
`cargo tree -d --depth 0`: 77 → 82 entries, and **no sqlx duplication** —
`sqlx-core` appears only at `0.9.0` (it was already listed twice at that same
version *before* this change, i.e. two feature-resolved units, not two
versions; measured by running the same command on the reverted tree). The
genuinely new version duplicates are `const-oid` (0.9.6 / 0.10.2),
`foldhash` (0.1.5 / 0.2.0) and — **added 2026-09-06 by review, the sentence
missed it** — `hashbrown`, which goes from three versions to four
(0.12.3 / 0.16.1 / 0.17.1, plus 0.15.5). Re-derived by listing every crate
with more than one version in each lockfile and diffing the two lists, rather
than by reading `cargo deny`'s warnings, which is how one of three came to be
left out. `multiple-versions = "warn"`, and `cargo deny check bans` is green;
`cargo deny check` reports 16 duplicate warnings in total.

**MSRV floor 1.94 → 1.98.** `cargo metadata … | jq '[.packages[].rust_version
| select(.!=null)] | max'` returns `1.98.0`; every `cratestack-*` package
declares it, and they displace `sqlx-*` (1.94.0) as the maximum. Cargo says
so on the resolve too (`Adding cratestack-core v0.11.1 (requires Rust
1.98.0)`). `Cargo.toml`'s `rust-version` moved with it, with the derivation
comment updated.

**`sqlx` re-pinned `"0.9"` → `"=0.9.0"`.** `cratestack-sqlx-0.11.1` declares
`sqlx-core = "=0.9.0"` / `sqlx-postgres = "=0.9.0"` and takes neither through
the `sqlx` umbrella. `run_in_tx` accepts vpay's `Transaction<'_, Postgres>`
only while both halves resolve the same `sqlx-core`; a caret pin lets 0.9.1
break that as an inscrutable trait error. The brief called the base
`=0.9.0`; it was `"0.9"`, so this is a real change rather than a no-op.

---

## 2. Decisive test 1 — the Blue Oak exception is load-bearing

Run **before** writing the exception, i.e. with the dependency in and
`exceptions = []`:

```
$ cargo deny check licenses
error[rejected]: failed to satisfy license requirements
   ┌─ …/minicbor-2.3.0/Cargo.toml:33:12
   │
33 │ license = "BlueOak-1.0.0"
   │            ━━━━━━━━━━━━━ rejected: license is not explicitly allowed
error[rejected]: failed to satisfy license requirements
   ┌─ …/minicbor-serde-0.7.1/Cargo.toml:33:12
licenses FAILED
```

Exactly **two** rejections, both Blue Oak, and **no other new licence or ban
from the CrateStack graph** — checked by grepping every `┌─` in the report
(two crate manifests, plus two pre-existing `license-not-encountered`
warnings for `CC0-1.0`/`MPL-2.0` that are unrelated). With the scoped
exception in place:

```
$ cargo deny check
advisories ok, bans ok, licenses ok, sources ok
```

The exception names the two crates and never touches the `allow` list, so a
third Blue Oak crate would fail this gate rather than arrive silently.

**It cannot be engineered away.** `cratestack-pg`'s manifest declares
`[dependencies.cratestack-axum]` and `[dependencies.cratestack-client-rust]`
with no `optional`, and both take `minicbor` unconditionally
(`cratestack-client-rust` directly + through `cratestack-codec-cbor`;
`cratestack-axum` the same). `cargo deny`'s own inverted tree shows both
paths. Taking `cratestack-sqlx` + `cratestack-macros` directly instead is not
a way out: `compose_server_schema` calls `axum_module::build_axum_module`
unconditionally and the emitted module opens `use ::cratestack::HttpTransport;`
(`cratestack-macros-0.11.1/src/include/server.rs`,
`.../server/axum_module.rs`).

---

## 3. The schema, and the drift number

`model DisabledClient` with three fields and `@@allow("read",
auth().isSystem())` and nothing else. `cratestack_min_declarations` 12 → 13.

**`@default(now())` versus `@default(dbgenerated())` — both were run.** This
is the §6 item the design flagged as unverified, and it decides whether the
drift count moves at all:

| Spelling | Report line for `disabled_clients` | Header |
|---|---|---|
| `@default(dbgenerated())` | `[safe] column disabled_at default value differs from the schema` | `drift detected in 17 table(s)/view(s) (86 change(s) total)` |
| `@default(now())` | *(the table is absent from the report)* | `drift detected in 16 table(s)/view(s) (85 change(s) total)` |

`migrate baseline` reads the live default through `parse_default("now()")` →
`ColumnDefault::Function("now()")`; `@default(now())` converts to the same
thing and compares equal, `@default(dbgenerated())` converts to
`ColumnDefault::DbGenerated` and never does. **With `dbgenerated()` the total
stays at exactly 86** — one line swapped for another, a whole table entering
the schema invisible to the count. That is the mutation `exp13` predicted and
the exact-set assertion is what catches it; the first run of the drift test on
this branch failed on precisely that assertion:

```
assertion `left == right` failed: the set of tables the migrations build and the schema does not declare
  left:  [… "checkout_sessions", "events", …]
  right: [… "checkout_sessions", "disabled_clients", "events", …]
```

`@default(now())` was chosen, so the constants moved:
`EXPECTED_DRIFT_CHANGES` 86 → **85**, `EXPECTED_DRIFTED_RELATIONS` 17 → **16**,
the table set loses `disabled_clients`, and `EXPECTED_UNMAPPABLE_COLUMNS`
stays **18**. All in the same commit as the schema change.

```
$ cargo nextest run -p vpay-tests-integration --test postgres_smoke the_cstack_schema_drifts
    PASS [   1.376s] (1/1) …::the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount
```

`just check-schema` is green throughout (`13 model/enum declarations,
datasource present`, `schema OK`).

---

## 4. The Rust side

`mod schema;` (private) holds
`::cratestack::include_server_schema!("../../../schemas/vpay.cstack", db = Postgres);`.
The path is resolved against `CARGO_MANIFEST_DIR`
(`cratestack-macros-0.11.1/src/include/parse.rs`), not against the file.

**One manifest change the design did not predict.** The first build failed
with **110 errors**, all `error[E0463]: can't find crate for 'serde'`,
because the generated code writes `#[derive(serde::Serialize,
serde::Deserialize)]` by bare path. `vpay-db` had `serde_json` but not
`serde`; adding `serde.workspace = true` fixes it. Worth recording because
the error names the macro invocation, not the missing manifest line.

After that the whole schema — all seven models, six enums, the generated
`pub mod axum` — compiles with **zero warnings**, including under
`unreachable_pub`/`missing_debug_implementations` and the workspace's
`unwrap`/`expect`/`panic` clippy denies.

`PgRepositories` gains `cs: crate::schema::cratestack_schema::Cratestack`,
built from `pool.clone()` in `boxed`. One pool, two views:
`SqlxRuntime::new` takes a pool the caller built and `pool()` hands it back.

`is_client_disabled` becomes
`self.cs.disabled_client().find_unique(id).run(&system_context()).map(|r| r.is_some())`,
errors through `classify_cratestack("DisabledClient", "read", …)` into
`DbError::Persistence`.

**One call site needed an annotation, and it is a real fact about the seam.**
`UnitOfWork::transaction`'s `E: From<DbError>` used to have exactly one
candidate impl (reflexive `From<T> for T`), so `E` fell out of inference.
`DbError::Persistence(#[from] PersistenceError)` adds a second and the bound
becomes ambiguous:

```
error[E0283]: type annotations needed
note: multiple `impl`s satisfying `error::DbError: From<_>` found
   --> backends/crates/vpay-db/src/error.rs:211:19
    |
211 |     Persistence(#[from] crate::PersistenceError),
```

Exactly one site in the workspace was affected (`repository::closure_shape`,
the compile-only guard); it now spells
`-> TxFuture<'_, Result<TxOutcome<crate::ChargeRow>, DbError>>`, with a
comment saying why. `cargo build --workspace --all-targets` is otherwise
clean.

---

## 5. Decisive test 3 — the gate, measured in both directions

The claim being made is that the gate as it stood **could not** see either
evasion. That was measured by checking out the base `.xtask/src/main.rs` over
the branch's and running it against the mutated tree:

```
=== BASE gate, mutation 3a (pub mod schema;) ===
verify-repositories: ok — 3 concrete implementation(s) … exit=0
=== BASE gate, mutation 3b (pub use schema::cratestack_schema;) ===
verify-repositories: ok — 3 concrete implementation(s) … exit=0
```

With the new third check:

```
=== MUTATION 3a: pub mod schema; ===
xtask: repository-pattern violations:
  - backends/crates/vpay-db/src/lib.rs:66: `pub mod schema;` publishes the module holding
    `include_server_schema!`, and with it every generated model, delegate and the generated
    `axum` surface, as `vpay_db::schema::…`. Declare it `mod schema;` (ADR-0016, standard 5)
exit=1

=== MUTATION 3b: pub use schema::cratestack_schema; ===
xtask: repository-pattern violations:
  - backends/crates/vpay-db/src/lib.rs:105: a `pub use` naming `schema` re-exports the
    expansion of `include_server_schema!` out of `vpay-db`. The generated module does not
    exist in any source file, so nothing else in this repository would object
    (ADR-0016, standard 5)
exit=1

=== restored ===
verify-repositories: ok — 3 concrete implementation(s) in backends/crates/vpay-db
(PendingTransaction, PgRepositories, SqlClientAssertionStore), named by none of the 66
source file(s) outside it, and no generated schema module is exported
exit=0
```

`DB_HANDLE_TYPES` also grew to `["PgPool", "Transaction", "Cratestack",
"SqlxRuntime"]`. Three new `.xtask` tests:

- `a_cratestack_handle_is_a_handle_and_its_error_is_not` — a synthetic
  `struct CsChargeStore { cs: Cratestack }` and `struct RawRuntimeStore {
  runtime: SqlxRuntime }` join the set; `struct PersistenceError { inner:
  CratestackError }` and `struct Scoped { ctx: CratestackContext }` do not.
  Whole-identifier matching is what separates them, and both halves are
  asserted.
- `publishing_the_generated_schema_module_fails_the_gate_itself` — the four
  cases above plus `pub(crate) mod schema;` (allowed: it leaves no crate) and
  "the module is not declared at all" (fails, rather than passing vacuously).
- `a_similarly_named_module_is_not_the_generated_one` — `pub use
  schema_helpers::Thing;` is not a hit; a private `use` is not a leak.

`cargo nextest run -p xtask`: **197 passed, 0 skipped** (194 before).

---

## 6. Errors

`PersistenceError` has six variants and one `impl Classify`;
`DbError::Persistence(#[from] …)` delegates in both `category()` and `code()`
by naming the variant, which `verify-errors` requires.

Unit tests in `persistence.rs`:

- `a_duplicate_key_classifies_the_same_through_cratestack_as_through_sqlx` —
  a `23505` through `classify_cratestack` gives the *same* `category`, `code`
  and `retry` as `DbError::UniqueViolation` (`Conflict` / `resource_conflict`
  / `Never` / 409), and **`assert_ne!` against CrateStack's own answer**,
  which is `500` for a `DatabaseTyped`. A `23503` is checked against
  `DbError::ForeignKeyViolation` the same way.
- `a_policy_denial_is_internal_and_never_the_callers_fault` — `Forbidden` →
  `Internal`, `Retry::Never`, `assert_ne!(…, Category::Forbidden)`, the
  public message carries none of the framework's text, and the classification
  survives the trip through `DbError`.
- `an_untyped_read_failure_is_a_storage_outage_like_the_sqlx_read_it_replaced`
- `the_system_context_is_the_one_the_schema_policy_names`
- `a_non_database_sqlx_error_is_not_classified_as_an_integrity_violation`

**Two honest limits**, both in the code's doc comments rather than glossed:
`FindUnique::run` maps its `sqlx::Error` with
`CratestackError::Database(error.to_string())` — **not** through
`cratestack_error_from_sqlx` — so a CrateStack *read* never carries a
SQLSTATE and every failure of the one query vpay runs lands on `Backend` →
`Storage`. And `Forbidden` is produced only on the write/batch paths
(`query/write/*_exec.rs`, `query/batch/*`), never on a read, because a
refused read is a `WHERE` clause. The SQLSTATE and `Denied` arms are
therefore unit-tested rather than exercised, and say so.

---

## 7. Gates run on this branch, and the ones this host could not run

The rootless Docker daemon on the authoring machine wedged twice on
2026-09-06 (two `dockerd` processes became kernel-stuck zombies under disk
saturation, load above 150) and was declared unrecoverable until a reboot.
Everything that needs a container is therefore **written, compiled and
listed, but not executed here**. That is stated as a gap rather than papered
over, and each unexecuted case is named below.

### Run, green

| Gate | Result |
|---|---|
| `cargo build --workspace --all-targets` | clean, **zero warnings** — including the whole generated schema under `unreachable_pub`, `missing_debug_implementations` and the `unwrap`/`expect`/`panic` clippy denies |
| `just fmt-check` (`cargo fmt --all -- --check`) | exit 0 |
| `just clippy` (`--workspace --all-targets -- -D warnings`) | exit 0 |
| `just verify` — the **ten** gates | all ok; `verify-repositories` now also prints "and no generated schema module is exported" |
| `just check-schema` | `cratestack 0.11.1`, 13 model/enum declarations, `schema OK` |
| `cargo nextest run -p vpay-db --lib` | **24 passed, 0 skipped** — including all five `persistence::tests` |
| `cargo nextest run -p xtask` | **197 passed, 0 skipped** (194 on master) |
| `just test-doc` | **90 passed, 0 failed, 1 ignored**. This branch adds **no** doctest fence (`git diff origin/master..HEAD -- '*.rs' \| grep -c '^+.*```'` = 0), so the count is master's. |
| `just verify-ignored` | `0 ignored (expected 0), 41 test binaries (expected 41), 1288 total (minimum 1080)` — master was 1279, and +9 is exactly this branch's 1 parity + 5 persistence + 3 xtask tests. No new test binary, so `expected_suites` and `min_tests` do not move. |
| `just deny` | `advisories ok, bans ok, licenses ok, sources ok` |
| `just docs-check` | ok (`verify-status`, `verify-links`) |
| `just lint-web` | exit 0 |
| `just test-web` | exit 0 |

### Run before the outage, on this branch

`cargo nextest run -p vpay-tests-integration --test postgres_smoke
the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` —
**PASS**, with the final constants (85 / 16) and the final asserted table
set, against a `postgres:16-alpine` testcontainer. The only thing that has
touched that file since is `cargo fmt --all`, and the tree is rustfmt-clean.
The failing runs quoted in §3 are from the same session.

### NOT executed on this host — owed to CI

Each of these compiles and is listed by `cargo nextest list` (they are part
of the 1288 above); none has been run:

1. **`vpay-db::repositories a_disabled_client_reads_the_same_through_both_paths`**
   — the parity test, and the single most important case on this branch. It
   has **never been executed**. Until it runs green, "the CrateStack read
   returns what sqlx returns" is a claim supported by reading the generated
   SQL builder, not by a measurement.
2. **Decisive test 2** — delete `@@allow("read", auth().isSystem())` from
   `model DisabledClient` and confirm the parity test **FAILS**. Not run,
   because it needs test 1 to run first. The mechanism is read out of
   `cratestack-sqlx-0.11.1/src/query/support/conditions.rs`
   (`push_action_policy_query`) and `cratestack-core`'s
   `context/system.rs`, and the schema's own comment records it — but it is
   an argument, not evidence, until the mutation has been run.
3. **`just test-rust`** (`cargo nextest run --workspace`) — the workspace was
   *listed* (1288 tests, 41 binaries, 0 ignored) but not run. Every
   container-backed suite in `vpay-db`, `vpay-tests-integration`,
   `vpay-server` and `vpay-worker-bin` is therefore unexecuted on this
   branch, including the ones this change could plausibly disturb:
   `vpay-db::repositories disabled_client_lookup_reflects_disable_and_enable`
   (the pre-existing kill-switch test, which exercises the changed method)
   and the `client_store` / `merchant_token_flow` integration suites that
   reach `is_client_disabled` through the token path.
4. **The re-run of the drift test on the final tree** — see above; it passed
   on the same content before the outage, but not after the last
   `cargo fmt --all`.
5. **The server image build and its size delta.** Not attempted. The musl
   static link now has to carry CrateStack's twelve crates plus `minicbor`,
   `chumsky`, `ariadne` and the rest, and **the size cost of this change is
   unmeasured**. No buildx builder was created, so there is nothing to clean
   up.

Nothing in this branch was weakened, skipped or `#[ignore]`d to accommodate
the outage: the tests exist, compile and are listed, and the list above is
the honest account of which of them a machine has actually run.

---

## 8. Rebased onto `c456f24` (2026-09-06)

This branch was rebased from `65a5952` onto master at **`c456f24`**, which had
since merged #49 (account-holder lookup — a new port method, a capability, the
MTN adapter, `GET /v1/account_holders`, both SDKs and their parity rows), #51
(`GET /v1/refunds/{id}` — `vpay_db::Refunds`, `RefundObject`, a new integration
binary, `expected_suites` 41 → 42 → 43), and #52 (the two-directional
`verify-sdk-parity` in `.xtask/src/main.rs`).

**Fifteen commits replayed with zero conflicts, which is the thing to be
suspicious of rather than pleased about.** `git merge-tree` reported a clean
tree before the rebase was started, so every overlap was checked by hand
afterwards instead of being trusted: `.xtask/src/main.rs` (both #52's
SDK-reading parity tests and this branch's `repository_tests` are present —
the two `the_repositorys_own_tree_passes` functions that a duplicate-name scan
flags are **not** a duplicate definition, one is `serde_tests`' and one is
`repository_tests`'), `backends/crates/vpay-db/src/lib.rs` (#51's
`pub mod refunds;` / `pub use refunds::{RefundRow, Refunds};` *and* this
branch's `mod persistence; mod schema;` both survived), `repository.rs`,
`justfile` (#52's header text and this branch's `cratestack_min_declarations
:= "13"` are in different hunks), `CLAUDE.md` (master's "ten self-checks"
correction plus this branch's CrateStack bullet), `Cargo.lock`, and
`docs/{status.md,reference/vpay-db.md,flows/merchant-auth.md}`.

`Cargo.lock` needed no hand-merge and `cargo build` did not move it: the tree
still resolves **497 packages**, exactly one `sqlx` and one `sqlx-core` (both
`0.9.0`, which is what `run_in_tx` requires), and the twelve `cratestack-*`
crates at **`0.11.1`**.

**The drift constants stay at 85 / 16.** #49, #51 and #52 added no migration —
`backends/migrations` holds **30** files on `c456f24` and 30 here — so nothing
about the schema-versus-migrations delta moved.

### Re-measured on the rebased tree, not carried over

| Gate | Before the rebase | On `c456f24` + this branch |
|---|---|---|
| `cargo nextest run -p xtask` | 198 passed | **215 passed, 0 skipped** |
| `just verify-ignored` | 41 binaries, 1289 total | **43 binaries, 1361 total, 0 ignored** |
| `just test-doc` | 90 passed, 1 ignored | **94 passed, 1 ignored** |
| `cargo nextest run -p vpay-db --lib` | 24 passed | **24 passed, 0 skipped** |
| `just verify` | ten gates ok | **ten gates ok**, incl. `verify-sdk-parity: 354 proving test(s), 28 dated gap(s), 14 SDK method(s) across 17 row(s)` |

`expected_suites` was re-measured and **stays 43**: this branch adds ten tests
and no test binary. `test-doc` is master's 94 — the only fence this branch adds
is a ```` ```text ```` block, which compiles nothing. `just fmt-check`,
`just clippy`, `just deny`, `just docs-check`, `just lint-web` and
`just test-web` are all exit 0, and `cargo build --workspace --all-targets` is
clean.

The three non-container mutations were re-run on the rebased tree and all three
still fail as designed: deleting the two `BlueOak-1.0.0` exceptions fails
`just deny` (exit 4, both `minicbor` and `minicbor-serde` rejected); `pub mod
schema;` fails `verify-repositories`; and
`pub type X = crate::schema::cratestack_schema::Cratestack;` fails it too, on
the alias/`pub fn` arm the review added.

### Rebased again onto `6978901` (2026-09-06)

Rebased a second time, from `c456f24` onto master at **`6978901`**, which
merged #50 (`refunds.fee`: migration `0031_refunds-fee.sql`, the column in
`vpay_db::Refunds`' `COLUMNS` and `RefundRow`, a ten-key `RefundObject` with a
typed `vpay_core::RefundStatus`, the port's `Refunded { fee }`, both merchant
SDKs' `fee` fields, the parity rows and the docs).

**Sixteen commits replayed with zero conflicts — checked by hand rather than
trusted, for the second time.** The overlaps:
`backends/tests/integration/tests/postgres_smoke.rs`, where master's
migration-count assertion moved 30 → **31** and gained #46's sentence about
`0031 refunds.fee` while this branch's drift block below it is untouched —
both sides survived; and `docs/status.md`, `docs/reference/vpay-db.md` and
`docs/flows/merchant-auth.md`, where #50's rows and this branch's CrateStack
corrections are independent paragraphs. `vpay-db/src/lib.rs` did not conflict
at all this time: #50 changed `refunds.rs`, not the export list.
`cargo build --workspace --all-targets` is clean, which is the only thing that
proves a conflict-free rebase did not silently drop a delimiter.

**The drift constants do not move: still 85 changes over 16 relations, and 18
unmappable columns.** Re-measured with the drift test itself on the rebased
tree rather than reasoned about, because #50 *did* add a column.
`0031_refunds-fee.sql` puts `fee` on `refunds`, a table `schemas/vpay.cstack`
does not declare at all, and an undeclared table contributes exactly one
`table ... is not declared in the schema` line whatever its column count — so
the schema grew by a column and this total did not move. `refunds.fee` is
`numeric`, which cratestack maps, so it did not join the 18 unmappable
columns either. All three constants now carry that reasoning, dated, in their
own doc comments.

### The five formerly-owed items, measured 2026-09-06

The authoring host's Docker daemon came back (the original daemon, cached
images). Every item §7 and the section above listed as *written, compiled and
listed but never executed* has now been executed on a machine. `just ci` ran
**end to end, exit 0**.

| # | Owed | Measured 2026-09-06 |
|---|---|---|
| 1 | `vpay-db::repositories a_disabled_client_reads_the_same_through_both_paths` | **PASS — first execution ever.** 6.371 s standalone, and 1.627 s as case 1042/1369 inside the full `just ci` run. "The CrateStack read returns what the sqlx read returns" is now a measurement, not a reading of the generated query builder |
| 2 | The decisive read-policy mutation | **FAILS as designed** — transcript below |
| 3 | `just test-rust` (`cargo nextest run --workspace`) | **1369 tests run, 1369 passed, 0 skipped**, 722.977 s. Master is 1359 (this branch adds exactly ten test attributes and removes none), so 1359 + 10 = 1369 |
| 4 | The drift test on the final tree | **PASS**, 13.919 s, `drift detected in 16 table(s)/view(s) (85 change(s) total)` |
| 5 | The server image and its size delta | **Built, ran, and it found a defect** — see below. `vpay-server 0.1.0`, **16.9 MB** against master's **16.1 MB**: **+0.8 MB (+5.0%)** |

Item 3's three named suites, which §7 called out specifically because this
change could plausibly disturb them, all pass:
`vpay-db::repositories disabled_client_lookup_reflects_disable_and_enable`
(1.274 s), `vpay-tests-integration::client_store` including
`find_client_reflects_the_disabled_clients_kill_switch` (1.138 s), and all ten
`vpay-tests-integration::merchant_token_flow` cases including
`a_disabled_client_is_refused_with_invalid_client_and_401` (2.417 s).

**Item 2, the decisive mutation, in full.** Deleting `@@allow("read",
auth().isSystem())` from `model DisabledClient` in `schemas/vpay.cstack` and
re-running the parity test:

```
thread 'a_disabled_client_reads_the_same_through_both_paths' panicked at
backends/crates/vpay-db/tests/repositories.rs:471:5:
assertion `left == right` failed: a row written by the sqlx path must be
visible to the CrateStack read: CrateStack says false, sqlx says true. If
CrateStack says false and sqlx says true, the model's `@@allow("read",
auth().isSystem())` clause is missing or the context this crate reads under
stopped being a SystemContext — the read is compiled into the WHERE clause,
so a denied row is indistinguishable from an absent one and the kill-switch
is silently OFF
  left: false
 right: true
test result: FAILED. 0 passed; 1 failed
```

That is the argument in §4 turned into evidence: the policy is load-bearing,
its absence is silent, and only this test catches it. `just check-schema`
stays green through the mutation. The schema was restored and the tree is
clean.

### What the image build found — a defect this branch introduced

**Item 5 failed on its first attempt, and the failure was real rather than
environmental.** `docker buildx build -f backends/Dockerfile --target server`
on a dedicated `vpay-exp14-land2` builder:

```
error: failed to read schema file
/build/backends/crates/vpay-db/../../../schemas/vpay.cstack:
No such file or directory (os error 2)
error: could not compile `vpay-db` (lib) due to 1 previous error
```

`backends/Dockerfile` copies `.xtask`, `backends`, `sdks/rust` and
`examples/merchant-demo` into the build context and **never copied
`schemas/`**, because until this branch that directory was documentation.
`include_server_schema!` resolves its path against `CARGO_MANIFEST_DIR`, so it
climbs two levels out of `backends/` to a file that was not in the image at
all. **This branch, as reviewed and as rebased, made the release image
unbuildable, and nothing in `just ci` would ever have caught it** — the gate
builds on the host, where the file is present.

Fixed here by adding `COPY schemas ./schemas` to **both** the `planner` and
`builder` stages, the two the Dockerfile's own header rule 3 requires to stay
identical, each with a comment naming the macro and the error. The `chef`
cook stage does not need it: cargo-chef stubs workspace members, so the macro
never expands there. After the fix the image builds, and the binary runs:
`docker run --rm … --version` → `vpay-server 0.1.0`.

**The size delta, measured paired rather than quoted.** The 15.7 MB figure in
`docs/plans/exp11-notes/opus-review.md` is from a different tree and a
different toolchain, so master was rebuilt from a `git archive` of `6978901`
on the same host, the same builder and the same day rather than compared
against it:

| Image | Size |
|---|---|
| master `6978901`, `--target server` | **16.1 MB** |
| this branch, `--target server` | **16.9 MB** |
| delta | **+0.8 MB, +5.0%** |

So CrateStack's twelve crates plus `minicbor`, `chumsky` and `ariadne` cost
**0.8 MB** on the static musl link, for one read. That is the number §7 said
was unmeasured. Both images run and both print `vpay-server 0.1.0`. The
`vpay-exp14-land2` builder was removed afterwards.

### What is still owed

Nothing on the list above. What remains is that **all of it was measured on
one host, once** — a single sample per number, on the authoring machine's
rootless Docker, and CI is the second measurement. In particular the image
fix is exercised by no gate in this repository: `just ci` never builds a
container, so the `COPY schemas` line is protected only by CI's `e2e
(compose)` and `release` jobs actually building the Dockerfile. That is a
real gap and it is named rather than closed here, because closing it means
adding a Docker build to a gate, which is a scope and a runtime decision for
the maintainer rather than something to slip into this branch.
