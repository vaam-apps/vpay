# Lane C — what mounting CrateStack's transport actually involves

_Read from the 0.12.0 sources on 2026-09-13, before any code was written.
Every claim below cites the file it was read from, because none of it is
guessable from the schema and one of them is a security trap._

## The generated router already exists, inside a module that is private on purpose

`::cratestack::include_server_schema!` expands to `pub mod cratestack_schema
{ … }` and that expansion **already contains `pub mod axum`** — the handlers,
the router functions, the DTOs
(`cratestack-macros-0.12.0/src/include/server/axum_module.rs`). Nothing has to
be generated; something has to be _reached_.

Reaching it is the problem. `backends/crates/vpay-db/src/schema.rs` holds the
invocation in a **private** `mod schema`, and its comment says why: at the
crate root the same expansion would publish every model struct, every delegate
and the generated `pub mod axum` to every consumer, "reversing ADR-0016
standard 5 in a diff that looks like one line". `cargo xtask
verify-repositories` fails if `mod schema` is ever made `pub` or named in a
`pub use`, precisely because the generated module exists in no source file and
a text-scanning gate would otherwise see nothing wrong.

**So lane C exposes the _router_, not the _module_.** A narrow `pub fn` in
`vpay-db` that returns a built `axum::Router` keeps the gate intact and keeps
ADR-0016 standard 5 true. Making `mod schema` public to get at the router is
the one thing this lane must not do — and if it is attempted, the gate is
what says no.

## The trap: `router()` mounts every model's CRUD, writes included

```rust
pub fn router<R, CR, C, Auth>(db, registry, resolvers, codec, auth_provider, body_limit_bytes) -> axum::Router
```

merges `model_router(...)` **and** `procedure_router(...)`
(`include/server/axum_module/router_fn.rs`). `model_router` is the generated
CRUD for **every model in the schema** — every one of the eighteen emits
`handle_{list,create}_<plural>` and `handle_{get,update,delete}_<singular>`
whether or not anything routes them.

The maintainer's decision for this work is **reads only**. So:

> **Lane C mounts `procedure_router`, never `router()`.**

`procedure_router` is emitted `pub` in the same module with the same arguments
minus `body_limit_bytes` (`axum_module.rs:138`), so this is a choice the code
can express directly.

What stands between a mounted `model_router` and a write is the model policy —
`@@allow("create"/"update"/"delete", auth().isSystem())` on `DisabledClient`,
`StaffMember`, `StaffSession`, `OauthAuthorizationCode`, `Customer`, `Event`,
`WebhookDelivery`, `Invoice`. That is one predicate, and **the thing on the
other side of it is the second rule below**.

## The second rule: never mint a system context for a request

`vpay_db::persistence::system_context()` is
`SystemContext::for_service(SYSTEM_SERVICE).into_context()` — and
`is_system()` is true for it. It is the only `CratestackContext` vpay mints
anywhere today.

If a request-scoped context were ever built from it, every
`auth().isSystem()` policy in the schema would pass for that request. Combined
with a mounted `model_router` that is remote write access to `staff_members`
and `oauth_authorization_codes`. The two rules protect each other, and neither
is sufficient alone.

The context for a request comes from lane B's tenancy type and carries a
tenant (or the admin's cross-tenant decision). It is not a `SystemContext` and
must never become one.

## The seam is a closure

`AuthProvider` (`cratestack-core-0.12.0/src/context.rs:92`) has a blanket impl
for any

```rust
Fn(&http::HeaderMap) -> Result<CratestackContext, E>  where E: Into<CratestackError>
```

so the authentication seam is a closure over lane B's output, not a new trait
hierarchy. That is the whole of what "mint a tenant-carrying context from the
staff token" costs structurally.

## What is already in the dependency graph

`cratestack-axum` and `cratestack-client-rust` are **non-optional**
dependencies of `cratestack-pg` with no feature gating them (root
`Cargo.toml`'s comment at line 284 says so, and `cratestack-pg-0.12.0`'s
manifest confirms it). Mounting the transport adds no dependency.

## The proving case

`procedure searchPaymentIntents` already has a complete, container-tested body
in `backends/crates/vpay-db/src/schema/search_payment_intents.rs`. Lane C
declares no new procedure. It is done when that one answers over HTTP with the
same rows its tests already assert, refuses an unauthenticated caller, and
refuses a tenant mismatch — and when `schema.rs`'s `cfg_attr(not(test),
allow(dead_code))` on `Payments` can be **deleted**, because something now
calls it. That comment says to delete it "the day something serves the
procedure — and update `docs/status.md` in the same commit". This is that day,
and that deletion is lane C's own completeness check.
