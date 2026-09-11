# Dashboard auth — scope, token lifetimes, JWKS publication, and where each piece lives

_Split out of [docs/flows/dashboard-auth.md](../dashboard-auth.md) on 2026-09-11 by exp57, which broke a 732-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## Scope

The dashboard's client registration requests exactly **one** OAuth2 scope,
not a scope per action. That follows directly from the dashboard being
**read-only**: it observes state — charges, intents, ledger entries,
adapter health — and performs no mutation today. One scope is all a
read-only surface needs; a finer-grained set only earns its cost once a
second, differently-privileged capability exists to distinguish from the
first.

[ADR-0008](../../adr/0008-dashboard-scope.md) — immutable, per this repo's ADR
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

| Token              | TTL knob                                                                                                              | Notes                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------------------ | --------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Authorization code | `OpConfig::authorization_code_ttl_secs`, **60 s**                                                                     | Single use, enforced by a compare-and-swap on `oauth_authorization_codes.consumed_at`. A second exchange is refused whatever else about it is right, and a _failed_ exchange spends the code too — so a captured code cannot be probed against candidate verifiers                                                                                                                                       |
| Access token       | `OpConfig::access_token_ttl_secs`, from **`staff_auth.access_token_ttl_seconds`** (900 by default, bounded 10..=3600) | Bearer, presented on every `/dash/v1/*` call. Configurable per deployment since 2026-09-10, unlike `/v1`'s, which is a constant and says why — see "Replacing the token before it expires" below                                                                                                                                                                                                         |
| Refresh token      | **Not issued**                                                                                                        | vpay does not use `RefreshTokenStore` for this flow. Staff re-run authorization-code + PKCE when the access token expires — a short-TTL access token with no refresh token, rather than a long-lived refresh token that `authkestra-op` has no endpoint to revoke. **The re-run is the refresh**, and it is the dashboard app's own server that performs it, before the expiry rather than after — below |
| ID token           | **Not issued**                                                                                                        | The dashboard reads who is signed in from `GET /dash/v1/staff/session`, which answers from the session row rather than from a claim — so an id token would be a second, staler copy of the same fact. `openid` is not in the registration's single scope, so the grant does not mint one                                                                                                                 |

**No revocation endpoint exists in `authkestra-op`.** A stolen or misused
access token cannot be revoked mid-lifetime through the OP itself — see the
Consequences section of ADR-0009. Not issuing a refresh token narrows this
exposure rather than closing it: there is no long-lived refresh token to
also protect, but the access token itself is still a live bearer credential
for the whole of its TTL. This flow's mitigation is a short access-token TTL and a deny-list.
~~**which one vpay implements is not yet decided.**~~ **Decided 2026-09-07,
[ADR-0017](../../adr/0017-staff-authentication.md) decision 2, and it is both:**
the access token lives in the session row, the dashboard's own server reads it
back on every render, and signing out deletes the row. So the _obtainability_
of the token is revoked even though the JWT stays cryptographically valid for
the rest of its TTL — see "The session, and what signing out actually does".

### Replacing the token before it expires

_Issue #88 item 1, 2026-09-10. The gap the exp28 review's finding F4 left half
closed._

**The problem is an arithmetic one.** The access token lives 900 seconds by
default; a session lives thirty minutes idle and twelve hours absolute. Until
the exp28 review nothing re-minted at all — `requireStaff` ran the
authorization-code leg only when the session row carried **no** token — so a
quarter of an hour into every sign-in `/dash/v1` began answering

    The bearer token is invalid, expired, or was not issued for this endpoint.

on every render, until the person signed out and back in. That review added the
**reactive** re-mint in `frontends/apps/dashboard/src/server/dash-read.ts`:
one retry, on a `401` only. It works, and it costs a refused request in front
of every read once a token has died.

**There is no refresh token and there is not going to be one.** The refresh
_is_ the authorization-code leg, run again on the session the browser already
holds. That is not a way around a check but strictly more checking than
carrying one token for twelve hours: `vpay_api::staff::oauth::authorize`
re-reads the staff row and re-checks the active status, the merchant binding
and `password_change_required` on **every** mint. `authkestra-op` offers a
`RefreshTokenStore` and this deployment gives it
`vpay_api::op::refusing_stores`' fail-closed type; adding a refresh token would
add a second long-lived credential with no endpoint to revoke it, which is the
thing "Token lifetimes" above already refused.

