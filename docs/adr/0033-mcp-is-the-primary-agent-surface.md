# ADR-0033: MCP is the primary agent surface

- **Status:** **Proposed.** Nothing this ADR decides is built. The
  maintainer gave the direction below on 2026-10-26, in a planning session.
  That direction is an instruction to design and build, not a review of this
  text.
  - _What the maintainer said:_ the MCP backend is a new **surface** on the
    one `vpay-server` image (ADR-0022), not a new binary. It serves **both**
    merchants and staff. It carries **reads and money-moving writes**. The
    transport is **Streamable HTTP**. Asked why the first draft left out
    `@@mcp(resource)`, the maintainer answered: "We're in the era of ai, so
    normally, no real user will even use again the rest api. MCP should be
    the most complete possible." Two more choices came from that session:
    the two audiences are separated by an **upstream CrateStack feature**,
    and staff stay **read-only**.
  - _What acceptance would rest on:_ D9's verification list, run against
    real Postgres and a real MCP client, on a dated page under
    [status/verification/](../status/verification/).
  - _Amended 2026-10-26, by step 3 (the first code)._ What building it
    found, recorded here rather than by editing the decisions below:
    - **No `mcp:` config block.** D9's block is not needed. The resource
      identifier is `{deployment.public_base_url}/mcp/v1`, the one allowed
      browser origin is that URL's own, and the authorization server is the
      OP's issuer. All three are derived from what the deployment already
      says about itself (`vpay_api::mcp::merchant_resource`).
    - **RFC 8707 `resource` is honoured.** D9 called this unverified. It was
      measured: the OP dropped `resource` and minted a token with the default
      audience, which `/mcp/v1` then refused. `vpay_api::op::token` now treats
      one `resource` as the audience it names, through the same
      `allowed_audiences` gate, and answers `invalid_target` for two targets.
    - **No resources in step 3.** D5's `jsonb` measurement has not been made,
      so no model carries `@@mcp(resource)` and the money models' pinned
      tests are unchanged.
    - **`/dash/v1/$procs` needed an allowlist** (D6). CrateStack's generated
      procedure router mounts every procedure, and 0.16 has no attribute to
      keep one off REST.
    - **Step 2 was narrowed** to the six payment-intent and refund writes, and
      the port arrived in step 3 with its first caller.
- **Date:** 2026-10-26
- **Deciders:** the vpay maintainer (the direction above) and the
  implementing agent (D1–D10 as written).
- **Extends:** [ADR-0022](0022-surface-isolation-and-independent-scaling.md),
  which is itself still Proposed. This ADR adds two `deployment.surfaces`
  values and two workloads, and changes nothing ADR-0022 decided.
