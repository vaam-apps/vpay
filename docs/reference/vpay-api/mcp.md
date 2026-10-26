# `vpay-api` — the merchant MCP endpoint (`mcp.rs`)

_New 2026-10-26, with ADR-0033 step 3. The process it implements is
[flows/mcp.md](../../flows/mcp.md); this page says why the code is shaped the
way it is._

## Three crates, one direction of dependency

`vpay-db` owns everything CrateStack:

- the `@mcp(tool)` procedures in `schemas/vpay.cstack`, and their bodies
  (`schema/mcp_merchant.rs`);
- the router built around the generated tool table
  (`Repositories::merchant_mcp_router`);
- the two traits the bodies and the transport need, and that it cannot
  implement itself:
  - `MerchantOperations`: run a `/v1` operation;
  - `McpAuthenticator`: turn a bearer token into a merchant.

`vpay-api` implements both traits (`mcp::MerchantOps`,
`mcp::MerchantMcpAuth`), and `vpay-server`'s `main` hands them over when
`deployment.surfaces` names `mcp`. `vpay-api` never builds a
`CratestackContext` and never names a CrateStack type, which is the
dashboard procedure transport's arrangement for the same reason
(`vpay-db/src/dashboard_transport.rs`).

## A tool call is a `/v1` request with another origin

`MerchantOps` reimplements no rule:

- **Reads** call the read handlers directly. Their extractors are tuple
  structs (`State`, `Path`), so a caller can build them.
- **Writes** call the `*_with` operation Phase 2 split out of each handler,
  with a `PostRequest::for_tool`.

That `PostRequest` carries:

- **The parameters.** These are the nested JSON value
  `crate::form::parse_form` would have produced from the matching form body,
  with string leaves. So the same `serde` decode runs, and so does every rule
  after it.
- **A fingerprint.** It is `request_hash("MCP", "tool/<name>", <the
parameters' JSON>)`. It is claimed in the same per-merchant
  `idempotency_keys` row as `/v1` would claim, under the same rules.

The answer is `/v1`'s response. A success is reshaped by
`reshape_payment_intent` / `reshape_refund` into the output type's shape. A
refusal is `/v1`'s envelope, carried across as `OperationRefusal` and rendered
by `vpay-db` as a CrateStack error whose message starts with `/v1`'s `code`.

## Authentication

`MerchantMcpAuth` makes `require_merchant_token`'s three checks against a
validator whose audience is the MCP resource identifier:

1. the token validates;
2. its client is registered to a merchant;
3. it holds `payments:read` or `payments:write`.

`can_write` is the second scope. The validator is a separate `JwtValidator`
for `loopback_dashboard_validator`'s reason: a validator holding two
audiences accepts a token for either.

The OP side is one function in `op/token.rs`, `apply_resource_indicator`.
`authkestra_op`'s `TokenRequest` has no `resource` field, and the form
decoder drops a key it does not know, so RFC 8707's `resource`, which MCP
clients send, never reached the audience gate. Measured 2026-10-26: a token
was minted with the default audience, and `/mcp/v1` refused it. One
`resource` with no `audience` is now the audience. Two targets, or a
`resource` that disagrees with `audience`, are `invalid_target`.

## Why `/dash/v1/$procs` has an allowlist now

`dashboard_procedure_router` wraps CrateStack's generated `procedure_router`,
which mounts every procedure the schema declares, the merchant MCP writes
included. CrateStack 0.16 has no attribute that keeps a procedure off REST.
Three things stop a staff member reaching one, outermost first:

1. `only_dashboard_procedures` in `lib.rs` answers the honest 404 for any
   name outside the dashboard's five.
2. The procedure's own `@allow` needs `audience == "merchant"`, a claim the
   dashboard's context does not carry.
3. The dashboard's registry has no port (`ops: None`), so a body that ran
   anyway would refuse.

`a_merchant_mcp_procedure_is_not_served_to_the_dashboard` pins the first. It
measures the second by turning `403` when the first is removed.
