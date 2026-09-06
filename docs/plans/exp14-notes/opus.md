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
genuinely new version duplicates are `const-oid` (0.9.6 / 0.10.2) and
`foldhash` (0.1.5 / 0.2.0); `multiple-versions = "warn"`, and `cargo deny
check bans` is green.

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

