---
name: cratestack-schema
description: Authoring and debugging .cstack schema files for CrateStack — models, fields, relations, enums, types, mixins, views, declarative query blocks, procedures, attributes and validators. Load whenever writing or editing a .cstack file, when cratestack check reports an error, or when a question mentions @id, @relation, @@allow, @@index, @computed, @@sql, transport rpc, or any other .cstack attribute.
---

# Authoring `.cstack`

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [cratestack/references/version-history.md](../cratestack/references/version-history.md).

One file, one schema. **There is no `import` and no `part`** — multi-service
setups keep separate `.cstack` files and separate macro invocations.

Validate with `cratestack check --schema schema.cstack`. Errors are precise and
worth reading literally; most of this skill is about the ones that are not
obvious.

## Top-level declarations — the complete list

```
datasource <name> { provider = "postgresql" | "sqlite" | "none", url = env("…") }
auth <Name> { fields }              // the shape auth() resolves against
mixin <Name> { fields }             // pulled into a model with @use(Name)
model <Name> { fields + @@attrs }   // a table
type <Name> { fields }              // a plain struct: procedure args, results, query rows
enum <Name> { VARIANTS }            // one bare identifier per line
extension rate_limit | pgvector | postgis { }
transport rest | rpc                // at most once; default rest
procedure <name>(a: T): R           // attribute lines follow, no braces
mutation procedure <name>(a: T): R
query <name>(a: T): ResultType      // declarative SQL, Postgres-only
view <Name> from <M>, <M> { … }
```

Anything else is `unsupported top-level declaration`. **`config` and `version`
are not keywords** — `@version` is a *field* attribute. `transport grpc` is
rejected with a migration message; gRPC was removed in 0.8.5.

## The 16 built-in types, and the two you will reach for that do not exist

```
String  Cuid  Int  Float  Boolean  DateTime  Decimal  Json  Bytes  Uuid
Page  PageInput  FindMany  Vector  Geography  Geometry
```

**`BigInt` is not a `.cstack` type** — `Int` already maps to `i64` / `BIGINT`.
**`Date` is not one either** (the docs site shows it in one snippet; that snippet
would not parse). There is no `@db.JsonB`; use the `Json` scalar.

Arity suffixes go at the end: `T` required, `T?` optional, `T[]` list. On a
schema with a `datasource`, a **list-arity scalar or enum field is rejected** —
there is no SQL bind representation for it. Model it as a relation, or drop the
datasource block if the schema is client-only.

`Page<T>` is return-position only; `PageInput` and `FindMany<T>` are
argument-position only; `Vector(n)` and the spatial types *(PostGIS since 0.10.0)* are field-position
only and each requires its `extension` block to be declared.

## A schema that exercises the common shapes

```cstack
datasource db {
  provider = "postgresql"
  url = env("DATABASE_URL")
}

auth Principal {
  id   Int
  role String?
}

mixin AuditFields {
  createdAt DateTime @default(dbgenerated())
  updatedAt DateTime @default(dbgenerated())
}

model Article {
  @use(AuditFields)

  id        Int     @id
  title     String  @length(min: 1, max: 200)
  body      String
  published Boolean @default(false)
  authorId  Int

  author    User    @relation(fields: [authorId], references: [id], onDelete: Cascade)

  @@allow("read",   auth() != null)
  @@allow("update", auth().id == authorId)
}

model User {
  id    Int      @id
  email String   @email @unique
  posts Article[] @relation(fields: [id], references: [authorId])
}
```

Note the has-many side carries `@relation` too. That is mandatory.

## Attributes you will use most

| | |
| --- | --- |
| `@id` | Primary key. **Exactly one per model** — a second is now a hard error. |
| `@default(…)` | `@default(false)`, `@default(dbgenerated())`, `@default(auth().organization.id)`. `dbgenerated()` takes **no argument**. |
| `@unique` | Single-column unique index. |
| `@relation(fields: [local], references: [target], onDelete: X, onUpdate: X)` | See below. |
| `@readonly` / `@server_only` | Out of inputs; `@server_only` also strips from responses. Mutually exclusive, and neither may sit on the PK. |
| `@pii` / `@sensitive` | Audit redaction only; no effect on inputs or outputs. |
| `@version` | Optimistic locking. Required `Int`, at most one, never the PK. |
| `@computed` / `@computed(params: T?)` | Resolver-backed response field. |
| `@length` `@range` `@regex` `@email` `@uri` `@iso4217` | Validators — see below. |
| `@@allow("action", expr)` / `@@deny(…)` | Model policy. Actions: `list`, `detail`, `read`, `create`, `update`, `delete`, `all`. |
| `@@id([a,b])` `@@unique([a,b])` `@@index([a,b], using: gist, opclass: "…", where: "…")` | Model-level keys and indexes. |
| `@@paged` `@@audit` `@@soft_delete` `@@retain(days: N)` `@@emit(created, updated, deleted)` `@@subscribe` `@@internal("action")` *(since 0.8.14)* | Behaviour markers. |

