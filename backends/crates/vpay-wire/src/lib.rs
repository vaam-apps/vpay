//! What a signed `/v1` request binds and a server accepts: the contract table.
//!
//! A COSE-signed request binds the digest of **the operation it calls** into
//! its external AAD (cratestack binding version 2, ADR 0006 §4), and the
//! server's `EnvelopeLayer` holds, per operation, every digest it accepts
//! ([`AcceptedContracts`]). For a schema-generated router cratestack emits
//! that table from the `.cstack` schema. `/v1` is hand-written (`vpay-api`'s
//! `V1_ROUTES`), so there is no schema to emit it from, and this crate is
//! where vpay says what it would have said: one row per operation, in one
//! place both the server and a client can read.
//!
//! * [`V1_OPERATIONS`] — the operations, as `(method, path)` pairs, with the
//!   path relative to the `/v1` mount exactly as `V1_ROUTES` spells it.
//! * [`operation_key`] — `"<METHOD> <path>"`, the key shape cratestack uses
//!   for a REST op, so `cratestack_core::find_contract` finds a row (and
//!   answers a `HEAD` from its `GET`).
//! * [`contract_digest`] — the 32 bytes an operation binds.
//! * [`V1_CONTRACTS`] — the table, one digest per operation.
//!
//! # What the digest is, and what it is not
//!
//! cratestack's own op digest (`cratestack_core::op_contract_digest`) hashes
//! the *closure of the schema declarations* an op is decoded under, so it
//! moves when a field of its input or output changes. It is computed from a
//! parsed `Schema`, and the only ops it can describe are the ones a schema
//! generates (`model.X.verb`, `POST /$procs/name`); there is no entry point
//! that takes a hand-written route. The derivation is not reusable here and
//! this crate does not pretend otherwise.
//!
//! Instead [`contract_digest`] is domain-separated SHA-256 over the
//! operation's **identity**: transport, method and route template. It moves
//! when a route is added, removed, renamed or changes verb, and it does
//! **not** move when a form field is added to a body, because vpay has no
//! machine-readable description of its bodies to hash (there is no OpenAPI
//! file in this repository, `docs/flows/merchant-auth/resource-contract.md`
//! is prose). That is a weaker guarantee than a generated table gives, and
//! it is stated so a later ticket can strengthen it deliberately: a body
//! change that must refuse older clients needs a second derivation input,
//! and the domain tag moves to `v2` when it gets one.
//!
//! ```
//! use cratestack_core::find_contract;
//! use vpay_wire::{V1_CONTRACTS, contract_digest};
//!
//! // The table is keyed the way a REST op is: `"<METHOD> <path>"`.
//! let digests = find_contract(&V1_CONTRACTS, "GET", "/payment_intents/{id}").unwrap();
//! assert_eq!(digests, &[contract_digest("GET", "/payment_intents/{id}")]);
//!
//! // A `HEAD` is answered from the `GET` row, as axum serves it.
//! assert_eq!(find_contract(&V1_CONTRACTS, "HEAD", "/events"), find_contract(&V1_CONTRACTS, "GET", "/events"));
//!
//! // A method no route answers has no row.
//! assert!(find_contract(&V1_CONTRACTS, "PUT", "/events").is_none());
//! ```

mod contract;
mod operations;

pub use contract::{CONTRACT_DOMAIN, V1_CONTRACTS, contract_digest, operation_key};
pub use cratestack_core::AcceptedContracts;
pub use operations::{V1_MOUNT, V1_OPERATIONS, V1Operation};
