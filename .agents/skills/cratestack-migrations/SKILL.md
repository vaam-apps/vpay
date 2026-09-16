---
name: cratestack-migrations
description: CrateStack database migrations — cratestack migrate diff and baseline, the snapshot model, destructive-change classification, up.pre.sql backfills, @@rename markers, the Postgres-only forward-only applier in cratestack-sqlx, and the SQLite differences. Load when generating or applying migrations, adopting an existing database, or debugging a lossy/blocking migration refusal.
---

# Migrations

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [cratestack/references/version-history.md](../cratestack/references/version-history.md).

Two subcommands. That is the whole authoring surface.

```bash
cratestack migrate diff --schema schema.cstack [--out-dir migrations]
                        [--backend postgres|sqlite|both] [--name slug] [--allow-destructive]

cratestack migrate baseline --schema schema.cstack --database-url <url>
                            [--out-dir migrations] [--backend postgres] [--strict]
```

**`migrate verify` does not exist** — one source comment claims it is the CI gate
for snapshot integrity; it is stale.

## The model: snapshot in, SQL out, no database

`migrate diff` never connects to anything. It is a pure pipeline:

```
Schema → project() → Projections IR → diff against snapshot → Ops → dialect SQL
```

Output layout:

```
<out-dir>/<backend>/<YYYYMMDDHHMMSS>_<slug>/{up.sql, down.sql[, up.pre.sql]}
<out-dir>/<backend>/schema.snapshot.json
```

**The snapshot must be committed.** An absent snapshot means "empty", so a first
run emits full `CREATE TABLE`s. The snapshot stores the **projections IR**, not
the schema — introspection can never recover mixins, procedures, auth blocks or
attribute provenance, so storing a `Schema` would be a lie. Format version 2;
**v1 snapshots are rejected with no migration path and must be regenerated**.

If there are no ops, it prints `no changes` and writes nothing.

## Destructive-change classification

| Class | Which ops | Behaviour |
| --- | --- | --- |
| Safe | create table, add/drop index, alter default, rename table/column, drop check, required→optional | written |
| **Lossy** | drop table, drop column, **any `AlterColumnType`**, list↔scalar arity flips | **refuses without `--allow-destructive`** |
| **Blocking** | optional→required, non-enum add check | written, plus a scaffolded `up.pre.sql` and a loud stderr warning |

`AlterColumnType` is lossy *unconditionally* — the IR has no dialect-aware
widening view, so even a widening change needs the flag.

The run prints its label: `safe`, `lossy`, `blocking`, or `blocking+lossy`.

## What is *not* auto-generated

**Renames.** Matching is by name only, so a rename looks like drop-plus-create
unless you declare it: `@@rename(from = "OldModel")` on a model,
`@rename(from = "oldField")` on a field. A **malformed rename marker is silently
treated as absent** and falls back to drop-plus-add — check your spelling.

**Backfills.** `up.pre.sql` *(since 0.10.0)* is scaffolded for a blocking migration and you write
the body. It runs immediately before `up.sql` **in the same transaction**. It is
written create-new and never truncated: an existing `up.pre.sql` is kept and the
scaffold is skipped with a note.

**`down.sql` for a lossy migration** is an explicit error stub, not reverse SQL.
Hand-write any destructive reversal.

**Enums on SQLite** — not emitted at all; variant changes are Rust-side only.

You also get a stderr warning listing every `@default(dbgenerated())` on a
required column, because the emitter cannot verify the database actually has that
default.

## `migrate baseline` — adopting an existing database

Postgres only, enforced at the type level. Sequence:

1. **Refuses immediately** if `<out-dir>/postgres/schema.snapshot.json` already
   exists — before parsing, before connecting.
2. Introspects the live database into `Projections`.
3. Diffs introspected against authored and prints a drift report grouped by table,
   plus unmapped columns.
4. `--strict` plus any drift → bail; no snapshot, no baseline row.
5. Writes the snapshot **from the introspected shape**, not the authored one.
6. Records a synthetic `<timestamp>_baseline` row in `cratestack_migrations` whose
   `up` is pure SQL comments carrying a checksum of the introspected shape. It
   applies no DDL, but the checksum makes a re-baseline against a since-drifted
   database detectable.

Both writes matter: without the history row, `apply_pending()` would try to
recreate existing tables.

Introspection is knowingly lossy. Numeric precision is ambiguous coming back, and
**validator-derived CHECKs are impossible in principle** to reverse-map —
`@range` / `@length` / `@iso4217` compile to raw CHECK SQL with no way back. The
design treats this as **reported drift, not attempted reconciliation**.

## Applying

**Not in the CLI.** The applier lives in `cratestack-sqlx` and is **Postgres
only**.

Table `cratestack_migrations (id, description, checksum, applied_at)`.
`status()` reports `Pending` / `Applied` / `ChecksumMismatch` per migration.
`apply_pending()` **aborts the entire apply on any checksum mismatch before
applying anything**, then applies each pending migration in its own transaction
— `up_pre` then `up`, via `raw_sql` (the simple-query protocol, so dollar-quoted
PL/pgSQL survives) — inserts the history row, and commits.

`Migration.down` is recorded and **never executed**. There is no rollback
function anywhere: irreversible-by-default is the deliberate banking posture.

Nothing in the repo wires migration-directory reading into the runner — that glue
is yours. `cratestack-service` exposes `migrations_from_dir` and `run_migrations`
under its default-on `postgres` feature.

## SQLite

| | Postgres | SQLite |
| --- | --- | --- |
| Column types | real mapping (`String→TEXT`, `Int→BIGINT`, `Uuid→UUID`, …) | **every column is `BLOB`** |
| Enums | `TEXT` + `CHECK (col IN (…))`, deliberately not a native enum type | not emitted |
| Extensions | `EnsureExtension` DDL | n/a |
| `up.pre.sql` | scaffolded when blocking | **never** — guidance goes into `up.sql`, because SQLite has no runner to read a pre file |
| `DROP COLUMN` | native | native (SQLite ≥ 3.35) — no table-rebuild dance |
| Introspection / `baseline` | supported | **not supported** |
| Runner | `cratestack-sqlx` | **none exists** |

BLOB everywhere is because BLOB affinity is the only one that preserves the bound
storage class. Native enums are avoided because generated row decoders read enum
fields as `String`.

On device, the usual pattern is not migrations at all: call
`create_table_sql(&DESCRIPTOR)` (a `CREATE TABLE IF NOT EXISTS`) at app start.
See `cratestack-embedded`.

## Schema-diff as a companion gate

`cratestack diff <old> <new>` answers a different question — *is the wire
contract broken* — and is worth running alongside `migrate diff` on schema PRs.
See `cratestack-cli`.
