//! `schemas/vpay.cstack`, compiled.
//!
//! This module exists to hold one macro invocation and to keep everything it
//! expands to inside this crate. It is `mod schema;` in `lib.rs` — never
//! `pub mod` — and nothing below it is re-exported, ever.
//!
//! Why that is a rule rather than a preference, and what enforces it, is in
//! [docs/reference/vpay-db.md § CrateStack](../../../../docs/reference/vpay-db.md#cratestack).

// `include_server_schema!` expands to `pub mod cratestack_schema { … }`,
// which is why the invocation lives in a private module rather than in
// `lib.rs`: at the crate root the same expansion would publish
// `vpay_db::cratestack_schema::*` — every model struct, every delegate, and
// the generated `pub mod axum` — to every consumer, reversing ADR-0016
// standard 5 in a diff that looks like one line. `cargo xtask
// verify-repositories` fails if this module is ever made `pub` or named in a
// `pub use`, because the generated module does not exist until after macro
// expansion and a text-scanning gate would otherwise see nothing wrong.
//
// The path is resolved against `CARGO_MANIFEST_DIR`
// (`cratestack-macros-0.11.1/src/include/parse.rs`), not against this file,
// so it climbs out of `backends/crates/vpay-db`.
::cratestack::include_server_schema!("../../../schemas/vpay.cstack", db = Postgres);
