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

**No login has ever been performed. There is no `/dash/v1` route of any
kind.** That has not changed, and the merchant work of 2026-09-02 makes it
*more* important to say plainly, because several pieces this flow needs now
exist and serve a different surface.

**What exists now and belongs to `/v1`, not here:**

- **Signing keys and a JWKS endpoint are real.** A key is generated
  (`cargo xtask gen-signing-key --out <dir>`, 3072-bit PKCS#8, mode 0600),
  loaded at boot from `--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE`
  with an RFC 7638 thumbprint as its `kid`, announced in `oauth_signing_keys`
  through `vpay_db::ensure_active_signing_key` (one advisory-locked
  transaction, so replicas booting together rotate once between them), and
  published at **`/v1/oauth/jwks.json`** across a 24 h overlap window.
  `vpay-server` refuses to start without a key (exit 78). This is the
  signing-key half of what this flow needs — but the endpoint is on the
  merchant surface, and nothing here consumes it.
- **A shipping binary now constructs `SqlxOpStore<Postgres>`** — as three
  slots the `OpStore` supertrait demands and **no grant reaches**, not as
  anything serving `/dash/v1`. Do not read it as the OP this flow describes
  being wired up.
- The schema is unchanged and still tested against real Postgres:
  `authkestra.oauth_clients`/`oauth_codes`/`oauth_refresh_tokens`/`oauth_device_codes`
  (`0006`, a byte-faithful transcription of `authkestra-op` `=0.3.4`'s own
  DDL), the `=0.7.1` additive delta (`0013`: `oauth_dpop_jti`, and the `jkt`,
  `token_endpoint_auth_method` and `jwks` columns), and `oauth_signing_keys`
  (`0007`, reshaped by `0010`). `authkestra_op_smoke.rs`'s three tests prove
  the real `SqlxOpStore<Postgres>` reads and writes it: `find_client`,
  single-use `store_code`/`consume_code`, `store_token`/`get_token` with
  `jkt`, and `check_and_record_dpop_jti`. **A reader must not infer from any
  of that that a shipping binary can issue a dashboard token.**

**What blocks this flow, specifically:**

1. **No route.** No `/login`, no `/authorize`, no `/userinfo`, no
   `/dash/v1`. `authkestra-axum` is deliberately not a dependency (its
   bundled router mounts endpoints this deployment must not serve and
   publishes a one-key JWKS instead of the rotation window vpay serves), so
   these have to be written, not enabled.
2. **No session store.** `authkestra-engine` is pinned
   `features = ["rustls-no-provider", "token", "session"]` — **without
   `sql-postgres`** — so no SQL-backed session store is compiled into the
   workspace at all. Enabling it pulls `sqlx/chrono` and `sqlx/json` and
   needs `cargo deny` re-run.
3. **An audience problem that must be solved before any of the above.**
   `authkestra-op`'s `default_handle_authorization_code` mints the access
   token with `Some(client_id)` as the audience and has **no
   requested-audience path at all** (`authkestra-op-0.7.1/src/handlers/token.rs`,
   step 7). A token from that grant would carry `aud = <client_id>`, and
   `vpay_api::resource_auth::Surface::Dashboard.audience()` — `vpay:dash/v1`
   — rejects every one of them. `/v1` does not hit this because
   `handle_client_credentials` *does* honour a requested audience. Resolving
   it (a custom grant handler, a different dashboard audience rule, or an
   upstream change) is a maintainer decision, not a default to pick in
   passing.
4. **Key rotation has still never happened.** This flow's own definition of
   done includes rotating a signing key at least once. `TokenManager` holds
   one key for the life of the process; rotation is restart-based, nothing
   re-reads the key file, and a rollback to a retired `kid` is refused.

What is proven about the dashboard *validator*, and no more than that:
`JwtValidator`/`AuthenticatedDashboard` pinned to `Surface::Dashboard`
accepts a correctly-audienced token and rejects a merchant-audienced one
(`a_dashboard_audience_token_is_accepted_by_the_dashboard_validator`,
`a_merchant_audience_token_is_rejected_by_the_dashboard_validator`, unit
tests in `resource_auth.rs`), and the merchant surface rejects a
dashboard-audienced token over a real booted server
(`a_dashboard_audience_token_is_refused_on_v1`). `AuthenticatedDashboard` is
mounted on nothing. `hmac`, `sha2`, `subtle` and `aes-gcm` were listed here
as unused workspace pins; `sha2` gained its first real consumer on
2026-09-02 (the RFC 7638 thumbprint in `vpay_api::op::keys`), and the other
three are still unused.

This flow is now tracked as **Phase 2b** in [`docs/roadmap.md`](../roadmap.md),
split out of Phase 2 when the merchant half landed and this one did not. See
[../status.md](../status.md) for the full, row-by-row picture.