- **Keeps:** [ADR-0008](0008-dashboard-scope.md): the dashboard observes and
  never administers. The staff MCP endpoint is read-only for that reason.
  Also kept: [ADR-0010](0010-merchant-auth-private-key-jwt.md) (merchants
  authenticate with `private_key_jwt`) and
  [ADR-0017](0017-staff-authentication.md) (staff authorisation re-reads the
  staff row on every request).

  **Both were superseded under this ADR while it was Proposed.**
  - [ADR-0035](0035-v1-is-a-signed-message-not-a-token.md) (#380; dated
    2026-10-26, merged 2026-10-27) superseded ADR-0010 for `/v1`. The `private_key_jwt` grant survives only
    as the way `/mcp/v1` gets its tokens (D9).
  - [ADR-0036](0036-the-console-is-a-signed-message-under-a-paired-key.md)
    (#383; dated 2026-10-26, merged 2026-10-27) superseded ADR-0017 entirely. It deleted the staff OP,
    the staff sessions and the PKCE client D9's staff design relies on. See
    "Left to the maintainer" item 5.

- **Amends:** two conventions that are not ADRs but are pinned by tests and
  comments. "CrateStack lives in `vpay-db` and nowhere else" is unchanged,
  and so is its privacy. The conventions that move are "no money model
  carries an `@@allow` arm" and "every read arm is `auth().isSystem()`"
  (D5).
- **Threat model:** [security/threat-models/mcp.md](../security/threat-models/mcp.md),
  written with this ADR and before any of its code, as
  [security/README.md](../security/README.md) requires.
- **Number checked at branch time, not assumed:** `docs/adr/` on `master`
  at `172a13a` runs `0001` to `0032`.

## Context

CrateStack 0.16.0 (2026-10-23) ships `cratestack-mcp`. Applications reach it
through the `mcp` feature of the `cratestack-pg` and `cratestack-api`
facades, never directly:

- **What it serves.** A schema's `@mcp(tool)` procedures, and its
  `@@mcp(resource: "…")` models (`cratestack-pg` only), over MCP revision
  `2026-07-28`.
- **How a call runs.** Every tool call reaches its procedure through the
  generated `invoke_with_db`, so `@allow` and `@deny` run before the
  implementation. Every resource read is the same ORM call REST's `GET`
  handlers make, so `@@allow("read", …)` is in its SQL.
- **Streamable HTTP.** It mounts on axum behind the application's own
  `AuthProvider`. It serves RFC 9728 protected-resource metadata, refuses a
  foreign `Origin` with 403, and answers a missing or rejected token with a
  401 challenge. It never serves an anonymous context.
- **Listings are static.** `tools/list` and `resources/list` come from one
  table generated per schema (`cratestack-mcp/src/listing.rs`). They are not
  filtered by caller.

The maintainer's direction is that agents, not people, are vpay's callers
from now on, and that MCP should therefore be as complete a surface as
vpay can make it. Four facts about the repository decide how:

1. **CrateStack is private to `vpay-db`.** The generated module sits behind
   a private `mod schema` (`backends/crates/vpay-db/src/schema.rs`), and
   `verify-repositories` fails if it leaks. The one precedent for letting a
   generated surface out is `Repositories::dashboard_procedure_router`: it
   returns an axum `Router` built inside the crate, authenticated by an
   `AuthProvider` that reads the tenant from request extensions
   (`dashboard_transport.rs`).
2. **Only `cratestack-pg` serves resources.** A complete surface, with both
   tools and resources, can therefore only come from `schemas/vpay.cstack`.
   Its models were rewritten field-for-field against the live tables in S5
   (2026-09-07).
3. **The writes cannot be called from `vpay-db`.** Creating, confirming and
   cancelling an intent, creating and cancelling a refund, and the customer,
   checkout-session and invoice writes are `pub(crate)` axum handlers in
   `vpay-api`. They need the rail adapters and `ResourceConfig`, which sit
   above `vpay-db`. Their idempotency is bound to the HTTP request
   (`PostRequest` hashes method, path and form body).
4. **Today's model policies make every resource read empty.** PaymentIntent,
   Charge and Refund carry **no `@@allow` arm**, pinned by
   `the_three_money_models_answer_no_rows_to_every_action`. Their `jsonb`
   columns (`metadata`, `payment_method_types`) are not modelled. Every
   other model's read arm is `auth().isSystem()` only, so a merchant or
   staff context reads nothing.

## Decision

### D1. One MCP schema: `schemas/vpay.cstack`

`vpay.cstack` declares `mcp { name = "vpay" expose = [tools, resources] }`,
and vpay-db's `cratestack` dependency gains the `mcp` feature. No second
schema and no second facade is introduced. CrateStack stays a dependency of
`vpay-db` alone, and the generated `mcp` module stays private.

vpay-db exposes the surface the way it already exposes the dashboard's
procedures: one `Repositories` method that returns an axum `Router`, given
an audience, an `AuthProvider`, the allowed origins, the protected-resource
identifier and the D3 port. `verify-repositories` is extended so that a
`pub` path to `schema::mcp` fails it, as a `pub` path to `schema` already
does.

**Rejected: a procedures-only `db = none` schema in `vpay-api`.** It would
avoid D3's bridge, but it cannot hold resources, and resources are what the
maintainer asked for. It would also make `vpay-api` a second CrateStack
consumer, and two generated tables would need a hand-written combiner to
share one endpoint.

### D2. Two audiences, two endpoints, two surfaces, opt-in only

|                             | Merchant                                                    | Staff                                        |
| --------------------------- | ----------------------------------------------------------- | -------------------------------------------- |
| `deployment.surfaces` value | `mcp`                                                       | `mcp_staff`                                  |
| Endpoint                    | `POST /mcp/v1`                                              | `POST /dash/mcp/v1`                          |
| Tier (ADR-0022 §2)          | autoscaled, like `-server`                                  | fixed 2 replicas, like `-management`         |
| Tools                       | every `/v1` merchant operation, reads and writes            | every read (D6)                              |
| Resources                   | every merchant-facing model, scoped to the token's merchant | the same, scoped to the dashboard's merchant |

**Neither value is part of `EnabledSurfaces::ALL`.** An absent
`deployment.surfaces` keeps meaning what it has meant since ADR-0022:
"serve what you always served". A new internet-facing interface is never
gained by upgrading. An MCP-only backend is a pod configured
`surfaces: [mcp]`; `[mcp, mcp_staff]` on one pod is legal for a small
deployment.

**Separating the audiences is an upstream prerequisite.** One schema
produces one static table, so without help both endpoints would list every
tool and resource, and policy would merely refuse the other audience's
calls. The maintainer chose to add per-audience exposure to CrateStack
itself:

- `audience: [...]` on `@mcp(tool)` and `@@mcp(resource)`, against a
  declared `mcp { audiences = [...] }`;
- a builder option that filters both listings;
- a call or read outside the audience answered like an unknown name.

**Until that ships, only the merchant endpoint ships.** A filtering wrapper
around the generated table in vpay was rejected: it would be glue written by
hand in the security path, around a `&'static` slice.

### D3. Writes are procedures whose bodies reach the rails through a port

Each write is a `mutation procedure` in `vpay.cstack` with `@mcp(tool: …)`.
Its body lives in vpay-db's `impl procedures::ProcedureRegistry for
Payments`, as the five `search*` bodies do, and it delegates to an
`Arc<dyn MerchantOperations>`:

- **The trait.** vpay-db defines it, with plain request and result types.
- **The implementation.** `vpay-api` implements it over a transport-free
  service layer, extracted from the handlers, which the REST handlers then
  call as well.
- **The direction.** This is the only place `vpay-db` calls upward, and it
  does so only through a trait it owns. That is the same shape ADR-0002
  gave the rails.

Reads that need no rail are implemented directly in vpay-db, as the
`search*` procedures are.

### D4. Tenancy and scope come from the token, never from arguments

- **Tenancy.** Every procedure body takes the tenant from the context, as
  `tenant_of(ctx)` already does in `schema/search_payment_intents.rs`. No
  tool takes a `merchant_id` argument.
- **Scope.** Tool policy mirrors `v1::required_scopes`: a read needs
  `payments:read` or `payments:write`, a write needs `payments:write`.
- **Audience.** Both reads and writes also need the merchant audience on
  the context.

### D5. Resources: tenant read arms, server-only internals, the money models opened deliberately

- **Read arms.** Each merchant-facing model gains
  `@@allow("read", auth().tenant == merchant_id)` beside its existing
  `isSystem()` arms. The arm admits reads and nothing else.
- **Internal columns.** These are marked `@server_only`, so a resource never
  carries them: `seq`, `client_secret_suffix`, every digest and every
  rail-side reference.
- **The `jsonb` columns are the hard part, and they decide whether the
  money models get resources at all.**
  [reference/vpay-db/cratestack-money-tables.md](../reference/vpay-db/cratestack-money-tables.md)
  records why S5 left them unmodelled, and the reason still stands until
  measured otherwise. There are two problems:
  - CrateStack's `Value::from_plain_json` turned a `u64` above `i64::MAX`
    inside merchant-authored `metadata` into a float.
  - Declaring `metadata Json` added a blocking drift line while leaving the
    live column just as invisible to the comparison.

  The rule is:
  - If 0.16 maps `jsonb` losslessly in both directions, `metadata` and
    `payment_method_types` are declared.
  - If it does not, a money resource **without** `metadata` would be an
    object that silently lacks the merchant's own data. So the money models
    get **no** resource, and their reads stay tools over vpay's own renderer.
    That gap is dated on the parity page (D8).
  - `charges.provider_ref_extra` holds rail key material (Orange's
    `pay_token`) and is never declared on any surface.
  - **Tools are stricter than resources, by design upstream.** CrateStack
    0.16 refuses to expose a tool whose input or output contains a `Json`
    field, so a tool can never carry one
    (`cratestack-macros-0.16.0/src/include/mcp_gate/plan.rs`: "a type with
    no faithful mapping … is refused, never advertised as `{}`"). `/v1`
    metadata is a string-to-string map (`BTreeMap<String, String>` in
    `v1/payment_intents.rs` and `v1/refunds.rs`), so a tool carries it as
    `type MetadataEntry { key String value String }[]`. That mapping is
    lossless for every value `/v1` accepts. The `u64` precision problem above
    concerns the stored `jsonb`, not this input.

- **The two pinned tests.**
  `the_three_money_models_answer_no_rows_to_every_action` and
  `no_generated_read_on_a_money_table_can_carry_its_jsonb_column` change
  only in the change that adds a money model's read arm. The first test's
  own message asks for exactly that: "if a query moved, move this assertion
  with it". The reference page above records the move. **Only read arms are
  added.** No create, update or delete arm is added to a money model, and
  the replacement test pins that.
- **Out of scope.** Staff, credential, session and OAuth models, rate-limit
  windows and webhook deliveries are not resources. `LedgerTransaction` and
  `LedgerEntry` are not resources until a tenancy-safe ledger read exists
  (`Ledger::merchant_payable_balance` warns that it is not one).

### D6. Staff are read-only

ADR-0008 stands. The staff endpoint lists the five `search*` procedures,
the retrieve procedures and the resources, and no `mutation procedure`. The
upstream audience feature is what keeps the merchant writes out of the
staff listing. Policy refuses them as well, so a staff context that somehow
reached a write would still be refused.

### D7. Idempotency is vpay's own, and mandatory

Every mutation tool takes a **required `idempotencyKey` argument**, because
`/v1` requires `Idempotency-Key` on every `POST`. A required argument
appears in the tool's input schema; a key carried in `_meta` would not, and
an agent would not know to send one.

- **One store.** Mutation procedures carry `@no_idempotency`, and the
  server is built without an `OpExecutor`, so CrateStack reserves nothing.
  The key is claimed in vpay's own per-merchant store (`vpay_db::Idempotency`).
- **The fingerprint.** It is `("MCP", "tool/<name>", sha256(canonical
arguments))`.
- **Replay and mismatch.** A replay returns the stored result. The same key
  with different arguments is the same mismatch refusal REST gives. A key
  reused across REST and MCP for different requests is refused for the
  same reason.

### D8. Completeness is a gate, not a promise

A new gate, `verify-mcp-parity`, reads a table in `docs/api/mcp-parity.md`.
It checks that every `V1_ROUTES` row, every `DASH_ROUTES` row and every
`search*` procedure maps to an MCP tool or resource, **or** to a dated
exclusion with a reason. It fails in both directions: a route with no row,
and a row naming a tool the schema does not declare.

The permanent exclusions are:

- `/v1/browser/*`: the payer's checkout, which is not an agent's surface;
- `/provider/*`: rail callbacks;
- the OAuth OP's own routes.

**REST is not removed.** The merchant SDKs and the browser checkout depend
on `/v1` and `/v1/browser`. An MCP-only pod simply does not mount them.

Refund tools say in their descriptions what `docs/status.md` says about
refunds: `orange_money::refund` and `notchpay::refund` are `NotImplemented`,
and nothing settles a `pending` refund. **No balance tool exists** until a
tenancy-safe ledger read does.

### D9. Authentication

- **Merchants.**
  - A merchant opts in by listing the merchant MCP resource identifier in
    its `allowed_audiences`, beside `vpay:v1`, and requesting a token for
    it from `/v1/oauth/token`. That is the gate the OP already applies to
    `client_credentials`.
  - Whether the OP also honours RFC 8707's `resource` parameter, which MCP
    clients send, is **unverified** and is checked before the merchant
    endpoint is built.
  - The endpoint's `AuthProvider` validates the token with the existing
    `JwtValidator`, with the audience equal to the resource identifier. It
    maps the subject to a merchant through `ResourceConfig::merchant_id_for`
    and builds a principal context carrying the tenant, the scopes and the
    audience.
  - An `aud = vpay:v1` token is refused on `/mcp/v1`, and an MCP-audience
    token is refused on `/v1`. Since VP-3 (#380, merged 2026-10-27), `/v1` refuses
    every bearer token, because every `/v1` request is signed; the second
    half holds for that reason now.
  - The token endpoint stays on the business tier (ADR-0022 §2). The
    discovery routes mount on `mcp` pods too, so the loopback JWKS validator
    works there.
- **Staff.** **Superseded when #383 merged on 2026-10-27, and not replaced.** The two bullets
  below assume ADR-0017's staff OP. #383 (ADR-0036) deleted that OP: the
  console now signs every `/dash/v1` request under a paired P-256 key, and no
  staff token exists. They are kept as what was designed, not as a plan. How
  a staff member's agent authenticates to `/dash/mcp/v1` is open ("Left to the
  maintainer" item 5).
  - A second staff OP client, a public PKCE client with loopback redirect
    URIs, mints `aud = <staff MCP resource>`.
  - The endpoint's `AuthProvider` performs every check
    `resolve_dashboard_tenancy` performs, including the per-request
    `Staff::find` re-read (ADR-0017). A disabled staff member is refused on
    their next call.
- **Configuration.** A new `mcp:` block holds the two resource identifiers
  and `allowed_origins`, which CrateStack refuses if empty. Boot fails if a
  surface is enabled without its block.

**Verification this ADR requires before it is Accepted.** Real Postgres, and
rmcp's own Streamable HTTP client over a socket, with tokens minted by
vpay's real OP:

- **Tenancy:** merchant A cannot call or read merchant B's objects, and the
  answer is the same "not found" REST gives, for tools and resources alike.
- **Scope:** a `payments:read` token cannot call a mutation, and the
  implementation never runs.
- **Audience:** the cross-audience refusals above hold in both directions.
- **Origin:** a foreign `Origin` gets 403.
- **Idempotency:** replay and mismatch behave as D7 says.
- **Rails:** `confirm_payment_intent` against the WireMock stub rail
  (configured in test YAML, never in the binary) reaches `processing`, and
  the worker settles it.
- **Shape:** each resource and the matching tool output agree with a pinned
  field correspondence to the `/v1` object.
- **Listings:** each audience's listing holds only its own entries.
- **Surfaces:** an absent `surfaces` key serves no `/mcp/v1`.

### D10. Delivery

Each step below is one change, and each lands its status pages, its
verification page and its
[vpay-skills](https://github.com/vaam-apps/vpay-skills) counterpart with
it:

0. This ADR and its threat model.
1. CrateStack 0.15.0 to 0.16.0, with no MCP declared.
2. The service layer (D3), with the `/v1` suites unchanged.
3. The plumbing, plus payment intents and refunds as tools and resources,
   plus D8's gate.
4. The rest of `/v1`.
5. The upstream audience release, then the staff endpoint.
6. The chart's two workloads, inside ADR-0022 §3's connection-budget guard.

## Consequences

- **A new kind of caller.** An agent that reads attacker-controlled text,
  such as a customer name, an invoice memo or a rail's failure prose, can
  be talked into calling a tool. Every control in the threat model exists
  because of that sentence. vpay cannot see the agent's prompt; what it
  controls is what a token may do.
- **Payer data enters a model's context.** MSISDNs, customer names and
  emails rendered in a tool result or a resource leave vpay for whatever
  the agent's host does with them. That is a new disclosure surface. It
  gets an `mcp` entry in `schemas/privacy-inventory.yaml` in the change
  that mounts the endpoint, not before: a registered surface that does not
  exist would be the kind of claim `docs/status.md` warns against.
- **The money models gain their first CrateStack permission.** It is a read
  arm, scoped to the tenant. The test that pinned "no arm" becomes a test
  that pins "a read arm, and nothing else".
- **The parity gate makes MCP lag visible.** A `/v1` route added without a
  tool fails `just verify` until either the tool or a dated exclusion
  exists. That is the intended cost.
- **Gates and counts move.** The gate count moves from 15 to 16 in step 3,
  and every `count:verify-gates` marker and AGENTS.md's gate history move
  with it.

## Left to the maintainer

1. Accepting this ADR, after D9's verification exists.
2. Whether `confirm_payment_intent` and `create_refund` need a per-merchant
   amount ceiling on MCP that `/v1` does not have. The threat model argues
   one; nothing here decides it.
3. Whether staff MCP ever gets writes. That would supersede ADR-0008's
   write boundary, and this ADR does not.
4. What issues `/mcp/v1`'s tokens after VP-4. Since VP-3, the merchant OP's
   `client_credentials` grant serves this endpoint alone, and VP-4 plans to
   retire `jwks` and the token path. MCP's authorization specification needs
   an OAuth authorization server, so either VP-4 keeps the OP for MCP, or
   this endpoint gets another one first. Removing the OP with nothing in its
   place would make `/mcp/v1` unusable, and no gate would notice.
5. How the staff MCP endpoint authenticates, now that #383 (ADR-0036) has
   deleted the staff OP D9 designed it on. MCP's specification expects an
   OAuth bearer token, and ADR-0036's console signs messages under a paired
   key and holds no token. Either a staff agent gets an OAuth client again,
   for this endpoint alone, or the staff endpoint uses the paired-key
   transport, which MCP clients do not speak today. Nothing is built either
   way.
