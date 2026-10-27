# ADR-0037: Authentication methods are deployment configuration

- **Status:** Accepted. **Supersedes in part**
  [ADR-0035](0035-v1-is-a-signed-message-not-a-token.md): decision 1's "there is
  no optional or compatibility mode, no token fallback" and decision 6's flag
  day hold for a deployment that permits only `keypair`, and no longer hold for
  one that permits more. [ADR-0036](0036-the-console-is-a-signed-message-under-a-paired-key.md)
  is untouched in this release; staff sign-in returns under `human_login` in
  VPA-3.
- **Date:** 2026-10-27
- **Deciders:** the owner (ruling R5, 2026-10-27): _vpay serves other apps
  besides vaam. Which ways of authenticating a deployment accepts is
  deployment configuration, in YAML. Absent, all four._

## Context

[ADR-0035](0035-v1-is-a-signed-message-not-a-token.md) and
[ADR-0036](0036-the-console-is-a-signed-message-under-a-paired-key.md) made the
signed message the only credential of vpay, because vaam's ruling is
_keypairs everywhere_. vpay is not only vaam's. Another application's
deployment may hold merchants that integrated with a bearer token, staff who
sign in with a password, and receivers that verify an HMAC; deleting those doors
for every deployment because one product forbids them is a decision about
vaam, taken in a repository that serves others.

## Decision

1. `auth.methods` in the YAML lists the ways a deployment authenticates:
   `keypair`, `machine_tokens`, `hmac_webhooks`, `human_login`. **Absent means
   all four.** An empty list, an unknown name and a duplicate are refused at
   boot. vaam sets `[keypair]` (plus `hmac_webhooks` until sealed webhooks,
   VP-8).
2. **A deployment may turn `keypair` off.** A legacy-only deployment is
   supported; the `/v1` envelope runs `Off`, and the console, which is signed,
   is not available there. At least one way into `/v1` must remain: with
   `keypair` off, `machine_tokens` must be on (`ConfigError::NoWayIn`).
3. **`/v1` takes both credentials when both are permitted**, one per request:

   | `keypair` | `machine_tokens` | Envelope mode | An unsigned request                               |
   | --------- | ---------------- | ------------- | ------------------------------------------------- |
   | yes       | no               | `Required`    | the layer's coarse `401`                          |
   | yes       | yes              | `Optional`    | validated as a bearer token                       |
   | no        | yes              | `Off`         | validated as a bearer token; a COSE body is `415` |

   A request that is signed **and** carries an `Authorization` header is `400`:
   one caller, one credential. A signed caller gets a sealed answer, a token
   caller a plain one. Under `Optional` the policy is a closure with the
   unresolved mode pinned to `Required`, so a route the resolver cannot bind
   still fails closed.

4. **Permitted is not served, and a block without its method is refused.** A
   method that needs a block runs only where the block is; a block present for a
   method that is not permitted is refused at boot
   (`ConfigError::AuthMethodDisabled`), and a method named explicitly whose
   block is missing is refused too (`AuthMethodNotConfigured`). Dormant
   configuration is how an operator comes to believe a door is shut. The
   `deployment.surfaces` entry `mcp` requires `machine_tokens`
   (`McpNeedsMachineTokens`): the MCP endpoint is bearer-only.
5. **`/v1/oauth/token` exists iff `machine_tokens` is permitted** on a process
   that mounts the business surface. Otherwise it is the honest `404`. The
   JWKS and discovery documents are mounted where something reads them.
6. **`hmac_webhooks` off means no merchant webhooks**: an endpoint with `secrets`
   is a boot refusal. VP-8 adds sealed delivery per endpoint; until then there
   is no other mode.
7. `GET /.well-known/vpay-auth` answers `{"methods": [...]}`, unauthenticated
   and outside every envelope: the methods this deployment permits and serves.
   A console reads it instead of a second switch that could disagree.
8. `human_login` is **permitted by the default and served by nothing** in this
   release. Naming it explicitly is refused ("not available in this release");
   `staff_auth` and `dashboard_client` stay retired keys. VPA-3 restores it.
9. Every `/v1` request span carries `auth.method = keypair | machine_token`.
   It is tracing only: no policy reads it.

## Consequences

- vaam's behaviour is unchanged once it sets `methods: [keypair]` and deletes
  the OAuth half (`jwks`, `grant_types`, `allowed_audiences`) of its merchant
  registrations, which the new rule refuses to leave dormant. Until it does,
  the default permits tokens it never uses.
- A token caller gets no contract-lock protection (the `426` for an operation
  whose shape is gone applies to sealed callers), no sealed answers, and a
  rate-limit and idempotency namespace of its own, keyed on its token rather
  than its key. A retry that switches credential does not deduplicate.
- A token minted before a client is disabled stays valid until it expires; a
  signed caller is cut off on its next request. That is the behaviour before
  [ADR-0035](0035-v1-is-a-signed-message-not-a-token.md), restored.
- The SDKs still only seal. Their token modes are VPA-6.
- A legacy-only deployment still loads the envelope response keys at boot (the
  layer needs a signer to be built, even `Off`); removing that is a later change.
