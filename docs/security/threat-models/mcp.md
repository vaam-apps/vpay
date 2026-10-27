# Threat model: the MCP surface

- **Written:** 2026-10-26, with
  [ADR-0033](../../adr/0033-mcp-is-the-primary-agent-surface.md) and before
  any of its code, as [the security README](../README.md) requires.
- **Method:** STRIDE, per element of the data flow below.
- **Scope:** what ADR-0033 adds:
  - two inbound HTTP interfaces, `POST /mcp/v1` (merchants) and
    `POST /dash/mcp/v1` (staff);
  - two token audiences on vpay's existing authorization servers;
  - a new kind of caller, an LLM agent, which acts on text it did not
    write;
  - a new disclosure surface: tool results and resources entering a model's
    context.

  Out of scope: the rails, the ledger, and `/v1` itself. Nothing here
  changes them, and the rails' own model is
  [notchpay.md](notchpay.md).

- **Evidence status:** written before any code, with every row "not built".
  ADR-0033 step 3 (2026-10-26) built the merchant endpoint, and each row it
  moved names its passing test. Rows about the staff endpoint, resources and
  the chart are still **not built**, because none of those exists.

## The data flow

```text
  merchant's agent host                 staff member's agent host
  (holds a private_key_jwt key)         (PKCE, loopback redirect: SUPERSEDED,
                                         see the note under this diagram)
        │ client_credentials,                 │ authorization code
        │ audience = <merchant MCP resource>  │ aud = <staff MCP resource>
        ▼                                     ▼
  /v1/oauth/token (business tier)      /dash/v1/oauth/* (management tier)
        │ RS256 JWT                           │ RS256 JWT
        ▼                                     ▼
 ┌──────────────────────────── vpay-server ─────────────────────────────┐
 │ POST /mcp/v1  (surface `mcp`)        POST /dash/mcp/v1 (`mcp_staff`)  │
 │   cratestack-mcp guard: Origin, Bearer, 401 challenge, no anonymous   │
 │   AuthProvider ── JwtValidator (aud) ── tenant, scopes ── context     │
 │   tools/call → invoke_with_db → @allow → body ─┐                      │
 │   resources/read → ORM read, @@allow in SQL ───┤                      │
 │                                                ▼                      │
 │        vpay-db ── MerchantOperations (vpay-api) ── rail adapters      │
 │                         │                                             │
 │                    Postgres (idempotency_keys, payment_intents, …)    │
 └──────────────────────────────────────────────────────────────────────┘
        ▲
        │ text the agent read elsewhere: a customer name, an invoice memo,
        │ an email, a web page. Any of it can carry instructions.
  a third party
```

**The staff column is superseded (2026-10-27).** #383 (ADR-0036) deleted
`/dash/v1/oauth/*`, the staff OP and staff sessions; the console signs with a
paired key instead. The staff side of this diagram is therefore what was
designed, and nothing serves it. How a staff agent authenticates is
ADR-0033's "Left to the maintainer" item 5. The two staff rows below keep
their mitigations as designed, and are **not built** for that reason as well
as for the missing endpoint.

The trust boundaries crossed are:

- agent host ↔ vpay, inbound and authenticated by token;
- agent host ↔ each authorization server;
- vpay ↔ the model's context, outbound, where data leaves vpay's control;
- third party ↔ the agent, a boundary vpay does not see. **That is the one
  that makes this surface different.** On `/v1` the caller is code a
  merchant wrote. On `/mcp/v1` the caller can be steered by any text it
  has read, and vpay may itself have rendered that text into a tool result.

## STRIDE

### 1. The endpoints, agent → `/mcp/v1` and `/dash/mcp/v1`

