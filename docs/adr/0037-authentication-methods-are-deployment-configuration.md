# ADR-0037: Authentication methods are deployment configuration

- **Status:** Accepted. **Supersedes in part**
  [ADR-0035](0035-v1-is-a-signed-message-not-a-token.md): decision 1's "there is
  no optional or compatibility mode, no token fallback" and decision 6's flag
  day hold for a deployment that permits only `keypair`, and no longer hold for
  one that permits more. [ADR-0036](0036-the-console-is-a-signed-message-under-a-paired-key.md)
  is untouched except for what decision 8 adds: staff sign-in returns under
  `human_login` (VPA-3), and staff SSO with it (VPA-4), beside the console key
  and not in its place.
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
   _Served_ is _permitted_, except that `human_login` is listed only when both of
   its blocks are present (`AuthMethods::served`): it says what the deployment
   offers, not what any one merchant can do. A console reads it instead of a
   second switch that could disagree.
8. **`human_login` is the staff sign-in, and it follows `machine_tokens`' rules
   exactly (VPA-3).** Its blocks are `staff_auth` (the password pepper and the
   TOTP key) and `dashboard_client` (the authorization-code client). It is
   _served_ when it is permitted and both blocks are written
   (`AuthMethods::human_login_served`); it is the one list of states there is:

   | `human_login` | `staff_auth` + `dashboard_client` | Result                                                                                                     |
   | ------------- | --------------------------------- | ---------------------------------------------------------------------------------------------------------- |
   | permitted     | both                              | served: sign-in routes mounted, `/dash/v1` runs `Optional`, `staff add` mints a one-time password          |
   | permitted     | neither, by the default           | not served: the routes are `404`, `/dash/v1` is `Required`, `staff add` writes a member and no credential  |
   | named         | neither, or only one              | refused at boot, `AuthMethodNotConfigured`                                                                 |
   | permitted     | exactly one, by the default       | refused at boot, `AuthMethodNotConfigured`: half a sign-in is the dormant configuration decision 4 refuses |
   | not permitted | either                            | refused at boot, `AuthMethodDisabled`, naming the block to delete                                          |

   `/dash/v1` and the procedure transport beneath it take **a console key or a
   staff session**, one per request. The envelope runs `Optional` iff the method
   is served; otherwise it stays `Required`, so a deployment that never wrote the
   blocks is signed-only (`SurfaceMode::for_dashboard`). With `keypair` off the
   layer is `Off`, as `/v1`'s is for tokens alone. As on `/v1`, the policy is a
   closure with the unresolved mode pinned to `Required`. A request that is
   signed **and** carries a bearer is `400`. A session is a staff access token
   from the authorization-code grant, audience `dashboard_client.client_id`,
   scope the registration's, with the tenant claim equal to the dashboard's;
   it is held to the same member re-read and the same admin override as a key,
   through one resolver. `/svc/v1` stays `Required`.

   **There is no refresh token on this surface**, then or now (ADR-0017: "no
   refresh token on this surface and there is not going to be one"; the owner's
   rulings of 2026-09-29 and 2026-10-09). Q8's "the `refresh_token` grant is
   restored only conditionally" has nothing to restore here: the dashboard
   client's registration carries `authorization_code` alone, the refresh slot of
   the OP is the fail-closed `RefusingRefreshTokenStore`, and a `refresh_token`
   request to `/dash/v1/oauth/token` is a `400` while the method is served and a
   `404` when it is not. A keypair caller never reaches it. Offering a refresh
   grant would be a new design (a store, a rotation rule, a migration), not a
   restoration.

   ~~**Staff SSO is not part of this release** (`staff_auth.sso`, VPA-4): the key
   is an unknown field of `staff_auth` and is refused, not ignored.~~
   **Corrected 2026-10-30 (VPA-4):** staff SSO
   ([ADR-0030](0030-staff-sign-in-federates-to-an-oidc-provider.md)) is restored
   and is a part of this method, not a fifth one. `staff_auth.sso` is a sub-block
   of the block this decision already pairs with `dashboard_client`, so it needs
   no switch of its own and is served exactly when `human_login` is: with the
   method not permitted, the `staff_auth` block that carries it is refused
   (`AuthMethodDisabled`, naming `staff_auth`), as `staff_auth` itself is; with
   the method served and the sub-block absent, the three SSO routes answer
   `enabled: false` and `404`; with it written, `StaffLogin::from_config` builds
   the provider and a provider that cannot be built is a boot refusal, not a log
   line. Password and TOTP stays mounted beside it as the break-glass path.

9. Every request span on `/v1` and `/dash/v1` carries
   `auth.method = keypair | machine_token | human_session`. It is tracing only:
   no policy reads it.

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
