# exp14 (opus) — sabotage review of the first CrateStack read

Reviewer pass over `claude/exp14-cratestack-read-opus` (base `65a5952`,
implementation HEAD `c0e967c`, six commits). Everything below was **run**,
on 2026-09-06, on the authoring host, under `rustc 1.98.0` (the pin),
`cratestack 0.11.1`, `cargo-deny 0.20.2`, Node `v22.23.2` (`.nvmrc`) with
`pnpm 9.15.0`. Docker was dead on this machine for the whole pass (two
kernel-stuck `dockerd` processes; load average above 150 throughout), so
**no container-backed case was executed by this review either** — §5 names
what is still owed to CI, and that list is unchanged from the
implementation's own.

---

## 1. The non-container gate, re-run

The project's own recipes, not reconstructions of them.

| Command | Result |
|---|---|
| `cargo build --workspace --all-targets` | exit 0, **0 warnings** (36m46s under load) |
| `just fmt-check` | exit 0 |
| `just deny` | exit 0 — `advisories ok, bans ok, licenses ok, sources ok` |
| `just verify` (ten gates) | exit 0 — all ten ok, `verify-docs` advisory |
| `cargo nextest run -p vpay-db --lib` | **24 passed, 0 skipped** |
| `cargo nextest run -p xtask` | **197 passed, 0 skipped** (master 194; +3) |
| `just test-doc` | **90 passed, 0 failed, 1 ignored** |
| `just docs-check` | exit 0 |
| `just verify-ignored` | `0 ignored (expected 0), 41 test binaries (expected 41), 1288 total (minimum 1080)` |
| `just clippy` | exit 0 |
| `just lint-web` | exit 0 |
| `just test-web` | exit 0 (302 + 3 cases; dashboard has none) |

Every number the implementation notes claim for these gates reproduced
exactly. `verify-repositories` prints the new tail — "and no generated schema
module is exported".

## 2. Claims checked against the artefact

| Claim | Verdict |
|---|---|
| `Cargo.lock` 469 → 497 (+28) | **true** at the package level. 25 of those are new crate *names*; the other three (`const-oid` 0.9.6, `foldhash` 0.2.0, `hashbrown` 0.15.5) are new *versions* of crates already in the graph. See finding R5. |
| licences of the new crates | **true**, read from every published manifest: twelve `cratestack-*` MIT; `ariadne`/`chumsky` MIT; `ar_archive_writer` Apache-2.0 WITH LLVM-exception; `foldhash` Zlib; the rest MIT/Apache dual; **`minicbor` 2.3.0 and `minicbor-serde` 0.7.1 BlueOak-1.0.0** |
| new duplicate majors are exactly `const-oid` and `foldhash` | **incomplete** — `hashbrown` gains a fourth version (0.15.5). Finding R5. |
| `cargo tree -i aws-lc-rs` empty | **true** — no `aws-lc-rs`/`aws-lc-sys` entry exists in `Cargo.lock` at all, and `deny.toml`'s ban is green |
| one `sqlx` | **true** — `sqlx-core`/`sqlx-postgres` appear at `=0.9.0` only; no new sqlx duplication |
| MSRV floor re-derived 1.94 → 1.98 | **true** — every `cratestack-*` 0.11.1 manifest declares `rust-version = "1.98.0"`; the other new crates declare 1.88 or less (`ar_archive_writer` 1.88.0, `object` 1.65, `chumsky` 1.65, `ariadne`/`minicbor` none) |
| the `cratestack-pg` feature set is minimal | **true** — `default = ["postgres", "decimal-rust-decimal", "codec-json"]`; `postgres` is `dep:cratestack-sqlx` + `cratestack-macros/postgres`, and dropping the other two is the smallest set that still yields a data layer |
| the Blue Oak exception is unavoidable | **true** — `cratestack-axum` and `cratestack-client-rust` are declared **non-optional** in `cratestack-pg`'s manifest; only `cratestack-sqlx` is feature-gated |
| `sqlx = "=0.9.0"` is a real change | **true** — base was `"0.9"`; `cratestack-sqlx` declares `sqlx-core = "=0.9.0"`/`sqlx-postgres = "=0.9.0"` and `run_in_tx` takes a bare `&mut sqlx::Transaction<'_, Postgres>` |
| the generated module is private and never exported | **true** — the only names of `cratestack_schema` outside comments are `repository.rs:553` (a `pub(crate)` field) and `:571` (inside a `pub(crate) fn` body); nothing else in the workspace names `cratestack`, and `cratestack.workspace = true` appears in exactly one crate manifest |
| the generated `axum` module is never mounted | **true** — no reference to `router()`, `model_router`, `procedure_router` or `HttpTransport` anywhere under `backends/` |
| the read goes through vpay's own pool, not a second one | **true** — `FindUnique::run` ends `.fetch_optional(self.runtime.pool())`, and `SqlxRuntime::new` stores the pool the caller built; `Cratestack::builder(pool.clone()).build()` in `PgRepositories::boxed` is that pool |
| a read never carries a SQLSTATE | **true** — `FindUnique::run` maps with `CratestackError::Database(error.to_string())`, not `cratestack_error_from_sqlx` |
| `@@allow("read", …)` reaches `find_unique`'s policy slot | **true**, and worth stating because it is not obvious: `find_unique` defaults to `ReadPolicyKind::Detail`, and the macro fills `detail_*` from the actions `["detail", "read"]` (`cratestack-macros/src/model/descriptor.rs:46`), so an `@@allow("read", …)` does populate the slot `find_unique` consults |
| a system context renders the predicate `TRUE`, a missing `@@allow` renders `(FALSE)` | **true** — `policy_predicate.rs:34-36` and `render/policy.rs:28-30` |
| the model matches migration 0012 exactly | **true** — three columns, `client_id TEXT PRIMARY KEY` / `disabled_at TIMESTAMPTZ NOT NULL DEFAULT now()` / `reason TEXT`, and `@default(now())` is what the migration writes |
| the drift constants and the table set moved in the schema's own commit | **true** — `53adad5` carries `schemas/vpay.cstack`, `postgres_smoke.rs` (86→85, 17→16, `disabled_clients` dropped from the asserted set) and the `justfile` floor together |
| the two "build facts" | **both principled, and both re-measured**: removing `serde.workspace = true` fails the build with 110 `E0463`s (M6 below); the `E: From<DbError>` annotation is the unavoidable cost of the `#[from]` composite ADR-0011 asks for, and exactly one site needed it |

