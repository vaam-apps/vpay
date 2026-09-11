# `vpay-db` — CrateStack: what it is, what stays raw, and what compiling proves

_Moved out of [docs/reference/vpay-db.md](../vpay-db.md) on 2026-09-11 by exp57, which split a 3 330-line reference into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

This page carries the seam itself. **The per-step record is on four sibling pages**,
and the text below jumps straight from "Re-checked at 0.12.0" to "Why the generated
module is private" because they sit between the two on `vpay-db.md` as it was:

- [cratestack-what-runs-through-it.md](cratestack-what-runs-through-it.md) — the
  transaction seam measured rather than argued, `events.data`, the event vocabulary
  hazard, and the per-action cost of a missing policy
- [cratestack-currencies-and-providers.md](cratestack-currencies-and-providers.md) —
  migrations 0032 and 0033, `reconcile`, and decision D7
- [cratestack-money-tables.md](cratestack-money-tables.md) — migration 0037, the
  `jsonb` blocker, and the ten report lines that are false
- [cratestack-procedure.md](cratestack-procedure.md) — the `procedure` seam and
  `searchPaymentIntents`, the schema's first, 2026-09-11

## CrateStack

`vpay-db` compiles `schemas/vpay.cstack` with
[CrateStack](https://cratestack.dev)'s `include_server_schema!` macro
(`cratestack = { package = "cratestack-pg", version = "=0.12.0" }`) and runs
**thirty-two** statements through the generated data layer, spread over
**twelve** tables.
This section says which twelve, what deliberately did not move, and which of
CrateStack's behaviours vpay has had to work around rather than adopt. It is
the application's side of `schemas/vpay.cstack`'s own header, which carries
the schema's side.

**Corrected 2026-09-10 (issue #87), and the correction is two things.** These
two lines read ~~"**twenty-two** queries … over ten"~~ and ~~"**twenty-four**
queries … over nine"~~ _one after the other_, contradicting each other in
consecutive sentences — a conflict resolved by keeping both halves — and
neither was right. **Thirty-two over twelve** is measured, by the rule the
registry below states, and the running totals in the paragraphs that follow
were never re-derived after S4b, S5 or ADR-0017: read them as the history of
what each change _said_, not as arithmetic that adds up to today's number.

It said "**one** query" until 2026-09-06, when the two `disabled_clients`
writes followed the read; "**three**" until later the same day, when migration
0032 made `currencies` modellable and `ConfigReconcile::reconcile`'s currency
pass moved; and "**five**" until migration 0033 dropped the `providers`
capability defaults and the provider pass moved with them. It said "**six**"
until the outbox landed, and "**eight** over five tables" from 2026-09-06,
when S4a's `customers` added `touch_last_used` and `delete`.

~~**"Twenty-two over ten" since 2026-09-07** (S4b's `invoices` and
`invoice_items`, one query each — see "Migration 0036" below). It was
**"twenty over eight"** earlier the same day
([ADR-0017](../../adr/0017-staff-authentication.md)),~~ ~~**"Twenty-four over
nine" since 2026-09-07** (S5, the money tables), and the +4 is
`checkout_sessions`' four reads~~ — **two totals written the same day on
branches that did not see each other, both kept, left contradicting each
other in consecutive sentences, and struck 2026-09-10 for their arithmetic
and not for their substance.** The substance is right, and the sections below
expand it: S4b added `invoices` and `invoice_items`, one statement each;
ADR-0017 added three whole tables; and S5 added `checkout_sessions`' four
reads — the first and so far only CrateStack statements on a table money
moves through. The section "The money tables: what moved, and what stays raw
forever" below is the whole account, including why the other three money
tables moved **nothing** and what would have to change upstream before they
could.

ADR-0017's increment is different in kind from every one before it: it is not
a method here and a method there, it is **three whole tables whose every
repository method runs through the generated layer** — sixteen of the
thirty-two statements in the registry below, exactly half, on tables created
for the purpose. `staff_members`
(**seven**, corrected 2026-09-10 — `create`, `find_by_email`, `find`,
`enrol_totp`, `record_totp_step`, `set_password`, `record_sign_in`; this said
six from the day it was written), `staff_sessions` (six) and
`oauth_authorization_codes` (**three** generated calls — `store_code`'s
`create`, and `consume_code`'s `find_unique` followed by the `update_many`
that is the swap; this said two, counting only the pair inside `consume_code`)
have no raw `sqlx` statement between them.

That is a property of migration `0035` rather than of ambition. Everything
this section records as a reason for staying on raw `sqlx` was designed out
of those three tables before they were created: no `jsonb` (which is what
keeps five of `Customer`'s seven hand-written and `Event`'s insert blocked),
no `bytea`, no native enum (the defect migration 0032 had to convert
`providers.flow` out of), no `DEFAULT` on any column a writer names (0033's
problem), and no `seq` cursor (whose correlated sub-select no delegate
expresses). The cost is stated where it is paid: the encrypted TOTP secret is
base64url `TEXT` rather than `BYTEA`, and `staff_members.last_totp_step` is
`NOT NULL` seeded to 0 because `NULL < step` is NULL in SQL and a nullable
column would refuse every staff member's _first_ TOTP code forever.

**The measured drift is the evidence.** The three tables cost 17 lines
(`EXPECTED_DRIFT_CHANGES` 113 -> 130) and every one of them is a hand-named
CHECK or an undeclared index — the two kinds 0.11.1 structurally cannot close.
Not one `column ... type differs`, not one `column ... default value differs`,
not one `column ... is declared in the schema but does not exist`, and
`EXPECTED_UNMAPPABLE_COLUMNS` does not move at all. Every earlier modelled
table carries at least one of those.

One more thing worth carrying: **the table name is decided by the model
name.** 0.11.1 derives it with
`cratestack_core::route_naming::pluralize(to_snake_case(model))` and has no
`@@map`, so `model Staff` reads and writes a table called `staffs`. The first
draft of migration 0035 created `staff`, every query answered `relation
"staffs" does not exist`, and **no gate said anything** — not `cargo build`,
not `just check-schema`, not `clippy`, not any of the ten `just verify` gates —
until a container-backed test ran. The model is `StaffMember` and the table is
`staff_members`; the Rust trait is still `vpay_db::Staff`, because it is a
trait about staff and not about a table.

Everything here was measured against the 0.11.1 sources on 2026-09-06;
`docs/plans/exp14-notes/opus.md`, `docs/plans/exp16-notes/opus.md` and
`docs/plans/exp17-notes/opus.md` have the transcripts.

### Re-checked at 0.12.0 (2026-09-07), and nothing below changed

The pin moved `=0.11.1` → `=0.12.0` (CLI and library together, as they must
be). The version numbers in this section moved with it, and they were
re-derived rather than substituted, because "we upgraded and the prose still
says what it said" is the failure this section is otherwise wide open to.

Five measured upstream gaps are named below. Four were measured at 0.11.1 and
**all four are still open at 0.12.0**; the fifth was found at 0.12.0 on
2026-09-07, when S5 declared relations on four money tables and every one of
the ten foreign keys came back as drift. The cheapest honest proof is file identity: each file the claim
rests on is byte-identical between the two releases (`md5sum`), and the
whole of `cratestack-core`, `cratestack-sqlx`, `cratestack-sql`,
`cratestack-pg` and `cratestack-policy` is unchanged at the source level. The
fifth gap needs no such argument: it is documented by the tool itself, in the
module that would implement it.

| Gap                                                                                                            | Where it lives at 0.12.0                                                                                                                                                                                  | State                                         |
| -------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------- |
| `@default(...)` fields are in neither `Create{Model}Input` nor `upsert_update_columns`                         | `cratestack-macros-0.12.0/src/model/inputs.rs:20-23` (`create_input_fields` filters `is_generated_on_create`), which is `has_default` at `src/shared/attrs.rs:91-93`; `model/descriptor/columns.rs:85-91` | **open** — both files md5-identical to 0.11.1 |
| `upsert(..)` gates the update policy on a _second_ pooled connection                                           | `cratestack-sqlx-0.12.0/src/query/write/upsert_exec.rs:45` and `upsert_resolve.rs:161-169` (`row_passes_update_policy(runtime.pool(), …)`)                                                                | **open** — whole crate's `src/` unchanged     |
| `Value::from_plain_json` demotes any non-`i64` number to `f64`                                                 | `cratestack-core-0.12.0/src/value.rs:95-106`                                                                                                                                                              | **open** — whole crate's `src/` unchanged     |
| `jsonb`, `bytea`, `int2`/`int4` have no read-back mapping, so introspection excludes those columns             | `cratestack-migrate-0.12.0/src/introspect/postgres/types.rs:16-36`, with the tool's own test asserting `map_scalar("int4", …) == None` at line 57                                                         | **open** — file md5-identical                 |
| Foreign keys are not introspected at all, so every `.cstack`-declared relation reports as a missing constraint | `cratestack-migrate-0.12.0/src/introspect/postgres/mod.rs:21-26` ("Known gaps"), `TableProjection::foreign_keys` set to `Vec::new()` at `:109`                                                            | **open** — found 2026-09-07 by S5             |

What 0.12.0 _did_ change, and why none of it reaches this document: the
breaking change gives `SchemaError` file identity, so `render()` takes no
arguments (`cratestack-macros-0.12.0/src/include/parse.rs:30-36`,
`cratestack-cli-0.12.0/src/migrate/baseline_cmd.rs:56-60`) — internal to the
macro and the CLI, and `just check-schema` reads an exit code rather than the
diagnostic. `cratestack-macros` also grew enum-typed query filters on
generated list routes (`src/shared/enum_query_parser.rs`, new), which vpay
compiles and does not call, and `cratestack-migrate` gained one doc comment
about `@computed` fields. The drift measurement was re-derived against a
fresh `postgres:16-alpine` at 0.12.0 and is unchanged at **101 changes / 16
relations / 17 unmappable columns**.

### Why the generated module is private, and what keeps it that way

`include_server_schema!` expands to `pub mod cratestack_schema { … }` **at the
invocation site**. Put it in `lib.rs` and every generated model struct, every
query delegate and a whole generated `pub mod axum` become `vpay_db::…` — a
wholesale reversal of ADR-0016 standard 5 in a one-line diff. So the
invocation lives in `src/schema.rs`, declared `mod schema;`, and nothing is
re-exported from it.

That could not be left to review, because the module the macro creates does
not exist in any source file: no lint, no rustdoc pass and neither of
`verify-repositories`' two original signals can see a
`pub use schema::cratestack_schema;`. **Measured on 2026-09-06: with the macro
in place, both `pub mod schema;` and `pub use schema::cratestack_schema;` made
`cargo xtask verify-repositories` print `ok`.** The gate now carries a third
check that fails both, and `DB_HANDLE_TYPES` has learned `Cratestack` and
`SqlxRuntime` so that a future store holding the runtime instead of a `PgPool`
is recognised as an implementation.

### What stays raw sqlx, and why

Everything else. That sentence used to begin "Everything else, **including
the two `disabled_clients` writes**"; those moved on 2026-09-06, and the
`currencies` and `providers` upserts followed the same day, so the line was
~~three tables wide~~ **— twelve tables wide as of 2026-09-10, the count this
clause stopped tracking after S4b, S5 and ADR-0017; the registry above is the
list** — plus one statement on a table that has otherwise moved.
That statement is `reconcile`'s disable pass, `UPDATE providers SET enabled =
false WHERE code <> ALL($1) AND enabled`: it addresses rows by their
_absence_ from a list, which no generated builder expresses, and it is the
statement that makes "configuration is the authority" true for a rail the
deployment dropped. Three properties of 0.12.0 decide where it
falls, and none of them is a matter of taste:

- **Model policies are compiled into the SQL.** `@@allow`/`@@deny` become
  predicates in the `WHERE` clause of every generated read and write, and a
  model with no `@@allow` is deny-by-default. A policy mistake therefore does
  not raise an error — it returns zero rows. On the kill-switch that means
  "no client is disabled", silently. `model DisabledClient` carries
  `@@allow("read", auth().isSystem())` and nothing else, and the parity test
  above is what makes a missing clause red.
- **Multi-column CHECK constraints are invisible to the migration tooling.**
  `cratestack-migrate` introspects only single-column CHECKs
  (`AND array_length(c.conkey, 1) = 1`), and the grammar has no
  `@@check(expr)` at all. vpay's ten cross-column CHECKs — `no_over_refund`,
  `partial_refunds_imply_refunds`, `lock_is_paired` and the rest — cannot be
  expressed here and would not be noticed missing. `backends/migrations/*.sql`
  stays the authoritative schema; the `.cstack` file is compared against it,
  never generated from it.
- **Six Postgres types are mapped, and vpay uses more than six.** `int4`,
  `int2`, `numeric`, `jsonb`, `bytea` and arrays are not in `map_scalar`, so
  seventeen live columns are excluded from the drift comparison outright — the
  number `EXPECTED_UNMAPPABLE_COLUMNS` pins in `postgres_smoke.rs`. Since
  2026-09-06 four of those seventeen sit inside tables the schema models
  _fully_ (`events.data`, `events.fanout_attempts`,
  `webhook_deliveries.attempt`, `webhook_deliveries.status_code`), which is a
  sharper version of the same point: unmeasured drift can hide inside a table
  the report otherwise compares column by column, and only that constant says
  so.
- **`FOR UPDATE SKIP LOCKED` has no delegate.** `FindMany::for_update()`
  emits a bare `FOR UPDATE`, so `Jobs::claim` — and therefore the whole
  `jobs` table — stays hand-written. A lease mechanism that silently lost
  `SKIP LOCKED` would turn every worker's claim into a queue behind every
  other worker's. `jobs.payload` is `jsonb` and `jobs.attempts` is `int4`
  besides.

A fourth reason used to be recorded here — that moving a read and a write
together produces a parity test that cannot say which of the two it is
testing. That was an argument for _sequencing_, not for staying, and it was
honoured: the read landed on 2026-09-06 and the writes a change later, with
the read's test re-seeded from an inline `INSERT` so it kept testing the read.
See "What runs through it today" above.

### What compiling the schema does not prove

`cargo build` now parses and type-checks `schemas/vpay.cstack`, and
`just check-schema` runs the CLI over it. Neither of them knows anything about
the database, and it is worth writing down exactly how little that leaves,
because "it compiles" reads like more than it is. Both measured on
2026-09-06 (`docs/plans/exp14-notes/opus-review.md`, M7 and M8):

- **A modelled column may name a type the live table cannot produce.** Change
  `reason String?` to `reason Json?` on `DisabledClient` — the column is
  `TEXT` — and `just check-schema` says `schema OK`, `cargo build` succeeds,
  and `just clippy` and all ten `just verify` gates stay green. The failure
  arrives as an `sqlx` decode error at the first read, i.e. in production, or
  in the container-backed drift test and the parity test, which are the only
  two things that would have caught it.
- **A missing `@@allow` is invisible to every one of them.** Delete any of
  the four on `DisabledClient` and the same list stays green — including the
  three added on 2026-09-06 for the writes, re-measured then. That is true
  even for the two whose absence _does_ raise an error at runtime
  (`create`, `update`): the check happens in `cratestack-sqlx` at query time,
  not at macro-expansion time, so nothing a compiler or the CLI can see is
  different.

So the guard on all of them is container-backed by construction. Adding a
`model` to this schema is not free, and the drift test and the two parity
tests are what make it safe — not the compiler.

The transaction seam is untouched, and can stay untouched: every CrateStack
builder exposes `run_in_tx(&mut sqlx::Transaction<'_, Postgres>, &ctx)`, so a
write that needs to join vpay's own transaction can, and neither of the two
that moved on 2026-09-06 does — `disable_client` and `enable_client` are each
one statement with nothing to be atomic with, so both call `.run(ctx)` and let
CrateStack own whatever transaction it wants underneath (the upsert opens
one; `delete_many` opens one). `UnitOfWork::transaction` and
`TxOutcome::Abandon` survive — `UnitOfWork::transaction` and `TxOutcome::Abandon` survive —
CrateStack's own `transaction` combinator commits on `Ok` and rolls back on
`Err` with no third ending, and `Abandon` is what the confirm path's
duplicate-charge `409` is built on. The first write that _does_ need to be in
a vpay transaction is where `run_in_tx` gets exercised; nothing exercises it
today. This is also why the root `Cargo.toml`
pins `sqlx = "=0.9.0"` exactly rather than `"0.9"`: `run_in_tx` only accepts
vpay's transaction while both halves resolve the same `sqlx-core`.

### Errors: why vpay classifies them itself

`PersistenceError` (`src/persistence.rs`) is the leaf, `DbError::Persistence`
`#[from]`s it and delegates, and `classify_cratestack` is the single place a
`CratestackError` is read — the mirror of `error::classify_write`, branching
on the same SQLSTATEs and carrying the same constraint names.

CrateStack's own `status_code()`/`public_message()` are **not used anywhere**,
and that is load-bearing rather than stylistic: its `DatabaseTyped` variant is
a `500` with the canned message `"internal error"`, so a duplicate charge
would reach a merchant as an outage. Here a `23505` is a `409` with
`error.code = "resource_conflict"` and `Retry::Never`, exactly as the sqlx
path already made it —
`a_duplicate_key_classifies_the_same_through_cratestack_as_through_sqlx`
asserts the two agree _and_ that neither matches what CrateStack would have
said.

Two honest limits on that mapping:

- **A read never carries a SQLSTATE.** `FindUnique::run` maps its
  `sqlx::Error` with `CratestackError::Database(error.to_string())` rather
  than through `cratestack_error_from_sqlx`, so today every failure of the one
  query vpay runs lands on `PersistenceError::Backend` → `Category::Storage`.
  That is the same answer `DbError::Query` gave for the `SELECT` it replaced,
  so nothing regressed; the SQLSTATE arms exist for the writes that have not
  moved yet and are unit-tested rather than exercised.
- **`Denied` cannot be produced by the read path either.** A policy refusal on
  a read is a `WHERE` clause, not an error; `CratestackError::Forbidden` comes
  only from the write and batch paths. It classifies `Internal`, not
  `Forbidden`: every CrateStack call in this crate runs as the system
  principal, so a refusal means the schema and the call site disagree — a
  deploy bug that pages, never something to tell a merchant they are not
  allowed to do.

  **`Denied` stopped being unreachable on 2026-09-06**, when `disable_client`
  became an upsert: that path evaluates its policies in Rust and raises a real
  `Forbidden`, so the variant is now exercised by a container-backed mutation
  and not only by `persistence.rs`'s unit test. `enable_client` still cannot
  produce it — `delete_many`'s policy is a `WHERE` clause like the read's.
  The `Unique`, `ForeignKey` and `Check` arms remain unit-tested rather than
  exercised: `disabled_clients` has no foreign key and no CHECK, and the one
  unique constraint it has is the primary key the upsert exists to absorb.

### The context, and the policy it satisfies

`system_context()` returns `SystemContext::for_service("vpay-db")`'s inner
context. `SystemContext` is the only producer of a context that satisfies
`auth().isSystem()`, it has no `From<CratestackContext>` and no constructor
taking one, and `CratestackContext::system` is `#[serde(skip)]` — so the
marker cannot arrive over a wire and no request-derived context can become
one. That asymmetry is why the schema names `auth().isSystem()` rather than
`auth() != null`: the weaker predicate would be satisfied by any authenticated
caller if this table ever were reached on behalf of a request.

The service name reaches `actor.id = "system:vpay-db"` in
`cratestack_sqlx::audit::actor_from_context`. Nothing in vpay audits through
CrateStack yet — `model DisabledClient` carries no `@@audit`, so
`descriptor.audit_enabled` is `false` and neither write calls
`ensure_audit_table`; the same is true of `@@subscribe` and the event outbox,
so no generated write in this crate creates a table. The name is fixed now so
it does not have to be chosen retroactively for rows already written, and
2026-09-06 is when it first attributed one: until the writes moved, every
CrateStack call in this crate was a read.

### What this added to the dependency graph

`Cargo.lock` goes from 469 packages to 497 (+28), of which twelve are
`cratestack-*` and all twelve are MIT. `syn` moves 3.0.3 → 3.0.5. The feature
set is `default-features = false, features = ["postgres"]`, which drops
`decimal-rust-decimal` (this schema declares no `Decimal`; vpay's money is
integer minor units) and `codec-json` (vpay generates no client).

Two of the twenty-eight are **BlueOak-1.0.0** — `minicbor` and
`minicbor-serde` — and they are not optional. `cratestack-pg` declares
`cratestack-axum` and `cratestack-client-rust` as non-optional dependencies
with no feature gating either, and both take `minicbor` unconditionally;
dropping down to `cratestack-sqlx` + `cratestack-macros` directly does not
help, because `include_server_schema!` emits
`pub mod axum { use ::cratestack::HttpTransport; … }` unconditionally.
`deny.toml` therefore carries a **scoped** exception for those two crate names
— not an entry on the `allow` list — with the maintainer's decision and the
reasoning beside it. `cargo deny check` reports
`advisories ok, bans ok, licenses ok, sources ok`, and no other new licence or
ban appeared. `cargo tree -i aws-lc-rs` is still empty and there is still
exactly one `sqlx`.

The generated `pub mod axum` compiles and is never referenced: vpay keeps its
own router and its Stripe-shaped `/v1`. `crypto-aws-lc-rs` must never be
enabled — `deny.toml` bans `aws-lc-rs` (ADR-0005, ADR-0007), and at 0.12.0 the
feature is a `compile_error!` rather than a working mode in any case.

One ergonomic consequence, recorded because the failure message does not
explain itself: `UnitOfWork::transaction`'s `E: From<DbError>` bound used to
have exactly one candidate (the reflexive `impl<T> From<T> for T`), so `E`
fell out of inference. `DbError::Persistence(#[from] PersistenceError)` adds a
second, and a call site that named neither now needs
`-> TxFuture<'_, Result<TxOutcome<T>, DbError>>` on the closure. One site in
this workspace was affected (`repository::closure_shape`); the annotation is
there with a comment.
