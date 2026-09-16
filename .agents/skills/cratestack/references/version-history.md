# Feature-to-release map

**Why this file exists:** a skill that says "CrateStack does X" is only true for
some range of versions. CrateStack is pre-1.0, every public crate shares one
version, and minor releases break. A fact that is right for 0.12.0 can be wrong
for the 0.9.x a reader is actually on — and, worse, right for an unreleased
`main` and wrong for every published version.

**How to check what you are on:**

```bash
cratestack --version
grep -n 'cratestack' Cargo.toml          # the facade version
```

Everything in these skills is verified against **0.12.0** unless a marker says
otherwise. Read every *(since X.Y.Z)* as "absent before X.Y.Z" and every
**breaking** note as "your call sites change".

## Reading order for an upgrade

The framework's own `CHANGELOG.md` is the authority and is unusually detailed —
entries explain the defect, not just the change. This file is a routing index
into it, not a replacement.

---

## 0.12.0 (2026-09-06)

- `generate-typescript --rtk` — a generated RTK Query endpoint set, with
  invalidation tags **derived from the schema** by walking each procedure's args
  and return type.
- An **enum-typed field is now a filterable query-filter scalar**, not only
  sortable (#928).
- The Rust client's HTTP transport is **pluggable**, so retries and tracing no
  longer need a fork.
- A `@computed` field may be typed with a `type` block **on a model**, not only
  on a `type`.
- **Breaking:** `SchemaError` carries its own `file` and `source_text`, and
  `render()` **lost both arguments** (#916). Re-exported from `cratestack-pg`,
  `-api` and `-sqlite`, so a consumer calling `error.render(path, source)` gets a
  compile error. Drop both arguments; parse with `parse_schema_named` if you want
  the error to carry a path you know. `ANONYMOUS_SCHEMA` is exported so you can
  recognise the placeholder. `--format json` diagnostics gained a `file` key
  (purely additive).

## 0.11.1 (2026-09-03)

- **`@no_idempotency` works**, and idempotency admission moved into the new L3
  crate `cratestack-exec`. **Breaking.**
- **Breaking (#871):** the rate-limit bucket keyspace is bounded — the
  princ/auth/ip key scheme with the 128 and 8192 caps.
- Procedures and auth providers are documented as plain `async fn` in every
  example and in the trait docs.
- ADR 0015 accepted (amended): the L3 `OpExecutor` is being built in slices.
- Open VSX publishing went live for the VS Code extension.

## 0.11.0 (2026-09-03)

- **A `.cstack` schema can declare a parameterized custom SQL `query`** — the
  whole `query name(args): Type @@sql("…")` surface.
- **Breaking (#846):** rate-limit store failures fail open **only** for transport
  errors; every middleware error body is typed.
- The VS Code extension's display name became `CrateStack Schema`.
- `@cratestack/cbor-node` ships musl (Alpine) platform packages.

## 0.10.0 (2026-08-31)

- **PostGIS spatial columns are declarable** (#842) — `Geography` / `Geometry`
  and `extension postgis { }`.
- **`up.pre.sql` is a real mechanism** (#843) — before this, blocking migrations
  had no backfill hook.
- `@default(dbgenerated())` no longer drops the default it asserts exists (#843).
- `cratestack migrate diff` no longer panics on a `pgvector` schema.
- Generated client version ceilings follow the release line automatically.

## 0.9.1 (2026-08-29)

- **Read policies gained `in` / `not in` against a set of literals** (#666).
- **`Bytes` survives `transport rpc`** (#820, #806) — two independent defects,
  one symptom.
- **Breaking, `--template-dir` only:** TypeScript templates render under
  `UndefinedBehavior::Strict` (#774).
- Linux arm64 for the Dart CBOR codec corrected from "half open" to **blocked
  upstream**, both halves (#823).

## 0.8.15 (2026-08-28)

- **A misspelled field attribute is now a parse error, not a silent no-op**
  (#679). Before this, `@reedonly` reported `schema OK` and enforced nothing.
- **Breaking:** generated TypeScript clients type `Bytes` as `Uint8Array`
  (#783 follow-up), and `Bytes` fields round-trip a JS `Uint8Array` (#783).
- The parser rejects a schema name colliding with the generated client's own
  methods (#784 follow-up) and a procedure colliding with a model's generated
  CRUD handler (#784).
- Every generated dependency constraint is an API floor, not the workspace
  version (#779).
- `--tanstack` rejects a procedure hook colliding with a model hook (#802);
  `--swr` gained the same check in 0.8.14 (#777).

## 0.8.14 (2026-08-27)

- **`@@internal("action")` route suppression** — REST, RPC and every generated
  client (#743).
- **`@@unique` / `@@index` gained `where: "<sql predicate>"`** — partial index
  DDL. **Breaking for `cratestack-core` API consumers** (#742).
- **`ConflictTarget` can target a partial unique index** and the upsert conflict
  probe honours it. **Breaking for exhaustive external matches** (#741).
- **Breaking (#746): `@cratestack/cbor` became the default codec for generated
  TypeScript RPC clients.** This is the one that silently upgrades a regenerated
  package's wire format from JSON to CBOR and makes a JSON-only server answer
  406/415.
- **Breaking (#765):** `--swr` + `transport rpc` honours `native_cbor` too.
- `.upsert(..).run(..)` stopped reporting `Created` for an update it lost a race
  on (#745).
- Schema validation reports **every independent error**, not just the first.
- `.cstack` editor features: rename (F2), semantic tokens, enum/mixin
  go-to-definition, find-all-references, and no longer blinking off on every
  syntax error.
- **Documented:** Studio's `[target.db]` write path enforces no schema-declared
  write constraint (#744).
- Generated Dart builders moved to `package:cratestack_builder` — **breaking for
  build tooling** (#668).
- `CRATESTACK_REQUIRE_DB` now fails when *no* database backend is configured
  (#747).

## 0.8.12 (2026-08-24)

- **RPC `get` gained the selection surface REST already had** — this is where
  `RpcGetInput` came from. Before it, a `fields` key on an RPC get frame was
  **silently dropped by serde** and the server returned the full record, with no
  error and no signal.
- `<Model>ComputedParams` gained the standard builder.
- **The transport-parity convention was written down here** — REST and RPC ship
  together, never REST first — because the `@computed` params surface had shipped
  REST-only and took three follow-up PRs to close.
- A caveat recorded for Rust clients: adding a model's first parameterised
  computed field changes `get(id, headers)` into
  `get(id, computed_params, headers)` and **breaks call sites**.

## 0.8.11 (2026-08-24)

- **`@computed` — resolver-backed response-time fields, replacing `@custom`.**
  `@custom` is now a parse error pointing here.
- flutter_rust_bridge moved to **2.13.0** — breaking for consumers on 2.12.0, and
  the pin is install-blocking because pub treats a bare version as exact.

## Earlier, and worth knowing because removed things linger

- **0.8.5 removed protobuf/gRPC.** `transport grpc` is a parse error with a
  migration message, and `@pb` is rejected at field position. Documentation and
  blog posts describing a third transport are describing something that no longer
  exists — this one stayed documented as shipped for nine releases.
- **0.4.0 split the umbrella crate into facades.** `cratestack-client` (the pure
  HTTP-client SDK facade) was added later, by #490. Before the split there was a
  single `cratestack` crate; today `crates/cratestack` is an **empty
  documentation-only vitrine crate** and `-p cratestack` returns a false green.
- **#505 made the decimal backends additive.** Both `decimal-rust-decimal` and
  `decimal-bigdecimal` may now be selected in one build, and selecting *neither*
  is also fine (#521). Before that, both-selected was a hard `compile_error!` in
  `cratestack-core` — which was itself the defect #505 reports. Backend choice
  moved to a schema-authored `decimal = …` macro argument. Several places in the
  repo still give the old rationale for avoiding `--all-features`.
- **#523 made `unsafe_code = "forbid"` actually enforced** by requiring every
  workspace member to opt in, with `just verify-lints-optin` as the guard. Cargo
  silently ignores `[workspace.lints]` for a member that does not opt in, which
  is exactly the drift that found.

## Things that are declared but inert — check before assuming a version fixed it

As of 0.12.0, each of these parses, validates, and then does nothing:

- `@isolation("…")` — no generated accessor, no auto-wrapping. Call
  `run_in_isolated_tx` yourself.
- `@@retain(days: N)` — descriptor metadata; no GC job exists.
- `@from(Model.field)` on a view field — checked by nothing.
- `prefer_for` in `studio.toml` — parsed and never consulted.
- The per-frame `idem` field on `/rpc/batch` — decoded and never read.
- `@@id([a, b])` — parses, emits correct DDL, then every entry macro rejects it
  (issue #136).
- The five `batch_*` ORM primitives — complete, and reachable from no generated
  route.

If a future release wires any of these up, that is a changelog entry to look for
before believing this list.

## Unreleased at the time of verification

Facts marked *(unreleased)* in a skill are on `main` and in **no published
version**. As of this writing that includes:

- Optional scalars exposing `eq` / `ne` / `in` query filters on generated list
  routes (#953). Before it, an optional field such as `verificationId String?`
  accepted `isNull` and string patterns but returned **HTTP 400** for
  `verificationId=value` — even though the typed Rust `Where` path already
  supported it.