## 3. Mutations run

| # | Mutation | Expected | Measured |
|---|---|---|---|
| M1 | delete the `deny.toml` Blue Oak exception | `cargo deny check licenses` FAILS naming exactly `minicbor` and `minicbor-serde` | **FAILS, exit 4**, `licenses FAILED`, exactly two rejected manifests: `minicbor-2.3.0`, `minicbor-serde-0.7.1`. Nothing else. |
| M2 | `mod schema;` → `pub mod schema;` | new gate FAILS, base gate passes | **base `ok` exit 0; new exit 1**, naming `lib.rs:69` and ADR-0016 standard 5 |
| M3 | append `pub use schema::cratestack_schema;` | new gate FAILS, base gate passes | **base `ok` exit 0; new exit 1**, naming `lib.rs:106` |
| M4 | append `pub type CratestackHandle = crate::schema::cratestack_schema::Cratestack;` | should FAIL | **compiles**, base `ok`, **new `ok` exit 0 — NOT CAUGHT** (finding R3) |
| M5 | append `pub fn cratestack_handle(pool: sqlx::PgPool) -> crate::schema::cratestack_schema::Cratestack` | should FAIL | **compiles**, base `ok`, **new `ok` exit 0 — NOT CAUGHT** (finding R3) |
| M6 | delete `serde.workspace = true` from `vpay-db` | build FAILS | **FAILS**, `error: could not compile vpay-db (lib) due to 110 previous errors`, all `E0463: can't find crate for serde` pointing at `schema.rs:23` |
| M7 | `reason String?` → `reason Json?` (a type the live `TEXT` column cannot decode into) | does anything refuse? | **`just check-schema` exit 0 (`schema OK`) and `cargo check -p vpay-db` exit 0.** No non-container gate refuses a modelled column whose type the table does not have. Only the container drift test and the parity read would. |
| M8 | delete `@@allow("read", auth().isSystem())` | does anything non-container notice? | **nothing does**: `check-schema` exit 0, `cargo check` exit 0, `verify-repositories` exit 0, and (by construction) `just clippy`/`just verify` too. The parity test is the sole guard — and it has never been executed. |

M7 and M8 are the two that matter for the verdict: this branch's two
load-bearing safety properties — *the model matches the table* and *the
policy admits the system read* — are both invisible to every gate that can
run without Docker.

## 4. Findings

| # | Severity | Finding |
|---|---|---|
| R1 | misleading-claim | The parity test's own doc comment claimed a decisive mutation had been run and cited a transcript that does not exist. |
| R2 | correctness (docs) | `is_client_disabled`'s `# Errors` contract still promised `DbError::Query`, which the new implementation can never return. |
| R3 | gate-hole | The new `verify-repositories` check reads only `pub mod` and `pub use`. A `pub type` alias or a `pub fn` returning the generated hub compiles, leaks the same surface, and passes. |
| R4 | misleading-claim | `docs/reference/vpay-db.md` called the parity test "the proof that it works" without saying it has never run, where `docs/status.md` and `docs/flows/merchant-auth.md` both do. |
| R5 | nit | The graph-delta accounting counts three new *versions* of already-present crates among the "28 new crates", and the duplicate-majors list omits `hashbrown` 0.15.5. |
| R6 | nit (recorded, no code change) | M7/M8: neither a wrong column type nor a missing read policy is refused by anything that runs without a container. Recorded in the reference doc so it is not rediscovered. |

Two things reviewed and **not** filed, so the reasoning is not lost:

- **The `E: From<DbError>` annotation at `repository::closure_shape`** is not
  a workaround. `#[from]` is what ADR-0011 and `verify-errors` ask a
  composite for, a second `From` impl is what `#[from]` means, and a bound
  with two candidates is ambiguous by the language's rules. One annotation at
  one site, with a comment saying why, is the honest cost.
- **`CLAUDE.md` was edited** outside the brief's list. The bullet it changed
  ("`schemas/*.cstack` is not wired into the build … do not try to make it
  compile") became false the moment `mod schema` landed, and leaving a false
  instruction in the file agents read first is worse than editing it. Correct
  call.

## 5. Still owed to CI — unchanged by this review

Docker was dead here too. The three cases the implementation named are still
the three:

1. `vpay-db::repositories a_disabled_client_reads_the_same_through_both_paths`
   — never executed.
2. The decisive mutation on it (delete `@@allow("read", auth().isSystem())`,
   confirm it fails). M8 above proves nothing *else* notices; it does not
   prove this test does.
3. `just test-rust` — every container-backed suite, including the
   pre-existing `disabled_client_lookup_reflects_disable_and_enable` and the
   `client_store` / `merchant_token_flow` suites that reach
   `is_client_disabled` through the token path.

Plus, from the same list and equally unexecuted here: the re-run of
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` on the
final tree, and the server image build (so the musl size delta of
CrateStack's graph is still unmeasured).