Exhaustive argument grammars, every error message, and the full validator table:
[references/attributes.md](references/attributes.md).

## Relations — the five rules that generate most errors

1. **Both sides must declare `@relation`**, including the `Model[]` has-many
   side. A model-typed field without it is an error.
2. **Exactly one local field and one reference.** Composite foreign keys are not
   supported.
3. **The two scalar types must match exactly.** `Int` ↔ `Int`, not `Int` ↔ `BigInt`.
4. `onDelete` / `onUpdate` may only appear on the **owning** side — the field
   typed as a single model. The has-many side owns no column.
5. Actions are **bare identifiers, not strings**: `Cascade`, `Restrict`,
   `SetNull`, `SetDefault`, `NoAction`. `SetNull` needs the local field optional;
   `SetDefault` needs it to carry `@default(…)`.

## Attribute traps that produce confusing errors

**No space before an argument list.** `@computed (params: X?)` is rejected;
write `@computed(params: X?)`.

**Unknown attributes are inert, near-misses are rejected** *(since 0.8.15)*. `@reedonly` is caught
as a typo of `@readonly`; a genuinely unknown `@whatever` parses, reports
`schema OK`, and enforces nothing. Do not assume silence means it worked.

**`@allow` at field position is rejected.** Field-level access policy never
existed. Use model-level `@@allow("read", …)`, or `@readonly` / `@server_only`.
`@custom` was renamed to `@computed` *(0.8.11)*; `@pb` was removed with gRPC *(0.8.5)*.

**`using:` is a bare identifier, `opclass:` is a quoted string.**
`@@index([embedding], using: ivfflat, opclass: "vector_l2_ops")`. The `where:`
argument on `@@unique` / `@@index` is *since 0.8.14*. The docs site
shows `using: "gist"` with quotes — that is wrong and will be rejected.

**`@@id([...])` parses and then fails at macro expansion.** `cratestack check`
validates composite primary keys, and all three entry macros reject them
outright. Treat composite PKs as not yet usable, whatever `check` says.

**`@@map` does not exist.** The table name is always
`pluralize(snake_case(ModelName))`.

**Unrecognised `@@` model attributes are silently inert** — the near-miss check
only runs on field attributes.

## Views and `query` blocks

A `view` projects over one or more models and needs a SQL body:

```cstack
view ActiveNote from Note {
  id    Int    @id @from(Note.id)
  title String @from(Note.title)

  @@embedded_sql("SELECT id, title FROM notes WHERE archived = 0")
}
```

`@@server_sql` for Postgres, `@@embedded_sql` for SQLite, `@@sql` for both. A
view needs **exactly one `@id` field** unless it declares `@@no_unique`.
`@@materialized` requires a server body and cannot combine with `@@no_unique`.
`@@allow` on a view supports only `"read"`. **`@from(...)` is completely inert** —
documentation of provenance, checked by nothing.

A `query` block *(since 0.11.0)* is raw Postgres SQL with positional parameters:

```cstack
query loyaltyFeeSummary(userId: String, cutoff: DateTime): LoyaltyFeeSummary
  @@sql("""
    SELECT COALESCE(SUM(discount), 0)::bigint AS "total"
    FROM loyalty_fee_events
    WHERE user_id = $1 AND created_at >= $2
  """)
  @allow(auth() != null && auth().subjectId == userId)
```

