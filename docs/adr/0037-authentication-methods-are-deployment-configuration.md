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
   all four.** An empty list, a null (`methods:` with every item commented
   out; it is _not_ the absent key), an unknown name and a duplicate are refused
   at boot, and so is a near-miss top-level key (`auth_methods:`,
   `authentication:`) that `Config` would otherwise drop in silence. vaam sets
   `[keypair]` (plus `hmac_webhooks` until sealed webhooks, VP-8).
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
   caller a plain one: under `Optional` the response seal policy is "never", so
   an unsigned request that asks for a sealed answer (`Cratestack-Nonce` plus
   `Accept: application/cose`, which cratestack's default policy would grant)
   still gets plain JSON, since that seal binds a nonce and a payload and not
   the caller. Under `Optional` the policy is a closure with the unresolved mode
   pinned to `Required`, so a route the resolver cannot bind still fails closed.

4. **Permitted is not served, and a block without its method is refused.** A
   method that needs a block runs only where the block is; a block present for a
   method that is not permitted is refused at boot
   (`ConfigError::AuthMethodDisabled`), and a method named explicitly whose
   block is missing is refused too (`AuthMethodNotConfigured`). Dormant
   configuration is how an operator comes to believe a door is shut. The
   `deployment.surfaces` entry `mcp` requires `machine_tokens`
   (`McpNeedsMachineTokens`): the MCP endpoint is bearer-only.

   **A merchant needs a credential, and the OAuth half is its own.** Every
   merchant registers `cose_keys` or a `jwks` (`MerchantNoCredential`). The three
   of `jwks`, `grant_types` and `allowed_audiences` are required together, and
   only for a merchant that has a `jwks` while `machine_tokens` is permitted: a
   merchant with `cose_keys` alone boots under the default and cannot use a
   token.

5. **`/v1/oauth/token` exists iff `machine_tokens` is permitted** on a process
   that mounts the business surface. Otherwise it is the honest `404`. The
   JWKS and discovery documents are mounted where something reads them.
6. **`hmac_webhooks` off means no merchant webhooks**: an endpoint with `secrets`
   is a boot refusal. VP-8 adds sealed delivery per endpoint; until then there
   is no other mode.
7. `GET /.well-known/vpay-auth` answers `{"methods": [...]}`, unauthenticated
   and outside every envelope: the methods this deployment permits and serves.
   _Served_ is _permitted minus `human_login`_ and nothing more (`AuthMethods::served`):
   it says what the deployment offers, not what any one merchant can do. A
   console reads it instead of a second switch that could disagree.
8. `human_login` is **permitted by the default and served by nothing** in this
   release. Naming it explicitly is refused ("not available in this release");
   `staff_auth` and `dashboard_client` stay retired keys. VPA-3 restores it.
9. Every `/v1` request span carries `auth.method = keypair | machine_token`.
   It is tracing only: no policy reads it.

## Consequences

- vaam's behaviour is unchanged once it sets `methods: [keypair, hmac_webhooks]`
  (decision 1: `hmac_webhooks` stays until sealed webhooks, VP-8, since its
  merchants' endpoints carry `secrets`), after registering its Ed25519 key under
  `cose_keys`, and deletes the OAuth half (`jwks`, `grant_types`,
  `allowed_audiences`) of its merchant registrations, which the new rule refuses
  to leave dormant. Until it does, the default permits tokens it never uses.
- A token caller gets no contract-lock protection (the `426` for an operation
  whose shape is gone applies to sealed callers) and no sealed answers. It does
  **not** get a namespace of its own: `Idempotency::claim` is keyed on
  `(merchant_id, Idempotency-Key)` and never sees the credential, so a retry
  that switches from a signature to a bearer token (or back) deduplicates, and
  the same key with another body is the `400` `idempotency_key_in_use` either
  way. No `/v1` handler keys a rate limit on the credential either.
- `disable_client` is instant on both doors. A signed caller's key resolves to
  nothing the next request; a token caller's `client_id` is checked against
  `disabled_clients` (`401` `invalid_token`) in `/v1`'s scope step and in the
  merchant MCP endpoint's authenticator, so a token minted before the switch
  flipped does not outlive it on either. A kill switch that cannot answer is an
  error, never "not disabled". (The `403` for a genuine token naming a client
  with no registration is unchanged, and is answered from memory before the
  switch is asked.) A token is also held to its registration's **current** YAML
  `scopes`, the source a signer is held to: a scope removed from the YAML and
  applied by a restart stops working for tokens minted before it.
- **Upgrade note: a signed request that also carries `Authorization` is `400`.**
  Before this ADR `/v1` never read the header on a signed request, so a client or
  proxy that added a bearer header to one was ignored. It is now refused on every
  deployment, `[keypair]` included, because one caller presents one credential
  (decision 3). Strip the header before upgrading; the vpay SDKs never set one.
- The SDKs still only seal. Their token modes are VPA-6.
- A legacy-only deployment still loads the envelope response keys at boot (the
  layer needs a signer to be built, even `Off`); removing that is a later change.