| Threat                                                                                              | Kind                   | Mitigation                                                                                                                                                                   | State                                                                                                                                                     |
| --------------------------------------------------------------------------------------------------- | ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A request with no token, or an anonymous context, is served                                         | Spoofing               | cratestack-mcp's guard requires a bearer before the provider runs, and refuses an unauthenticated context after it (401 with `WWW-Authenticate: Bearer resource_metadata=…`) | **built** 2026-10-26: `each_surface_accepts_only_its_own_audience` (401 and the challenge naming `resource_metadata`)                                     |
| A `/v1` token (`aud = vpay:v1`) is replayed against `/mcp/v1`, or an MCP token against `/v1`        | Spoofing               | Each validator checks its own audience. The merchant MCP resource is a distinct `allowed_audiences` entry                                                                    | **built** 2026-10-26: `each_surface_accepts_only_its_own_audience`, both directions                                                                       |
| The merchant OP is retired (VP-4) while `/mcp/v1` still depends on it for tokens                    | Denial of service      | None yet: since VP-3 the OP's `client_credentials` grant serves only this endpoint. ADR-0033, "Left to the maintainer" item 4                                                | **not built**: an open decision, 2026-10-27                                                                                                               |
| A merchant token is used on the staff endpoint, or a staff token on the merchant one                | Elevation of privilege | Distinct audiences, distinct validators, distinct issuer keys per surface                                                                                                    | **not built**: there is no staff endpoint yet (ADR-0033 D2)                                                                                               |
| A browser page drives the endpoint with a user's ambient credentials (DNS rebinding, CSRF)          | Spoofing               | Bearer-only (no cookie), and cratestack-mcp refuses a foreign `Origin` with 403. `mcp.allowed_origins` is required and non-empty                                             | **built** 2026-10-26: `a_foreign_origin_is_refused`. The one allowed origin is the deployment's own, derived from `public_base_url`                       |
| A token in the query string leaks into access logs                                                  | Information disclosure | cratestack-mcp answers `?access_token=` with 400 before the provider runs, and strips the token before `rmcp` sees the request                                               | **upstream**, not re-asserted here: cratestack-mcp's own `http_hardening` tests                                                                           |
| A request reaches the tool handler without passing the guard (a layer mounted around it by mistake) | Elevation of privilege | cratestack-mcp answers `-32603` for any method that lacks the guard's caller. vpay mounts the service with `nest_service` only, as in the crate's own example                | **structure, not a test**: vpay mounts only the router `vpay-db`'s `merchant_mcp_router` returns, which nests CrateStack's service with nothing around it |
| A deployment gains the surface without deciding to                                                  | Elevation of privilege | `mcp` and `mcp_staff` are not in `EnabledSurfaces::ALL`, so an absent `surfaces` key never mounts them                                                                       | **built** 2026-10-26: `an_absent_surfaces_key_mounts_no_mcp` (integration), `mcp_is_mounted_only_when_named` (`vpay-config`)                              |
| Flooding the endpoint exhausts the connection budget                                                | Denial of service      | ADR-0022 §3's connection-budget guard counts the MCP workloads. **There is no per-merchant rate limit on MCP**, as there is none on `/v1`                                    | **not built** (the chart guard, ADR-0033 step 6); the rate limit is a **gap**, recorded                                                                   |

### 2. Tokens, agent host → the authorization servers

| Threat                                                                                                                            | Kind                   | Mitigation                                                                                                                  | State                                                                                                                                                                                                                                                                                    |
| --------------------------------------------------------------------------------------------------------------------------------- | ---------------------- | --------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A client that never opted in obtains an MCP-audience token                                                                        | Elevation of privilege | The OP mints an audience only if the client's `allowed_audiences` lists it, which is the existing `client_credentials` gate | **built** 2026-10-26: `the_op_mints_an_mcp_token_only_for_a_client_that_opted_in`, through the real OP                                                                                                                                                                                   |
| An MCP client sends RFC 8707 `resource` instead of `audience` and gets a `vpay:v1` token it cannot use, or one it should not have | Tampering              | **Unverified** whether the OP honours `resource` (ADR-0033 D9). It must be settled before the merchant endpoint is built    | **built** 2026-10-26: measured first: the OP dropped `resource` and minted a default-audience token. `op::token::apply_resource_indicator` now makes it the audience, or `invalid_target`. `an_rfc_8707_resource_is_the_audience_it_names` (fails without the fix), and three unit tests |
| The staff MCP client's redirect is captured by another local process                                                              | Spoofing               | PKCE (S256) on a public client, loopback redirects only, a short code lifetime. These are the existing staff OP's rules     | **not built**: no staff endpoint yet                                                                                                                                                                                                                                                     |
| A disabled staff member keeps using a still-valid token                                                                           | Elevation of privilege | The provider re-reads `staff_members` on every request, as `/dash/v1` does (ADR-0017)                                       | **not built**: no staff endpoint yet                                                                                                                                                                                                                                                     |

### 3. Tool calls: what a token can make vpay do

