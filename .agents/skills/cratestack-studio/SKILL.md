---
name: cratestack-studio
description: CrateStack Studio — the admin and testing surface over one or more .cstack schemas. Covers studio.toml configuration, studio init/run/eject, the read and write HTTP API, the SQL-preview, drift, export and audit tools, the Leptos UI bundling requirement, and the security posture of rw database targets. Load when configuring studio.toml, running or ejecting Studio, or asking whether Studio enforces policy.
---

# Studio

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [cratestack/references/version-history.md](../cratestack/references/version-history.md).

An admin and testing surface over your schemas. Describe the workspace once and
the shipped binary serves the UI.

```bash
cratestack studio init            # writes ./studio.toml
cratestack studio run             # binds 127.0.0.1:7878
cratestack studio eject --out ./my-studio [--with-ui] [--name …] [--force]
```

`init` takes `--out` (default `.`) and `--force`; it always writes a file named
`studio.toml`. `run` takes `--config` (default `studio.toml`) and `--bind`
(default `127.0.0.1:7878`, parsed as a `SocketAddr`). `eject` requires `--out`;
`--name` defaults to the `--out` basename.

## Read this before enabling a write target

**`mode = "rw"` on a `[target.db]` target is database-level access.** Studio
evaluates **no policy** and consults **no suppression metadata**:
`@@allow` / `@@deny` — reads *and* writes — and `@@internal(...)` are **not
enforced at all**. The compensating control is the audit log, which is
**detective, not preventive**.

`rw` on an `[target.api]`-only target is different and much narrower: it grants
no more than the credential already has, because the deployed service enforces
everything.

Two related facts:

- `@version` **is** bumped for real on every driver.
- `@@emit(...)` writes a real `cratestack_event_outbox` row **only on Postgres**.
  On SQLite that is a permanent capability difference, and a write returns
  `403 UNSAFE_DB_WRITE` unless the target sets `allow_unsafe_writes = true`.

## `studio.toml`

```toml
[workspace]
name          = "studio"        # default "studio"
default_mode  = "ro"            # "ro" | "rw", default "ro"
cors_dev      = true            # DEFAULT IS true
audit_file    = "./studio-audit.jsonl"   # optional

[[target]]
key          = "main"           # required; [A-Za-z0-9_-], non-empty
display_name = "Main"           # optional
schema       = "./schema.cstack"
mode         = "rw"             # optional; falls back to workspace.default_mode

  [target.db]                   # at least one of db/api is required
  url              = "env:DATABASE_URL"   # literal, env:NAME, or file:PATH
  driver           = "postgres"           # postgres | sqlite | mysql
  max_connections  = 10
  allow_unsafe_writes = false

  [target.api]
  base_url   = "https://api.example.com"
  auth       = { kind = "bearer", token = "env:TOKEN" }
  # or       = { kind = "header", name = "X-Key", value = "…" }
  prefer_for = ["Widget"]
```

**`cors_dev` defaults to `true`** because Studio binds 127.0.0.1 and a Trunk dev
server on :8080 must reach :7878. Set it false when binding wider.

**`audit_file` is append-only and never rotated or truncated.** Unset means
in-memory only.

**Channel precedence is the load-bearing semantic:** the loader takes
`[target.db]` whenever it exists and falls back to `[target.api]` only when there
is no `[target.db]` block at all. A target declaring **both** is a database
target for every read and write — adding `[target.api]` alongside buys **no
enforcement**.

**`prefer_for` is parsed and consulted by nothing.** It does not redirect writes.

Any secret-bearing string accepts `env:NAME` or `file:PATH` (trimmed).

## The HTTP surface

Eleven read routes and four write verbs. `/api/*` is mounted before the UI, so it
always wins over SPA routes.

```
GET    /api/health
GET    /api/targets
GET    /api/targets/{key}/schema
GET    /api/targets/{key}/models
GET    /api/targets/{key}/models/{model}/records
POST   /api/targets/{key}/models/{model}/records
GET    /api/targets/{key}/models/{model}/records/{pk}
PATCH  /api/targets/{key}/models/{model}/records/{pk}
DELETE /api/targets/{key}/models/{model}/records/{pk}
GET    /api/targets/{key}/models/{model}/records/{pk}/rel/{field}
GET    /api/targets/{key}/models/{model}/snippet?pk=…
GET    /api/targets/{key}/models/{model}/sql?op=…&pk=…&explain=…
GET    /api/targets/{key}/models/{model}/export?format=csv|json&limit=N
GET    /api/targets/{key}/drift
GET    /api/targets/{key}/search?q=…
GET    /api/audit?limit=N
```

Errors are `{"error": {"code": "…", "message": "…"}}` with codes like
`UNKNOWN_TARGET`, `UNKNOWN_MODEL`, `NO_PRIMARY_KEY`, `FORBIDDEN`,
`UNSAFE_DB_WRITE`, `DATABASE_ERROR`.

## The tools

**SQL preview** — `?op=list|get|create|update|delete` renders the SQL and bound
parameters **without touching the database**. `explain=true` is the only part
that reaches it: it *plans* the statement, never executes, has no
`EXPLAIN ANALYZE` path, and **refuses mutations outright**. API-backed targets
get `501`.

**Drift** — a per-model column comparison against the driver catalog
(`information_schema` on Postgres, `PRAGMA table_info` on SQLite). It is a
**shallow name-set comparison**: it does not check types precisely, defaults,
primary keys, indexes, CHECKs, enums or views. It is **not** the same engine as
`migrate baseline`'s introspection.

**Export** — CSV or JSON, cursor-paginated internally, hard-capped. Intended for
"pull a sample for a notebook", not ETL.

**Search** — case-insensitive substring over models, fields, types, enums and
variants, mixins and procedures.

**Snippet** — renders a Rust `find_unique` snippet for a (model, pk) pair.

**Audit** — an in-memory ring buffer, optionally persisted to append-only JSONL
and replayed on boot.

## Ejecting

Default eject writes a self-contained binary crate:

```
<out>/{Cargo.toml, README.md, studio.toml, schemas/example.cstack, src/main.rs}
```

`--with-ui` additionally unpacks the Leptos UI sources into `<out>/ui/`. It
refuses a non-empty `--out` without `--force`.

The ejected UI needs `cargo install trunk` and
`rustup target add wasm32-unknown-unknown`. The dev loop is two terminals —
`cratestack studio run` on :7878 and `trunk serve` on :8080, with `Trunk.toml`
proxying `/api/*` so the browser sees one origin.

**An eject is a point-in-time snapshot with no automated upgrade path.**

## Building the bundled UI

`cratestack-studio-ui` is a Trunk-built wasm32 Leptos app and **its own
workspace**, excluded from the repo workspace so nobody is forced onto the wasm
toolchain. `just bundle-studio-ui` hard-fails if `trunk` is missing and produces
**two** gitignored tarballs: `embedded-ui.tar.gz` (sources, feeding
`eject --with-ui`) and `embedded-ui-dist.tar.gz` (the built dist, feeding the
`embed-ui` feature).

`embed-ui` is a default feature and compiles even when Trunk has never run — an
empty bundle degrades to a placeholder page with a warning rather than a build
failure. Which is convenient and also means a stale bundle is easy to miss.

`just publish-studio` refuses to publish if anything besides the two tarballs is
dirty.
