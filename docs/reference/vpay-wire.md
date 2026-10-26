# `vpay-wire`

The contract table of the `/v1` signed transport (VP-2): which operations
exist, and the 32 bytes a signed call to each binds. Nothing else.

## What a contract digest is for

A COSE-signed request binds, in its external AAD, the digest of the operation it
calls (cratestack binding version 2, ADR 0006 §4). The server's `EnvelopeLayer`
holds per operation every digest it accepts (`AcceptedContracts`), and a client
that is out of date gets an unsigned `426 contract_unsupported` before any key
is looked up. For a schema-generated router cratestack emits that table. `/v1`
is hand-written, so `vpay-wire` is where vpay says what the schema would have
said.

## What the digest covers, and what it cannot

cratestack's `op_contract_digest` hashes the closure of schema declarations an op
is decoded under, from a parsed `Schema`, and the only keys it knows are the
ones a schema generates (`model.X.verb`, `POST /$procs/name`). It has no entry
point that takes a hand-written route (read at 0.16.0:
`cratestack-core/src/client_contract/{ops,table}.rs`), and the layer side is
built for exactly this case: `BindingResolver` plus `ResolvedRoute::
with_contract_key` let a custom resolver name any key in the table. So no
upstream change is needed, but vpay must supply the digests itself.

`contract_digest(method, path)` is `SHA-256(CONTRACT_DOMAIN ‖ "rest" 0x00 ‖
method 0x00 ‖ path)`, domain-separated the way cratestack's own is. It is the
operation's **identity**:

| Change                                               | Moves the digest? |
| ---------------------------------------------------- | ----------------- |
| A route is added, removed or renamed                 | yes               |
| A path gains or loses a verb (`PATCH` beside `POST`) | yes (new key)     |
| A form field or a response field is added or removed | **no**            |

The last row is the weakness: vpay's bodies are described in prose
(`docs/flows/merchant-auth/resource-contract.md`), not in anything a program can
hash, so a body change that must refuse older clients is not caught here.
Strengthening it needs a machine-readable description of each body as a second
input, and the domain tag moves to `v2` when it gets one. The derivation is
frozen by `contract::tests::the_derivation_is_frozen`, whose expected values
were computed outside Rust.

## Why a crate, and why this one

- **Not `vpay-api`**: it carries axum, the database and the handlers, and a
  client must be able to read the table.
- **Not `vpay-core`**: every crate depends on it, adapters and the database
  included, and it should not learn what a COSE contract is.
- **Its own crate, with no `vpay-*` dependency** and only `cratestack-core` (for
  the `AcceptedContracts` type the layer takes) and `sha2`: a client can depend
  on it without inheriting the backend.

`sdks/rust` does not depend on it. That SDK is meant to be publishable without
this workspace, and an arrow from it into `backends/` would make its release
depend on publishing this crate. Publish `vpay-wire`, or vendor the table into
the SDK behind a parity test: VP-6's decision, left open on purpose.

## The two lists, and what holds them together

`V1_OPERATIONS` (here) and `V1_ROUTES` (`vpay-api`, with handlers) both list
what `/v1` answers. Linking them would put handlers in a client, so they stay
two, and `vpay_api::v1::contracts::contract_gaps` compares them in both
directions: a route with no contract fails (the layer would answer a `500` for
every request to it, since it cannot tell what the request was signed under),
and a contract with no route fails. Keys are `"<METHOD> <path>"` with the path
relative to the `/v1` mount, which the layer is told as `.mount_prefix("/v1")`
(VP-3); `find_contract` answers a `HEAD` from its `GET` row.

## `V1_CONTRACTS` is a `LazyLock`, not a `const`

`AcceptedContracts` is `&'static [(&'static str, &'static [[u8; 32]])]` and
SHA-256 is not const-evaluable. The alternative to building the table once at
first use (about as many small leaked allocations as there are operations, for
the life of the process) was pasting that many opaque hex literals from a
script: a second copy of what `contract_digest` computes. Use it as
`*V1_CONTRACTS`.