So what changed is _when_ the leg runs, and three pieces make it possible:

1. **`staff_sessions.access_token_expires_at`** (migration `0040`), written in
   the same statement as the token and paired with it by
   `staff_sessions_token_expiry_is_paired`. Migration 0040 clears every token
   already stored rather than inventing an expiry for one — nobody is signed
   out by that, because a session with no token is the state every session is
   in between the second factor and its first render.
2. **`GET /dash/v1/staff/session` carries it**, as `access_token_expires_at`
   (RFC 3339) beside `access_token_ttl_seconds`. The app does not read the
   JWT's own `exp`: it holds the token and presents it, it does not verify it,
   and reading a claim out of an unverified credential is a habit worth not
   having here.
3. **`server/gate.ts` decides**, with `REMINT_AFTER_FRACTION = 0.8`: a token
   with less than a fifth of its TTL left is `stale-token` rather than `ready`,
   and `requireStaff` runs the leg it already ran for a session with no token
   at all. A **fraction** and not a fixed number of seconds, because the TTL is
   configuration now — sixty seconds of margin would be a fifteenth of one TTL
   and three times another.

`stale-token` is a separate arm from `needs-token` for one reason: there is
something to fall back on. If vpay cannot be _reached_ for the re-mint, the
token in hand has not expired — that is what the margin bought — so the page
renders with it instead of showing an outage box. A `401` is **not** fallen
back on, stale or not: it means `/authorize` refused this session on this
request, and reading on with a token that refusal has just invalidated is the
hole the exp24 review's finding F1 closed one layer down.

The reactive retry stays. A clock that disagrees with vpay's, a token revoked
mid-render, and a render that arrives late are all things a margin cannot see.

**The TTL is configuration** (`staff_auth.access_token_ttl_seconds`, bounded
10..=3600) and `/v1`'s is not, which is deliberate: `vpay_api::op::ACCESS_TOKEN_TTL_SECS`
argues that a TTL varying by YAML is one more thing that can differ between the
sandbox a merchant integrates against and the production they go live on — and
that argument is about a number _merchants_ build against. Nothing outside this
deployment ever receives a dashboard token. What it buys is the case nothing
could otherwise exercise: `demo_staff_token_ttl` sets thirty seconds on the
demo stack, and `dashboard.cy.ts` crosses both the margin and the expiry in one
leg, in a real browser, without a re-login.

Proof:
`the_dashboard_token_is_re_minted_from_a_live_session_and_from_nothing_else`
over a booted server — the TTL honoured on the wire and in the row, a live
session minting a _different_ token, and the leg refused for a disabled staff
member, a staff member moved to another merchant, an idle session and a
signed-out one, each with the same session restored afterwards as a control;
`a_session_token_without_its_expiry_is_refused_by_the_database`; nine
`gateFor` cases in `gate.test.ts`, five of them about the margin; and the
browser leg. Decisive mutations:
writing `crate::op::ACCESS_TOKEN_TTL_SECS` back into the OP config (`expires_in`
reads 900 against a configured 10), `staleTokenFor` answering `false` for an
absent or unparseable expiry, and dropping the margin — for which the browser
leg asserts that `access_token_expires_at` **moved while the old one had not
yet passed**, because a page renders identically either way once `dash-read.ts`
has retried.

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