| Threat                                                                                                                                             | Kind                              | Mitigation                                                                                                                                                                                                                                                                                                                                                                             | State                                                                                                                                                                                                                                                                       |
| -------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **A prompt-injected agent creates, confirms or refunds a payment the merchant did not intend**                                                     | Tampering, Elevation of privilege | **vpay cannot see the prompt.** What it bounds is what the token may do: writes need `payments:write`, so a merchant can issue its agent a `payments:read`-only client. Every write needs an idempotency key. Every write lands as an `events` row the merchant's webhooks receive. A per-merchant amount ceiling on MCP writes is **undecided** (ADR-0033 "Left to the maintainer" 2) | **built** 2026-10-26: `a_read_scoped_token_cannot_call_a_mutation` (no row written). The ceiling is still **open**                                                                                                                                                          |
| Text vpay renders into a tool result carries an injected instruction: a customer name, invoice memo or `metadata` value, or a rail's failure prose | Tampering                         | Tool results are structured JSON, never prose vpay composed around the value. Tool descriptions are static schema text, never data. **The agent host is still the only defence against an instruction inside a value.** This row is the reason the ceiling above exists as a question                                                                                                  | **accepted** as residual risk. Structured part **built**: every tool answers `structuredContent` of a declared type (`an_intent_created_over_mcp_is_the_one_v1_serves`)                                                                                                     |
| Merchant A reads or acts on merchant B's objects                                                                                                   | Information disclosure, Tampering | The tenant comes only from the token's context (`tenant_of(ctx)`). No tool takes a `merchant_id`. Resources are filtered in SQL by `@@allow("read", auth().tenant == merchant_id)`. Absent and forbidden answer the same "not found"                                                                                                                                                   | **built** 2026-10-26: `merchant_b_cannot_read_or_cancel_merchant_as_intent`, `the_refund_tools_reach_v1s_refund_code`. Tools only; no resources exist yet                                                                                                                   |
| A retried call charges twice                                                                                                                       | Tampering                         | The `idempotencyKey` argument is required on every mutation. vpay's own per-merchant store returns the first result; the same key with different arguments is refused                                                                                                                                                                                                                  | **built** 2026-10-26: `idempotency_is_vpays_own_and_mandatory` (replay, mismatch, empty key)                                                                                                                                                                                |
| An agent calls a write the policy should refuse, and the implementation runs anyway                                                                | Elevation of privilege            | Every tool reaches its body only through the generated `invoke_with_db`, which needs the `Authorized` witness the policy check builds. A path that skipped `@allow` does not compile (cratestack#512)                                                                                                                                                                                  | **upstream**, and re-asserted by `a_read_scoped_token_cannot_call_a_mutation`                                                                                                                                                                                               |
| The staff endpoint is used to move money                                                                                                           | Elevation of privilege            | ADR-0008: staff tools are reads only. The upstream audience feature keeps mutations off the staff listing, and policy refuses them as well                                                                                                                                                                                                                                             | **not built** for the staff endpoint (none exists). Its neighbour is **built**: `/dash/v1/$procs` answers 404 for every merchant procedure, `a_merchant_mcp_procedure_is_not_served_to_the_dashboard`, which turns `403` (the policy, the inner wall) without the allowlist |
| A refund tool's success is read by the agent as money returned                                                                                     | Repudiation                       | Refund tool descriptions state what `docs/status.md` states: two rails' `refund` is `NotImplemented`, and nothing settles a `pending` refund                                                                                                                                                                                                                                           | **built** 2026-10-26: `the_endpoint_lists_exactly_the_step_3_tools` asserts `create_refund`'s description                                                                                                                                                                   |

### 4. Resources and results leaving vpay: the model's context

| Threat                                                                                                                                        | Kind                   | Mitigation                                                                                                                                                                                                                               | State                                                                                                                                                                                                                                                           |
| --------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Payer personal data (MSISDN, customer name, email) enters a third party's model context and logs                                              | Information disclosure | A new `mcp` entry in `schemas/privacy-inventory.yaml`'s `non_db_surfaces`, added in the change that mounts the endpoint. Merchants opt in per client. Erasure in vpay does not reach copies an agent host kept, the same as for webhooks | **built** 2026-10-26: the `merchant_mcp` surface in `schemas/privacy-inventory.yaml`, and `ConfirmRequest`/`NewRefundRequest` registered as `debug_protections` (`a_payers_number_never_reaches_a_debug_line`). The reach of erasure is **accepted** and stated |
| Internal columns reach an agent: `seq`, `client_secret_suffix`, digests, rail references, `charges.provider_ref_extra` (Orange's `pay_token`) | Information disclosure | `@server_only` on every internal column. `provider_ref_extra` is never declared at all. A shape test pins each resource's field list                                                                                                     | **built** 2026-10-26: for tools: an output type declares only `/v1`'s public fields, so `/v1`'s `client_secret` is absent (`an_intent_created_over_mcp_is_the_one_v1_serves`). For resources: **not built**, none exist                                         |
| A resource silently lacks the merchant's `metadata` because `jsonb` is unmapped, and the agent acts on an incomplete object                   | Tampering              | ADR-0033 D5: no money-model resource ships without lossless `jsonb`. Until then those reads are tools over vpay's renderer                                                                                                               | **not reachable**: step 3 ships no resources, and D5's measurement has not been made                                                                                                                                                                            |
| A tool error leaks internals                                                                                                                  | Information disclosure | cratestack-mcp's error text is exactly the REST envelope (`code`, `message`), and vpay's public messages carry no hosts, tables or library text (ADR-0011)                                                                               | **built** 2026-10-26: a refusal is `/v1`'s public `code`/`message`/`param` only (`a_v1_refusal_keeps_its_code_message_and_param`, `a_refusal_keeps_v1s_code_and_param_and_its_status_class`)                                                                    |
| One audience's listing reveals the other's tools                                                                                              | Information disclosure | Upstream per-audience exposure (ADR-0033 D2). The staff endpoint does not ship until that exists                                                                                                                                         | **not built**, **blocked upstream**; no second audience exists yet                                                                                                                                                                                              |

## What would change this model

- **An amount ceiling is decided.** Section 3's first row then gains a
  mitigation with a test, rather than an open question.
- **The OP's handling of `resource` is measured.** Section 2's second row
  becomes a mitigation or a refusal.
- **Staff get writes.** That would supersede ADR-0008 and reopen section 3
  for a second audience.
- **The first real agent session against a running vpay.** Every row is
  design until then.
