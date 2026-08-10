# Dashboard authentication

## The invariant

> **No `/dash/v1` request runs without a token vpay itself issued, signed
> with a key vpay itself rotates, against a client vpay itself registered.**

`/dash/v1` never accepts a merchant API key and never federates to an
external IdP. vpay is its own OpenID Provider (OP) for this surface, running
[Authkestra](https://github.com/marcjazz/authkestra)'s `authkestra-op`
in-process. See [ADR-0008](../adr/0008-dashboard-scope.md) for why the
dashboard uses sessions at all, and [ADR-0009](../adr/0009-dashboard-oidc-provider.md)
for why vpay runs the OP itself rather than pointing at someone else's.

## Login flow

Authorization code + PKCE, the only flow this surface uses — no implicit,
no hybrid, no resource-owner-password.

```
staff browser                    /dash/v1 (vpay, as OP)
      |  GET /authorize?response_type=code&code_challenge=…    |
      |------------------------------------------------------->|
      |                          (staff authenticates)          |
      |  302 → redirect_uri?code=…                              |
      |<-------------------------------------------------------|
      |  POST /token  { code, code_verifier, redirect_uri }     |
      |------------------------------------------------------->|
      |  200 { access_token, id_token }                          |
      |<-------------------------------------------------------|
      |  subsequent /dash/v1/* calls, Authorization: Bearer …   |
      |------------------------------------------------------->|
```

`redirect_uris` are matched exactly — no prefix or wildcard matching
(`authkestra_op::client::ClientRegistration::allows_redirect_uri`). PKCE is
mandatory for the dashboard's client registration.

The diagram above intentionally does not show a `refresh_token` in the
`/token` response — see Token lifetimes below.

## Scope

The dashboard's client registration requests exactly **one** OAuth2 scope,
not a scope per action. That follows directly from the dashboard being
**read-only**: it observes state — charges, intents, ledger entries,
adapter health — and performs no mutation today. One scope is all a
read-only surface needs; a finer-grained set only earns its cost once a
second, differently-privileged capability exists to distinguish from the
first.

[ADR-0008](../adr/0008-dashboard-scope.md) — immutable, per this repo's ADR
rule, so it is not edited here — describes a dashboard that also performs
per-record write operations ("re-poll a charge, replay a webhook, issue a
refund, annotate an unresolved charge") with an `audit_log` row per write.
That boundary (records, never configuration) is still the accepted
architecture and this document does not reverse it. What changed, recorded
here rather than in a new ADR because it is a sequencing decision and not an
architectural one: **no mutating dashboard use case is being built now**, so
there is nothing yet to scope beyond read access, and no `audit_log`-writing
code exists. When a real mutating use case lands, it needs its own scope (or
scopes) added to the client registration and its own write path — at that
point ADR-0008's write actions move from described-but-unbuilt to actually
scoped work, not before.

## Token lifetimes

| Token | TTL knob | Notes |
|---|---|---|
| Authorization code | `OpConfig::authorization_code_ttl_secs` | Single use, consumed at `/token` |
| Access token | `OpConfig::access_token_ttl_secs` | Bearer, presented on every `/dash/v1/*` call |
| Refresh token | **Not issued** | vpay does not use `RefreshTokenStore` for this flow. Staff re-run authorization-code + PKCE when the access token expires — a short-TTL access token with no refresh token, rather than a long-lived refresh token that `authkestra-op` has no endpoint to revoke |
| ID token | Same signing key/alg as access token | `RS256`; symmetric algorithms are not valid per Authkestra's own `OpConfig` docs |

**No revocation endpoint exists in `authkestra-op`.** A stolen or misused
access token cannot be revoked mid-lifetime through the OP itself — see the
Consequences section of ADR-0009. Not issuing a refresh token narrows this
exposure rather than closing it: there is no long-lived refresh token to
also protect, but the access token itself is still a live bearer credential
for the whole of its TTL. This flow's mitigation is a short access-token TTL
and/or a deny-list; **which one vpay implements is not yet decided.**

## JWKS publication and key rotation

- vpay publishes `/dash/v1/.well-known/openid-configuration` and
  `/dash/v1/jwks.json`, backed by `authkestra_op::handlers::discovery` and
  `::jwks`.
- Signing keys are `RS256` (asymmetric only, enforced by `OpConfig`).
- Key generation and rotation are vpay's own operational responsibility —
  Authkestra does not ship a rotation policy, a key type, or any key store at
  any published version; it only requires an active key to exist before it
  can issue anything. `oauth_signing_keys` (`backends/migrations/0007_create-oauth-signing-keys.sql`)
  is vpay's own storage for this: at most one active key at a time (a partial
  unique index), an active key may not carry a scheduled expiry, and a
  retired key's expiry must postdate its own creation — all three proven to
  fire against real Postgres. **No private key material is stored at all.**
  Migration `0010_reshape-oauth-signing-keys.sql` dropped the original
  `private_key_pem` column and replaced it with `public_jwk JSONB`: the
  private PEM is to be injected from a Kubernetes Secret at boot and never
  persisted, while the database holds only what `/jwks.json` must publish.
  That is sound because `authkestra_engine::TokenManager::new_asymmetric`
  parses the PEM once at construction and retains only derived keys. No code
  generates, writes, reads, or rotates a row in this table yet; the schema
  alone does not make key rotation work.

## Where each piece lives

| Piece | Owner |
|---|---|
| OP handlers (`/authorize`, `/token`, `/userinfo`, discovery, jwks) | `authkestra-op`, mounted into `/dash/v1` |
| Client registration (dashboard's own `client_id`, redirect URIs, PKCE requirement, single read-only scope) | vpay configuration (ADR-0003 — YAML, not the dashboard) |
| Authorization codes, device codes | `authkestra_op::sqlx_store::SqlxOpStore` against vpay's Postgres. Schema exists (`backends/migrations/0006_create-authkestra-op-tables.sql`) and is proven compatible with the store (see Status below), but no shipping code constructs a `SqlxOpStore` yet |
| `oauth_refresh_tokens`, `oauth_device_codes` | Created by the same migration (`authkestra-op`'s fixed DDL is transcribed wholesale, not column-by-column selected) but structurally unused by this flow: refresh tokens are not issued (Token lifetimes, above) and the device grant is not offered on any client this deployment registers |
| Signing keys and rotation | vpay operational tooling. Storage schema exists (`backends/migrations/0007_create-oauth-signing-keys.sql`: `oauth_signing_keys`, at most one active key enforced by a partial unique index); key generation and rotation logic itself is not yet designed or written |
| Session → per-record authorization (which staff member may view which merchant's records) | vpay's own layer on top of the validated token; not Authkestra's concern. Scoped to *view* today — see Scope, above, for why there is nothing to authorize a write against yet |
| Audit log row per write | [ADR-0008](../adr/0008-dashboard-scope.md) — one row per dashboard action, independent of the auth mechanism. Not yet applicable: there is no write path to log (Scope, above) |

## Status

**No login has ever been performed, no token has ever been issued, and no
signing key has ever been rotated.** There is still no dashboard-auth *code*
in this repository, and no `/dash/v1` route exists at all — the HTTP surface
today is `/healthz` plus a Stripe-shaped 404 fallback
(`backends/crates/vpay-api/src/lib.rs`). `hmac`, `sha2`, `subtle` and
`aes-gcm` still exist only as unused version pins in the workspace
`Cargo.toml`.

What changed since this line last said "no crate depends on `authkestra` or
`authkestra-op`": that is no longer accurate, and restating it here would be
the exact kind of stale claim this repository's rules exist to prevent.

- The schema this flow needs now exists and is tested against real Postgres:
  `authkestra.oauth_clients`/`oauth_codes`/`oauth_refresh_tokens`/`oauth_device_codes`
  (`backends/migrations/0006_create-authkestra-op-tables.sql`, a byte-faithful
  transcription of `authkestra-op` `=0.3.4`'s own hardcoded DDL) and
  `oauth_signing_keys` (`backends/migrations/0007_create-oauth-signing-keys.sql`,
  vpay's own — Authkestra ships no signing-key type at all). See
  [../status.md](../status.md) for the constraint-by-constraint test list.
- `authkestra-op` and `authkestra-engine` are now real dependencies —
  **but only as `[dev-dependencies]` of `vpay-tests-integration`**
  (`backends/tests/integration/Cargo.toml`), used solely by one acceptance
  test, `backends/tests/integration/tests/authkestra_op_smoke.rs`, which
  proves migration `0006`'s schema is genuinely readable/writable by the real
  `SqlxOpStore<Postgres>` (insert a client, `find_client`, `store_code`,
  `consume_code` twice and observe single-use enforcement fire). **Neither
  `vpay-server` nor `vpay-worker-bin` depends on `authkestra` in any form.**
  A reader must not infer from the presence of these crates in `Cargo.lock`
  that any shipping binary can issue a token — it cannot; nothing constructs
  a `SqlxOpStore` outside that one test.

This flow is still blocked on the same prerequisite as before, unchanged by
the new migrations: the database *schema* exists, but there is still no
database *connectivity* layer anywhere in this workspace — no connection
pool, no query/repository code reachable from `vpay-server` — and
`authkestra_op::sqlx_store::SqlxOpStore` needs one to persist anything from a
real request. Key generation and rotation logic also does not exist; only
its storage table does. See [../status.md](../status.md) for the full,
row-by-row picture of what is and is not built.
