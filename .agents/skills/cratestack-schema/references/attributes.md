# `.cstack` attribute reference

Exhaustive argument grammars and the rules the validator actually enforces.
When this file and `cratestack check` disagree, the checker is right — report it.

## Field attributes

| Attribute | Grammar | Rules |
| --- | --- | --- |
| `@id` | bare | Exactly one per model. Rejected on `mixin` fields. A model needs `@id` or `@@id([...])`. |
| `@default(expr)` | one expression | Free-form. `dbgenerated()` takes **no argument**. `@default(auth().path)` pulls from the auth context and supports nested paths. Any `@default` makes the field generated-on-create. |
| `@unique` | bare or `@unique(...)` | Parser does not validate it; `cratestack-migrate` emits the constraint. |
| `@relation(...)` | `fields: [x], references: [y], onDelete: A, onUpdate: A` | See the SKILL. Unknown key → `unsupported @relation key`. Both `fields` and `references` are mandatory. |
| `@readonly` | bare | Out of Create + Update inputs; visible in responses and audit snapshots. |
| `@server_only` | bare | Out of inputs, stripped from responses, omitted from audit snapshots. |
| `@pii` | bare | Audit redaction as `"[redacted-pii]"`. No effect on inputs or outputs. |
| `@sensitive` | bare | Audit redaction as `"[redacted-sensitive]"`. |
| `@version` | bare | Required `Int`. At most one per model. Never the PK, never inside `@@id([...])`. |
| `@computed` | bare or `@computed(params: T?)` | `type` and `model` only. Once per field. **Cannot combine with any other field attribute.** The `?` on `params` is mandatory; `T` must be a declared `type` block. |
| `@from(Model.field)` | opaque | **Completely inert.** Provenance documentation on view fields, checked by nothing. |
| `@db_enforce` | bare | Alongside a validator, promotes it to a real Postgres `CHECK`. Not validated by the parser. |
| `@length` `@range` `@regex` `@email` `@uri` `@iso4217` | see below | Validators. |

### Mutual exclusions and position rules

- `@readonly` + `@server_only` together → rejected; use `@server_only` alone.
- Either of them on the primary key → rejected.
- `@default(dbgenerated(anything))` → rejected; the marker takes no argument.

### Rejected at field position

| Written | Why |
| --- | --- |
| `@custom` | Renamed to `@computed`. |
| `@pb` | protobuf/gRPC removed in 0.8.5. |
| `@allow` / `@deny` | Field-level access policy never existed and enforced nothing. Use model-level `@@allow`, or `@readonly` / `@server_only`. |

The single-`@` forms **are** valid on `procedure` and `query` declarations. Only
field position rejects them.

## Validators

| Attribute | Arguments | Valid on | Notes |
| --- | --- | --- | --- |
| `@length(min: N, max: N)` | both optional, non-negative | `String`, `Bytes` | `min <= max` enforced. |
| `@range(min: N, max: N)` | both optional, signed | `Int`, `Decimal` | `min <= max` enforced. |
| `@regex("pattern")` | one quoted string | `String` | **Compiled at parse time** — an invalid regex fails `cratestack check`. |
| `@email` | bare | `String` | |
| `@uri` | bare | `String` | |
| `@iso4217` | bare | `String` | |

**Validator type-checking runs only on `model` fields.** A validator on a `type`,
`mixin`, `view` or `auth` field is not type-checked (it still passes the
near-miss and removed-attribute checks). Runtime enforcement lives in generated
`validate()` impls on the Create/Update inputs — and on the **embedded** backend
nothing calls them automatically.

## Model attributes

