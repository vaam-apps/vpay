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
      |  200 { access_token, id_token, refresh_token? }         |
      |<-------------------------------------------------------|
      |  subsequent /dash/v1/* calls, Authorization: Bearer …   |
      |------------------------------------------------------->|
```

`redirect_uris` are matched exactly — no prefix or wildcard matching
(`authkestra_op::client::ClientRegistration::allows_redirect_uri`). PKCE is
mandatory for the dashboard's client registration.

## Token lifetimes

| Token | TTL knob | Notes |
|---|---|---|
| Authorization code | `OpConfig::authorization_code_ttl_secs` | Single use, consumed at `/token` |
| Access token | `OpConfig::access_token_ttl_secs` | Bearer, presented on every `/dash/v1/*` call |
| Refresh token | Issued via `RefreshTokenStore` | Rotated on use |
| ID token | Same signing key/alg as access token | `RS256`; symmetric algorithms are not valid per Authkestra's own `OpConfig` docs |

**No revocation endpoint exists in `authkestra-op`.** A stolen or misused
access token cannot be revoked mid-lifetime through the OP itself — see the
Consequences section of ADR-0009. This flow's mitigation is short TTLs
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
  fire against real Postgres. **The private key PEM is stored unencrypted**
  (`private_key_pem TEXT`, no encryption at rest implemented anywhere in this
  repository) — anyone able to `SELECT` that column reads the live signing
  key outright. No code generates, writes, reads, or rotates a row in this
  table yet; the schema alone does not make key rotation work.

## Where each piece lives

| Piece | Owner |
|---|---|
| OP handlers (`/authorize`, `/token`, `/userinfo`, discovery, jwks) | `authkestra-op`, mounted into `/dash/v1` |
| Client registration (dashboard's own `client_id`, redirect URIs, PKCE requirement) | vpay configuration (ADR-0003 — YAML, not the dashboard) |
| Authorization codes, refresh tokens, device codes | `authkestra_op::sqlx_store::SqlxOpStore` against vpay's Postgres. Schema exists (`backends/migrations/0006_create-authkestra-op-tables.sql`) and is proven compatible with the store (see Status below), but no shipping code constructs a `SqlxOpStore` yet |
| Signing keys and rotation | vpay operational tooling. Storage schema exists (`backends/migrations/0007_create-oauth-signing-keys.sql`: `oauth_signing_keys`, at most one active key enforced by a partial unique index); key generation and rotation logic itself is not yet designed or written |
| Session → per-record authorization (which staff member may refund which merchant's charge) | vpay's own layer on top of the validated token; not Authkestra's concern |
| Audit log row per write | ADR-0008 — one row per dashboard action, independent of the auth mechanism |

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
