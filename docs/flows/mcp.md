# MCP: the merchant agent surface

How a merchant's agent calls vpay over the Model Context Protocol, revision
`2026-07-28`, Streamable HTTP. The decision is
[ADR-0033](../adr/0033-mcp-is-the-primary-agent-surface.md) (Proposed), and the
threat model is
[security/threat-models/mcp.md](../security/threat-models/mcp.md). The route
by route map to `/v1` is [api/mcp-parity.md](../api/mcp-parity.md), and a gate
checks it.

## What happens

1. **The deployment opts in.** `deployment.surfaces` names `mcp`. An absent key
   never does: `EnabledSurfaces::ALL` does not include it. The endpoint is then
   `POST {public_base_url}/mcp/v1`, and that URL is also the resource
   identifier.
2. **The merchant opts in.** Its `merchant_clients[]` entry lists the resource
   identifier in `allowed_audiences`, beside `vpay:v1`.
3. **The agent gets a token.** It discovers the authorization server from
   `GET /.well-known/oauth-protected-resource/mcp/v1` (RFC 9728), which names
   `{public_base_url}/v1/oauth`. It then runs the usual `/v1` flow,
   `client_credentials` with `private_key_jwt`, asking for the resource either
   as `audience=` or as RFC 8707's `resource=`. The OP mints a token with
   `aud` set to the resource, and only for a client that listed it. The token
   endpoint itself is on a `business` pod (ADR-0022); an `mcp` pod serves the
   discovery document and JWKS but mints nothing.
4. **The agent calls tools.** Each request is a JSON-RPC `tools/list` or
   `tools/call` with the bearer token. CrateStack's transport checks:
   - the `Origin`: only the deployment's own, or none;
   - the bearer token;
   - the MCP headers.

   vpay's `MerchantMcpAuth` then validates the token against the resource
   audience, maps its client to a merchant, and reads `payments:write`.

5. **The call runs.** It goes through the procedure's generated policy
   (`@allow`), then its body in `vpay-db`, which asks the `MerchantOperations`
   port. `vpay-api` implements that port by running the same function the
   `/v1` route runs, with the parameters a form body would have decoded to.
6. **The answer** is `/v1`'s object reshaped in three named places. A
   refusal is an `isError` result whose text is
   `{code, message}`, where `message` begins with `/v1`'s own `error.code`.

## Invariants

- **The tenant comes from the token, never from arguments.** No tool takes a
  merchant id. Another merchant's object is the same "not found" as one that
  does not exist.
- **A write needs `payments:write`.** A `payments:read` token is refused by
  the procedure's own `@allow`, before any body runs.
- **Every write takes a required `idempotency_key`.** It is claimed in vpay's
  own per-merchant store under `("MCP", "tool/<name>", hash of the
arguments)`. A replay returns the first answer. The same key with different
  arguments is `/v1`'s `idempotency_key_in_use`.
- **No new rule.** Validation, rails, events and the ledger are `/v1`'s. A
  tool can do nothing its route cannot.
- **The dashboard transport serves none of it.** CrateStack's generated
  procedure router mounts every procedure, so `/dash/v1/$procs` has an
  allowlist of its five searches. The write procedures' policy also refuses a
  dashboard context.

## What can go wrong

- **A prompt-injected agent asks to move money.** vpay cannot see the prompt.
  The controls are scope (issue an agent a read-only client), the idempotency
  key, and the `events` every write emits. An amount ceiling is undecided
  (ADR-0033, "Left to the maintainer").
- **A token for the wrong audience** is answered `401` with a challenge naming
  the metadata document. That covers a `vpay:v1` token on `/mcp/v1` and an MCP
  token on `/v1`.
- **A tool that a route has but MCP lacks.** The parity table dates the gap,
  and the gate refuses a route with no row.

## Status

**Built 2026-10-26 (ADR-0033 step 3): the merchant endpoint, nine tools, no
resources, no staff endpoint.**

- **The tools.** These are built:
  - `create_payment_intent`, `retrieve_payment_intent`,
    `confirm_payment_intent`, `cancel_payment_intent`;
  - `create_refund`, `retrieve_refund`, `cancel_refund`;
  - the existing `search_payment_intents` and `search_refunds`.

  The other 28 `/v1` routes are dated gaps in
  [api/mcp-parity.md](../api/mcp-parity.md).

- **What is proven, and where.**
  [`mcp_merchant.rs`](../../backends/tests/integration/tests/mcp_merchant.rs)
  runs against a real Postgres, the real router and the real OP. It proves:
  - tenancy;
  - scope;
  - both audience directions;
  - the 401 challenge and the metadata document;
  - a foreign `Origin` refused;
  - idempotency replay, mismatch and an empty key;
  - `/v1`'s refusal of a confirm on an unknown rail;
  - the refund tools reaching `/v1`'s refund code;
  - an absent `surfaces` key mounting nothing;
  - tool outputs equal to the reshaped `/v1` objects;
  - the OP minting an MCP token only for an opted-in client, by `audience`
    and by RFC 8707 `resource`.

  [`dashboard_procedure_transport.rs`](../../backends/tests/integration/tests/dashboard_procedure_transport.rs)
  proves the merchant procedures are a 404 on `/dash/v1/$procs`.

- **Not proven: a confirm a rail settles through MCP.** The confirm runs
  `/v1`'s own `confirm_with`, and `confirm_rails.rs` proves that function
  against WireMock over `/v1`; no MCP case drives it to `succeeded`.
- **No real MCP client** (an agent host or `rmcp`'s client) has called it.
  The suite speaks the protocol's JSON-RPC over HTTP directly.
- **No resources.** ADR-0033 D5's `jsonb` measurement has not been made.
- **No staff endpoint.** It waits for the upstream CrateStack audience
  feature (ADR-0033 D2).
- **No chart workload.** ADR-0033 step 6.
- **No pod has run it.**
