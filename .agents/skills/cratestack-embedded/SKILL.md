---
name: cratestack-embedded
description: Building CrateStack's embedded offline-first mode — include_embedded_schema!, RusqliteRuntime, ModelDelegate, on-device SQLite for mobile, desktop, and the browser via wasm32 + OPFS. Load when the crate depends on package = "cratestack-sqlite", calls include_embedded_schema!, or when the task involves Flutter/flutter_rust_bridge, React Native/Expo, Tauri, sqlite-wasm-rs, OPFS persistence, or "why is my policy not enforced on device".
---

# Embedded mode (SQLite on device)

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [cratestack/references/version-history.md](../cratestack/references/version-history.md).

```toml
cratestack = { package = "cratestack-sqlite", version = "0.12" }
```

```rust
use cratestack::include_embedded_schema;
include_embedded_schema!("schema.cstack");
```

Sync API. `rusqlite` with **bundled** SQLite, no tokio on the data path. Compiles
to native *and* `wasm32-unknown-unknown` from the same source.

## Read this before you design anything

**Policies are not enforced here.** `@@allow` / `@@deny` still parse, still fail
compilation when malformed, and their slots are still compiled into the model
descriptor — but nothing in `cratestack-rusqlite` ever reads them. This is a
design decision, not a gap: clients are untrusted and authorization is the
server's job. An embedded build is never an enforcement boundary.

**Validators are generated but not automatically run.** `.validate()` exists on
`Create*Input` / `Update*Input`, and `CreateRecord::run` / `UpsertRecord::run`
deliberately do not call it (matching each other on purpose). If you want
validation on device, call `.validate()?` yourself. The one exception that does
run automatically is `ConflictTarget::validate` on upsert.

**There is no migration runner on device.** `create_table_sql(&DESCRIPTOR)` emits
`CREATE TABLE IF NOT EXISTS` and apps run it at startup. The CLI *can* emit
SQLite migration SQL (`cratestack migrate diff --backend sqlite`), but nothing
applies it for you — execute it yourself with `conn.execute_batch`. The
checksum-guarded applier in `cratestack-sqlx` is Postgres-only.

## The smallest program that works

```rust
use cratestack::include_embedded_schema;
use cratestack::{RusqliteRuntime, rusqlite_backend::ddl::create_table_sql};

include_embedded_schema!("examples/sqlite_quickstart.cstack");

use cratestack_rusqlite::ModelDelegate;
use cratestack_schema::models::Note;
use cratestack_schema::{CreateNoteInput, NOTE_MODEL};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = RusqliteRuntime::open_in_memory()?;      // or ::open(path)

    runtime.with_connection(|conn| {
        conn.execute_batch(&create_table_sql(&NOTE_MODEL)).expect("create table");
        Ok(())
    })?;

    let notes = ModelDelegate::<Note, uuid::Uuid>::new(&runtime, &NOTE_MODEL);

    let id = uuid::Uuid::new_v4();
    notes.create(CreateNoteInput { id, title: "First note".into(), body: "…".into(),
                                   pinned: true, createdAt: chrono::Utc::now() }).run()?;

    let fetched = notes.find_unique(id).run()?.expect("exists");
    let all = notes.find_many()
        .order_by(cratestack_schema::note::createdAt().desc())
        .run()?;
    Ok(())
}
```

Run the real thing: `cargo run --example sqlite_quickstart -p cratestack-sqlite`
(source: `crates/cratestack-sqlite/examples/sqlite_quickstart.rs`).

## The three types you actually use

**`RusqliteRuntime`** — the connection handle. `open_in_memory()`, `open(path)`,
`with_connection(|conn| …)`. One `Mutex<Connection>` inside, **not `Clone`** by
design; share it with `Arc`. It is `Send + Sync`. Opening sets
`PRAGMA foreign_keys = ON` and best-effort `journal_mode = WAL`.

**`ModelDelegate<'a, M, PK>`** — `ModelDelegate::new(&runtime, &NOTE_MODEL)`.
Cheap; construct it at the call site. `find_many()`, `find_unique(id)`,
`create(input)`, `upsert(input)`, `update(id).set(input)`, `update_many()`,
`delete(id)`, `delete_many()`, `aggregate()`, and the `batch_*` family.

**`ViewDelegate<'a, V, PK>`** — same shape for views. `@@no_unique` views get
`ViewDelegateNoUnique` with no `find_unique`.

Every write builder has `.run()`, `.run_in_tx(&conn)`, and `.preview_sql()`.
`find_many()` chains `where_`, `where_expr`, `where_optional`, `order_by`,
`limit`, `offset`, `include`, `select`, then `.run()` / `.paginate(PageInput)`.

Two guardrails worth knowing: `update_many()` and `delete_many()` **refuse to
run with zero filters** (they return `RusqliteError::Validation`), and
`.for_update()` exists but is a **silent no-op** — it is there so the same source
compiles against the server backend.

## Transactions

There is no `db.transaction(…)` helper. Use rusqlite's own:

```rust
runtime.with_connection(|conn| {
    let tx = conn.transaction()?;
    let row = delegate.create(input).run_in_tx(&tx).map_err(|e| match e {
        RusqliteError::Sqlite(err) => err,
        _ => rusqlite::Error::InvalidQuery,
    })?;
    tx.commit()?;
    Ok(row)
})
```

Dropping `tx` without committing rolls back. Batch writes already run in one
transaction with a `SAVEPOINT` per item; the cap is 1000 items.