| Piece                                                                                                                               | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ----------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `/dash/v1/oauth/authorize`                                                                                                          | `authkestra_op::handlers::authorize::handle_authorize`, called by `vpay_api::staff::oauth::authorize` — which supplies the `Identity` authkestra takes as a parameter and authenticates nobody for                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `/dash/v1/oauth/token`                                                                                                              | **vpay's own** (`vpay_api::staff::oauth::token`). Not `handle_token`'s dispatch: the mint has to stamp `vpay_config::DASHBOARD_MERCHANT_CLAIM`, and `default_handle_authorization_code`'s last step _is_ the mint. Every check the default performs is performed there, in the same order, each with its own test                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `/userinfo`, discovery, `/jwks.json` on `/dash/v1`                                                                                  | **Not served.** One OP, one issuer (`{public_base_url}/v1/oauth`) and one JWKS: a dashboard token's `iss` is the merchant surface's, and `/v1/oauth/jwks.json` is where its key is published. A second discovery document would be a second issuer identity for one signer                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Client registration (dashboard's own `client_id`, its bound `merchant_id`, redirect URIs, PKCE requirement, single read-only scope) | vpay configuration (ADR-0003 — YAML, not the dashboard). `merchant_id` since 2026-09-06: the one tenant `/dash/v1` reads, refused at boot if unregistered                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Authorization codes                                                                                                                 | `oauth_authorization_codes` (migration `0035`), a **vpay-owned** table modelled in `schemas/vpay.cstack` and reached through CrateStack, replacing `RefusingAuthorizationCodeStore` for this one grant. Not `authkestra.oauth_codes`: that table exists for `SqlxOpStore`, which is behind a feature pinning `sqlx ^0.8`, and this workspace is on `=0.9.0` so CrateStack and `vpay-db` can share a transaction. It also carries two columns authkestra's shape has nowhere to put — `session_id` (so signing out kills a code in flight) and `merchant_id` (so a staff row edited between issue and exchange cannot move a token to another tenant)                                                                                                                        |
| Staff identity and credentials                                                                                                      | `staff_members` (migration `0035`): argon2id with a deployment pepper, RFC 6238 TOTP sealed with AES-256-GCM under a second deployment key, and `last_totp_step` — the replay guard, a compare-and-swap. `vpay_api::staff_auth` owns the cryptography; `vpay-db` never learns what any of the strings mean                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Sessions                                                                                                                            | `staff_sessions` (migration `0035`), keyed by the SHA-256 of an opaque token. See "The session, and what signing out actually does"                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Whether a session is still alive _before_ the second factor                                                                         | `GET /dash/v1/staff/session/stage` (`vpay_api::staff::session_stage`), and it is the only route that answers for a `pending_totp` session. It publishes the stage and nothing about the person, because a caller at that stage has presented a password and no second factor. Added 2026-09-10 so that `/login/totp` could stop reading a wrong code as a sign-out                                                                                                                                                                                                                                                                                                                                                                                                          |
| Device codes                                                                                                                        | **Nothing.** The schema exists (`backends/migrations/0006_create-authkestra-op-tables.sql`) and its four tables are unread and unwritten by any code path. This row said `authkestra_op::sqlx_store::SqlxOpStore` against vpay's Postgres, and that was true of the type `/v1`'s OP put in three unreachable slots; those slots hold `vpay_api::op::refusing_stores`' fail-closed types now, and the `sqlx-postgres` feature that gated `SqlxOpStore` is off in every manifest. A `/dash/v1` that ever serves the authorization-code grant has to choose a store, and `SqlxOpStore` is no longer a free choice: it pins `sqlx ^0.8`, and this workspace has moved to 0.9. See "The three OP stores that pinned sqlx 0.8" in [status.md](../../status.md)                    |
| `oauth_refresh_tokens`, `oauth_device_codes`                                                                                        | Created by the same migration (`authkestra-op`'s fixed DDL is transcribed wholesale, not column-by-column selected) but structurally unused by this flow: refresh tokens are not issued (Token lifetimes, above) and the device grant is not offered on any client this deployment registers                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Signing keys and rotation                                                                                                           | vpay operational tooling. Storage schema exists (`backends/migrations/0007_create-oauth-signing-keys.sql`: `oauth_signing_keys`, at most one active key enforced by a partial unique index); key generation and rotation logic itself is not yet designed or written                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Session → per-record authorization (which staff member may view which merchant's records)                                           | vpay's own layer on top of the validated token; not Authkestra's concern. `vpay_api::require_dashboard_token` authorizes by **registration**: the tenant is `dashboard_client.merchant_id`, and no claim in any token changes it. Since ADR-0017 the token must also _carry_ that tenant as a `vpay_merchant_id` claim, which is a second lock on the same door — a forged claim buys a `403`, never another merchant's rows — and is what makes a `client_credentials` token structurally unable to read this surface. Which _staff member_ is which is `staff_members.id`, and it is the token's `sub`; it authorises nothing, and `/authorize` refuses a staff member whose `merchant_id` is not the binding one round trip earlier. Scoped to _view_ — see Scope, above |
| Audit log row per write                                                                                                             | [ADR-0008](../../adr/0008-dashboard-scope.md) — one row per dashboard action, independent of the auth mechanism. Not yet applicable: there is no write path to log (Scope, above)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