Rules that bite: the result type must be a **`type` block**, never a model (raw
SQL gets none of a model read's soft-delete or policy filtering). Parameters must
be **required scalars** from a restricted set (`String`, `Cuid`, `Int`, `Float`,
`Boolean`, `DateTime`, `Uuid`, `Bytes` — no `Decimal`, no `Json`). Every declared
parameter must be referenced by its `$N`, and every `$N` must exist — checked in
both directions. A `query` understands only `@@sql`, `@allow`, `@deny`, and is
rejected entirely under `provider = "none"`.

**Prefer `"""…"""` over `"…"` for SQL bodies.** Triple-quoted is verbatim;
single-quoted unescapes `\"` and `\\`. And keep every other attribute on its own
line — the extractor reads everything up to the **last** `)` on the line as the
SQL argument.

The correspondence between the declared result `type`'s fields and the SQL's
actual `SELECT` list is deliberately **not** checked. A mismatch surfaces at the
first execution as `sqlx::Error::ColumnNotFound`.

## Procedures

```cstack
mutation procedure reindex(args: ReindexArgs): ReindexResult
  @allow(auth() != null)
  @no_rate_limit
```

Attribute lines follow the declaration, unbraced. `procedure` is a query,
`mutation procedure` is a mutation. Per-argument documentation comes from
`/// @param <name> <description>` doc comments above the declaration.

| Attribute | Notes |
| --- | --- |
| `@allow(expr)` / `@deny(expr)` | Single `@`, unlike models. Evaluated in Rust, not SQL. |
| `@stream` | Bare only. **Requires a list return type.** |
| `@no_idempotency` | Bare only. No extension gate. |
| `@no_rate_limit` | Bare only. **Requires `extension rate_limit { }`.** |
| `@isolation("serializable")` | Quoted level. |
| `@api_version("v1")` | Alphanumeric plus `.`, `-`, `_`. Mounts at `/<version>/$procs/<name>`. |
| `@deprecated` / `@deprecated("msg")` | Adds `Deprecation` response headers. |
| `@status(202)` | 2xx only, and **rejected under `transport rpc`**. |
| `@authorize(Model, action, args.path)` | `detail`/`read`, `update`, `delete` only. |

`@status(204)` is accepted but the encoder still attaches a body — a known,
documented limitation. Do not use it.

## `@computed` — the restrictions are the whole story

`@computed` may only appear on a `type` or a `model` field, may appear once, and
**may not be combined with any other field attribute** — a computed field is
resolved at response-composition time, never stored and never accepted as input,
so no other attribute applies to it. The `params` form requires the trailing `?`:
`@computed(params: T?)`, where `T` must be a declared `type` block.

Beyond the field itself: a computed field's own type may not be a model and may
not itself contain computed fields; a procedure **argument** may not reference a
computed-bearing type (even through `Page<T>` / `FindMany<T>`); a `@stream`
procedure may not return computed-bearing items; and computed fields can never
appear in `@@id`, `@@unique` or `@@index`.

## The collision rules — where surprising errors come from

Nine independent families, all reported by `cratestack check`. The ones that
actually catch people:

- **`self`, `Self`, `super`, `crate`** are unusable as any identifier. Every
  *other* Rust keyword is fine (escaped as `r#type` at codegen).
- **snake_case collisions**: `myField` and `my_field` on one model both normalize
  to the same column. So do `model Foo` and `model foo`.
- **Route collisions**: `model Bus` and `model Buse` both route to `/buses`.
- **Builder collisions**: a `model Task` reserves `TaskBuilder`,
  `CreateTaskInputBuilder`, `TaskWhereBuilder` and four more — declaring a type
  by one of those names is rejected.
- **Handler collisions**: `model Order` plus `procedure getOrder` both generate
  `handle_get_order`. `@@internal(...)` does **not** exempt you — the handler
  functions are still emitted.
- **Client method collisions**: `model Procedure` collides with the `Client`
  built-in `procedures`; `procedure new` collides with `ProceduresClient::new`.

## `transport rpc` — what changes at schema level

Exactly two things the parser cares about: `@@subscribe` on a model **requires**
`transport rpc`, and `@status(...)` on a procedure is **rejected** under it.
Everything else about RPC is a codegen concern — see `cratestack-rpc`.

## Deeper references

- [references/attributes.md](references/attributes.md) — every attribute, exact
  argument grammar, and the verbatim error text.
- `cratestack-policy-auth` — the `@@allow` / `@@deny` expression language.
- `cratestack-data-integrity` — what `@@audit`, `@@soft_delete`, `@version`,
  `@@paged` and idempotency actually do at runtime.