## What the macro refuses to compile

These are hard `compile_error!`s at macro expansion, not runtime surprises:

| In the schema | Why |
| --- | --- |
| `query` blocks | No `@@embedded_sql` twin exists for a query; deliberate scope boundary |
| `@computed` fields | Embedded has no response-composition boundary |
| `@@materialized` views | SQLite has no materialized views |
| `extension pgvector` / `extension postgis` | Postgres-only, rejected regardless of features |
| composite `@@id([...])` | Rejected by all three entry macros |
| a `Decimal` field with no `decimal = …` macro argument | Backend must be chosen explicitly |

And these are **silently skipped** rather than rejected — the more dangerous
category, because nothing tells you:

`procedure` declarations · `event` blocks and `@@emit` · `@@audit` · `@version`
optimistic locking · views declared `@@server_sql` only · everything about HTTP,
routes, RPC and transports.

Spatial and pgvector distance **filters** compile but `panic!` at runtime.

## Browser (wasm32 + OPFS)

Default VFS on wasm is memory. Persistence requires installing the OPFS SAH-pool
VFS **from inside a dedicated Worker** — `SyncAccessHandle` is worker-only per
spec, and calling from the main thread returns `OpfsInstallError::NotSupported`.

```ts
await init();
init_panic_hook();
try {
  await install_opfs();     // async, Worker only
  open_db('notes.db');
} catch (error) {
  open_in_memory();         // legitimate fallback; say so in the UI
}
```

Build prerequisites that bite: `rustup target add wasm32-unknown-unknown`,
`cargo install wasm-pack`, and **a wasm-capable clang** — `sqlite-wasm-rs`
compiles SQLite's C source with `cc-rs`, and Apple's stock Xcode clang has no
wasm32 backend (`brew install llvm`; on Debian/Ubuntu `clang` + `lld`).

`cratestack-client-rust` is **target-gated off on wasm32** (reqwest does not
compile there), so `include_client_schema!` alongside `include_embedded_schema!`
works on native but not in the browser — use `fetch` from JS instead.

Note the repo's own warning: there is **no CI coverage for the wasm32 target**.
A missing-import bug once survived because host `cargo check` passed while
`wasm-pack build` did not. Build for wasm explicitly before believing it works.

## Host integrations

| Host | Shape | Detail |
| --- | --- | --- |
| Flutter | `flutter_rust_bridge` over a `cdylib` crate | [references/hosts.md](references/hosts.md#flutter) |
| React Native / Expo | local Expo native module over a C ABI + JNI | [references/hosts.md](references/hosts.md#react-native-expo) |
| Tauri | `tauri-native` (native SQLite in the shell) vs `tauri-web` (wasm in the webview) | [references/hosts.md](references/hosts.md#tauri) |
| Async Rust host (axum, daemon) | `Arc<RusqliteRuntime>` + `tokio::task::spawn_blocking` | below |

A CLI, an FFI `cdylib`, or wasm-in-a-worker needs none of this — they are
already synchronous hosts. You need `spawn_blocking` only when an async runtime
owns the thread:

```rust
let runtime = Arc::clone(&state.runtime);
let rows = tokio::task::spawn_blocking(move || {
    ModelDelegate::<Note, Uuid>::new(&runtime, &NOTE_MODEL).find_many().run()
}).await??;
```

Reference implementations: `examples/embedded-webhook` (axum) and
`examples/embedded-daemon` (batching daemon).

## Traps

**Every column is `BLOB`.** The generic DDL helper emits BLOB for every column,
because BLOB affinity is the only one that preserves the bound storage class.
Consequence: **integer primary keys do not alias rowid, so there is no
auto-increment**. If you need one, write that `CREATE TABLE` yourself through
`with_connection`. The helper also emits no composite indexes, foreign keys, or
named constraints.

**Both entry macros emit a module literally named `cratestack_schema`.** To use
`include_embedded_schema!` and `include_client_schema!` in one crate — a real,
supported combination, see `examples/tauri-native` — wrap each in its own `mod`.

**The schema path is relative to `CARGO_MANIFEST_DIR`**, not to the `.rs` file.

**`.cstack` fields are camelCase**, so embedded crates usually carry a crate-level
`#![allow(non_snake_case)]`.

**The macro emits `::cratestack::*` paths**, so the facade must be in
`Cargo.toml` under the name `cratestack` even in a crate that otherwise only
touches `cratestack-rusqlite`.

**`--all-features` breaks the build** — it enables both mutually exclusive
`decimal-*` backends and trips a `compile_error!` in `cratestack-core`.

**`embedded_flutter_native` is excluded from every workspace command**
(`--exclude embedded_flutter_native`). The reason is *not* the underscore in its
name — it is that the crate has an unconditional `mod frb_generated;` whose
source is generated by `flutter_rust_bridge_codegen` and deliberately not checked
in, so a fresh checkout fails with **E0583**. If you write your own frb-bridged
crate, prefer `cratestack-client-flutter`'s pattern instead: gate
`mod frb_generated;` behind an off-by-default `frb-glue` feature and stay a
normal workspace member.

**One crate = one database role.** `libsqlite3-sys` declares `links = "sqlite3"`
and Cargo permits only one such crate per graph, which is the mechanical reason
the server and embedded facades are kept disjoint. Server-plus-embedded means two
crates. Embedded-plus-client in one crate is fine.

**`cratestack-client-store-sqlite` is not this.** It is an offline request
journal for the HTTP client runtime, not the embedded ORM.
