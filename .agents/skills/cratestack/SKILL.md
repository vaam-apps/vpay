---
name: cratestack
description: Entry point and router for CrateStack, the Rust schema-first framework built on .cstack schema files. Load this whenever a project contains a .cstack file, a Cargo dependency renamed with package = "cratestack-pg" / "cratestack-api" / "cratestack-sqlite" / "cratestack-client", a call to include_server_schema! / include_embedded_schema! / include_client_schema!, or the cratestack CLI. It explains the role model, picks the right facade for the crate you are in, and points at the specific cratestack-* skill for the task.
---

# CrateStack

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [references/version-history.md](references/version-history.md).

A Rust-native, schema-first framework. You write one `.cstack` schema; a
**procedural macro reads it at compile time** and emits the typed Rust surface
for whatever the consuming crate is. No runtime reflection, no build script, no
generated Rust files checked in. Change the schema and the compiler tells you
what broke.

Pre-1.0. The public crates version together (currently **0.12.0**, edition 2024,
MSRV **1.98.0**) and minor releases can break.

## First: which crate am I in?

Everything else follows from this. The framework ships **four facades**, all of
which expose their library as `cratestack`, so you select one with Cargo's
`package =` rename and the `use` statements read the same either way.

| The crate you are writing | Dependency | Entry macro |
| --- | --- | --- |
| HTTP service that owns a Postgres database | `cratestack = { package = "cratestack-pg", version = "0.12" }` | `include_server_schema!("schema.cstack", db = Postgres)` |
| Procedures-only service, no database at all | `cratestack = { package = "cratestack-api", version = "0.12" }` | `include_server_schema!("schema.cstack", db = None)` |
| Mobile / desktop / browser app with local SQLite | `cratestack = { package = "cratestack-sqlite", version = "0.12" }` | `include_embedded_schema!("schema.cstack")` |
| A crate that only *calls* a CrateStack service | `cratestack = { package = "cratestack-client", version = "0.12" }` | `include_client_schema!("../schemas/billing.cstack")` |

**Exactly one per crate.** The four are strictly disjoint by design, and the
split is enforced by a CI job (`facade-disjointness`) running a real `cargo tree`,
not just documented:

- `cratestack-pg` does not pull `libsqlite3-sys`, so it coexists with the
  official `sqlx` umbrella without `links = "sqlite3"` collisions.
- `cratestack-api` has no `cratestack-sqlx` dependency under any feature gate.
- `cratestack-client` has no `cratestack-axum`, and therefore no `axum`,
  `tower`, `hyper` or `tower-http`, in its graph under default features.

If you find yourself wanting two of these in one crate, you want two crates.

## Where to go next

| You are doing this | Load |
| --- | --- |
| Writing or editing a `.cstack` file | `cratestack-schema` |
| Building the Postgres or no-database server | `cratestack-server` |
| `@@allow` / `@@deny`, auth providers, identity | `cratestack-policy-auth` |
| Idempotency, optimistic locking, audit, soft delete, money-shaped data | `cratestack-data-integrity` |
| Local SQLite: mobile, desktop, wasm/OPFS, Flutter, Expo, Tauri | `cratestack-embedded` |
| Generating or consuming Rust / Dart / TypeScript clients | `cratestack-clients` |
| `transport rpc`, `/rpc/batch`, streaming, batching links | `cratestack-rpc` |
| Migrations, schema diff, baselines | `cratestack-migrations` |
| The `cratestack` CLI, any subcommand or flag | `cratestack-cli` |
| Studio (admin/testing surface) | `cratestack-studio` |
| LSP, VS Code, Neovim/Helix/Zed, tree-sitter grammar | `cratestack-editor-tooling` |
| A build, test or macro error you do not understand | `cratestack-troubleshooting` |
| Contributing to the framework repo itself | `cratestack-contributing` |

## The shape of the thing

```cstack
model Post {
  id        Int      @id
  title     String
  published Boolean  @default(false)
  authorId  Int

  author    User?    @relation(fields: [authorId], references: [id])

  @@allow("read",   published == true)
  @@allow("create", auth() != null)
  @@allow("update", auth().role == "admin")
}

procedure getFeed(args: FeedArgs): Post[]
```

```rust
use cratestack::include_server_schema;

include_server_schema!("schema.cstack", db = Postgres);
```

That is the whole integration. `cratestack_schema` now exists, fully typed.

## Four facts that change how you write code here

**1. Codegen is compile-time, so the compiler is the contract.** There is no
generated file to read, and no `cargo generate` step to re-run. When you change
the schema, `cargo check` is the feedback loop. When you want to see what the
macro produced, expand it (`cargo expand`) rather than guessing.

**2. Policy lives on the model, and is enforced server-side only.** `@@allow` /
`@@deny` are enforced by generated server code on models, views and procedures.
The embedded (SQLite) backend **parses policies and does not enforce them**.
That is a deliberate decision, not a gap: clients are untrusted and
authorization is the server's job. Never treat an embedded build as an
enforcement boundary.

**3. REST and RPC are per-schema exclusive, and ship together.** A schema
declares REST routes (the default) or `transport rpc`. Any feature touching the
request/response surface lands on **both** transports in the same change — the
project treats a REST-only feature as a defect, not a phase. See
`cratestack-rpc`.

**4. The three macros are disjoint on purpose.** `include_server_schema!` emits
sqlx-only (or axum-only, for `db = None`) code; `include_embedded_schema!` emits
rusqlite-only code; `include_client_schema!` emits axum-free client code. No
cross-backend impl leaks between them. If you are tempted to "share" a runtime
type across two of these paths, that is the boundary talking.

## Known limits, current as of 0.12.0

Worth knowing before you design around them:

- `db = Postgres` is the only sqlx backend. The parser is wired so adding
  others is non-breaking at existing call sites.
- Generated routers enforce a **single configured codec** rather than
  negotiating per request. `application/cbor-seq` is a documented target, not an
  implementation.
- The embedded backend does not enforce `@@allow` / `@@deny` (see above).
- Exact typed non-Rust client generation across arbitrary projection shapes is
  still stabilizing.
- Runtime custom-field resolution beyond the generated trait metadata is not
  supported.

## Sources

- Framework: <https://github.com/cratestack/cratestack>
- Documentation for humans: <https://cratestack.dev>
- These skills: <https://github.com/cratestack/cratestack-skills>

When a skill and the running source disagree, **the source wins** — and the
disagreement is a bug in this repo worth reporting.
