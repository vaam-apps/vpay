//! The signed-transport half of the vpay merchant SDK, for Node.js.
//!
//! `@vaam-apps/vpay-sdk` is TypeScript, and a COSE_Sign1 envelope is not
//! something to reimplement beside the Rust one: the binding bytes (audience,
//! method, route template, path parameters, canonical query, contract digest,
//! payload type, `Idempotency-Key`) are the whole security of the transport, and
//! two implementations of them would eventually disagree about one byte. So the
//! Node SDK does not seal anything itself. It calls into this crate, compiled
//! to WebAssembly (`just wasm-build`), which is `vpay-sdk`'s own sealing path
//! over cratestack's own `ClientEnvelope`:
//!
//! ```text
//! Node (fetch)  -- sealCall(method, path, form, query, key) --> wasm
//!                <-- { body, headers, openResponse } ----------
//! Node (fetch)  -- POST sealed bytes --------------------------> vpay
//!                <-- sealed answer -----------------------------
//! Node          -- openResponse(status, headers, bytes) -------> wasm
//!                <-- the JSON text, verified ------------------
//! ```
//!
//! The HTTP stays in Node: `fetch`, the caller's agent, timeouts, proxies. The
//! key either goes in as a seed (this crate signs) or stays in Node behind a
//! callback (`crypto.sign(null, tbs, key)`, a KMS), in which case this crate
//! only ever sees the bytes to sign and the signature, and checks the
//! signature against the registered public key before using it.
//!
//! The operation table is `sdks/rust/src/contracts.rs`, included by path so
//! that the Rust SDK, this crate and (through `vpay-api`'s parity tests) the
//! server cannot hold three different tables.

// `contracts.rs` is included by path from `sdks/rust`, doc comments and their
// examples with it. Those examples are written against `vpay_sdk` and are run
// there (`just test-doc` covers `vpay-sdk`); collected a second time under this
// crate's name they could only fail to resolve it. rustdoc sets `cfg(doctest)`
// while it collects, so under it this crate is empty, and under every other
// build it is not.
#![cfg(not(doctest))]

mod core;

// The JavaScript surface exists only where there is a JavaScript to bind to.
// On a host the crate is `core` alone, which is what `cargo test --workspace`
// compiles and runs, so the sealing path has native tests and the workspace
// builds on a machine with no wasm toolchain.
#[cfg(target_arch = "wasm32")]
pub mod js;

#[cfg(all(target_arch = "wasm32", feature = "responder"))]
pub mod responder;

#[path = "../../src/contracts.rs"]
#[allow(unreachable_pub, dead_code)]
mod contracts;

pub use crate::core::{OpenError, SealError, Sealed, Sealer, SealerKey};