| Attribute | Grammar | Rules |
| --- | --- | --- |
| `@@allow("action", expr)` | quoted action + expression | Actions: `list`, `detail`, `read`, `create`, `update`, `delete`, `all`. `read` groups `list` + `detail`. Validated at macro time, not by the parser. |
| `@@deny("action", expr)` | same | Only `create`, `update`, `delete`, `list`/`read`, `detail`/`read` are wired. |
| `@@id([a, b])` | ≥2 fields | Real scalar fields, no repeats. Mutually exclusive with field-level `@id`. Listed fields must not carry `@readonly` / `@server_only` / `@version`. **Parses, then hard-rejected by every entry macro.** |
| `@@unique([a, b], where: "…")` | ≥1 field; ≥2 unless `where:` present | Only key accepted is `where`, at most once, value must be a quoted string. Duplicate field lists rejected. |
| `@@index([a], using: gist, opclass: "…", where: "…")` | ≥1 field | `using:` is a **bare identifier**; `opclass:` is a **quoted string**; `where:` is quoted SQL passed through verbatim. Duplicate `(fields, using)` pairs rejected — same fields with a different `using` is legal. |
| `@@paged` | **bare only** | Changes the list return type to `Page<Model>`. |
| `@@audit` | **bare only** | |
| `@@soft_delete` | **bare only** | |
| `@@retain(days: N)` | `days:` + non-negative integer | |
| `@@emit(created, updated, deleted)` | ≥1 of exactly those three | At most one per model; duplicates deduped. |
| `@@subscribe` | **bare only** | Requires `transport rpc` **and** requires `@@emit(...)`. |
| `@@internal("action")` | **exactly one quoted action per declaration** | Vocabulary: `list`, `detail`, `read`, `create`, `update`, `delete`, `all`. Suppresses the REST route, the RPC dispatch arm and the client stub. Does **not** suppress policy evaluation, and does not exempt handler-name collisions. |
| `@use(MixinA, MixinB)` | comma list | Written inside the model body. Expanded at parse time; the model's own field wins on a name clash. |

`@@map` does not exist. Table names are always `pluralize(snake_case(Name))`.

**Unrecognised `@@` attributes are silently inert.** The near-miss typo check
runs on field attributes only, so `@@sofT_delete` enforces nothing and reports
`schema OK`.

## `view` attributes

| Attribute | Notes |
| --- | --- |
| `@@server_sql("…")` | Postgres body. |
| `@@embedded_sql("…")` | SQLite body. |
| `@@sql("…")` | Both — used as the fallback for each. |
| `@@materialized` | Bare. Requires a server body. Cannot combine with `@@no_unique`. |
| `@@no_unique` | Bare. Opts out of the "exactly one `@id` field" rule. |
| `@@allow("read", expr)` | **Only `read`** is supported on a view. |

## `query` attributes

Only three are recognised: `@@sql`, `@allow`, `@deny`. `@@server_sql` and
`@@embedded_sql` are specifically rejected — a query has no per-backend split.
Exactly one `@@sql` body is required.

## Procedure attributes

| Attribute | Grammar | Rules |
| --- | --- | --- |
| `@allow(expr)` / `@deny(expr)` | single `@` | Evaluated in Rust against the decoded args and the auth context. |
| `@stream` | bare only | Requires a list (`T[]`) return type. `@stream(...)` is silently inert. Rejected when the item type contains `@computed` fields. |
| `@no_idempotency` | bare only | `@no_idempotency(false)` is rejected — it would read as re-enabling while doing the opposite. No extension gate. |
| `@no_rate_limit` | bare only | Requires `extension rate_limit { }`. |
| `@isolation("serializable")` | quoted | |
| `@api_version("v1")` | quoted, `[A-Za-z0-9._-]` | Mounts at `/<version>/$procs/<name>`. |
| `@deprecated` / `@deprecated("msg")` | bare or one quoted string | Adds `Deprecation: true` and `X-Deprecation: <msg>` response headers. |
| `@status(202)` | bare integer, 200..=299 | **Rejected under `transport rpc`.** `@status(204)` is accepted but the encoder still attaches a body — do not use it. |
| `@authorize(Model, action, args.path)` | three parts | Actions `detail`/`read`, `update`, `delete` only. The path's type must match the model's PK type. Parsed at macro time. |

## Parametric types

`Vector(n)` — requires `extension pgvector { }`, exactly one integer dimension
greater than zero, never list-valued, field position only.

`Geography` / `Geometry` — require `extension postgis { }`, never list-valued, at
most one integer SRID and at most one bareword subtype, field position only. An
SRID without a subtype is rejected because PostGIS's modifier is positional:
write `Geography(Point, 4326)`, not `Geography(4326)`. Subtype matching is
case-insensitive; the vocabulary is the 16 geometry bases each optionally
suffixed `Z`, `M`, or `ZM`.

## Multi-line SQL

Only `@@server_sql`, `@@embedded_sql` and `@@sql` may span physical lines, and
only via `"""…"""`. The body ends at the first following line containing `""")`.

Escaping is asymmetric: `"""…"""` is verbatim; `"…"` unescapes `\"` and `\\`.
Prefer triple quotes.

The extractor reads everything up to the **last** `)` on the line as the SQL
argument, so a second attribute on that same physical line silently corrupts the
body. Keep each attribute on its own line.

## Doc comments

`///` is a doc comment; `//` is an ordinary comment and **clears** pending docs,
as does a blank line. Per-argument procedure docs use
`/// @param <name> <description>`.
