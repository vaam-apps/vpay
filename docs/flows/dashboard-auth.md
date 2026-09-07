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

Two legs, and the first of them is vpay's own. `authkestra-op` authenticates
nobody — `handle_authorize` takes an already-authenticated `Identity` **as a
parameter** — so the thing that produces one is
[ADR-0017](../adr/0017-staff-authentication.md)'s staff table and the four
routes in front of it.

```text
staff browser              dashboard app              vpay
      |  POST /api/auth/login  |                       |
      |----------------------->|  POST /dash/v1/staff/login
      |                        |---------------------->|  argon2id + pepper
      |                        |  { session, next }    |  session row: pending_totp
      |                        |<----------------------|
      |  (QR on first sign-in) |                       |
      |  POST /api/auth/totp   |                       |
      |----------------------->|  POST /dash/v1/staff/totp
      |                        |---------------------->|  RFC 6238, +/-1 step
      |                        |                       |  replay guard: CAS on
      |                        |                       |  last_totp_step
      |                        |<----------------------|  session row: authenticated
      |                        |                       |
      |                        |  GET /dash/v1/oauth/authorize?code_challenge=...
      |                        |---------------------->|  exact redirect_uri match
      |                        |  302 -> ?code=...     |  code row, 60 s, single use
      |                        |<----------------------|
      |                        |  POST /dash/v1/oauth/token { code, code_verifier }
      |                        |---------------------->|  PKCE S256, CAS consume
      |                        |  { access_token }     |  aud = dashboard client_id
      |                        |<----------------------|  + vpay_merchant_id claim
      |                        |                       |
      |  a page                |  GET /dash/v1/payment_intents, Bearer ...
      |<---------------------->|---------------------->|
```

**The dashboard app follows the `302` itself**, and that is
[ADR-0017](../adr/0017-staff-authentication.md) decision 4 rather than an
accident. In a browser-driven flow the user agent follows it and a callback
*page* completes the exchange; here the app's own server does, and a browser
never sees a code, a verifier or a token. Everything the grant checks is
unchanged — the redirect URI is matched byte for byte against the
registration, PKCE is mandatory, the code is single-use — and what changes is
only who follows the redirect.

`redirect_uris` are matched exactly — no prefix or wildcard matching
(`authkestra_op::client::ClientRegistration::allows_redirect_uri`). PKCE is
mandatory for **every** client on this grant, unconditionally
(authkestra#273, OAuth 2.1 §4.1), not because a registration asks for it.

The diagram intentionally does not show a `refresh_token` — see Token
lifetimes below.

## The session, and what signing out actually does

The session token is 256 bits from the OS CSPRNG, held by the dashboard app in
an httpOnly, Secure, SameSite=Lax cookie on its own origin and presented to
vpay in `X-Vpay-Staff-Session`. `staff_sessions` is keyed by its **SHA-256**,
so a dump of that table yields no usable session.

Two bounds, and they are not one bound:

| Bound | Value | Moved by |
|---|---|---|
| Absolute | 12 h from creation | nothing — never extended |
| Idle | 30 min since the last accepted request | every accepted request |

Both are checked on **read** and neither deletes anything: an expired row is
refused and left in place, so "this session expired" and "this session never
existed" stay distinguishable to an operator reading the table. On the wire
they are the same `401`, for the reason in the next section.

`POST /dash/v1/staff/logout` **deletes** the row. That is a revocation rather
than a cookie clear: the row carries the access token the exchange minted, so
the dashboard's own server can no longer obtain it — and the cascade on
`oauth_authorization_codes.session_id` kills any code that session issued and
has not exchanged. **This is the server-side deny-list ADR-0009's
Consequences section explicitly left open**, and ADR-0017 is what decides it.

What it is not: the minted JWT stays cryptographically valid for the rest of
its TTL. Nothing can change that, and
`signing_out_deletes_the_session_and_with_it_the_access_token` asserts it
rather than implying otherwise.

## Every refusal is one answer

No such address, wrong password, disabled account, wrong code, replayed code,
expired session, idle session, forged session, session at the wrong stage:
one `401`, one sentence, `error.code = "authentication_error"`. The step that
refused reaches the log and never the body.

The **timing** half of the same property is
`vpay_api::staff_auth::StaffCredentials::verify_absent_account`: an address
with no account still costs one argon2id verification, so "no such account"
and "wrong password" take the same time as well as the same shape. A login
form that answered differently for either would be an account-enumeration
oracle.

Sign-in is rate limited per email **and** per IP, in-process, fixed window,
ten attempts per five minutes, refused **before** any credential work happens.
Both counters move on every attempt including a refused one — short-circuiting
would let an attacker who exhausted one address keep hammering a thousand
others from the same host with that host's counter frozen.

**The limits are per replica.** Three replicas admit three times the attempts
one does. That is the honest cost of in-process limiting; the alternative — a
shared counter in Postgres — puts a write on the unauthenticated path, which
is a denial-of-service amplifier of a different kind. It is the first thing to
revisit for a deployment that runs many replicas.

**The IP the limiter counts is the transport peer, and nothing else.** vpay
reads no `X-Forwarded-For` and no `Forwarded`, deliberately: both are
caller-supplied on an unauthenticated route, and honouring either without an
authenticated trusted-proxy list hands an attacker a fresh bucket per request.
The consequence is stated rather than hidden — **behind a reverse proxy or an
Ingress, every staff member shares one per-IP budget**, because the peer is
the proxy. The per-email budget still binds per account, and closing the rest
needs a trusted-proxy allow-list that this slice does not have. It is recorded
in ADR-0017's Consequences.

*Corrected 2026-09-07 (exp24 review, finding F2).* Until that review the
per-IP half **did not exist**: axum supplies the peer address through
`ConnectInfo`, `ConnectInfo` is present only when the service is built with
`into_make_service_with_connect_info`, and neither `vpay-server` nor the test
harness did that. Every attempt in the process was counted under the
limiter's one `ip:unknown` key, so ten requests from anywhere locked every
staff member out of the dashboard for five minutes — the deployment-wide
version of exactly the lockout the design refuses to build. Both call sites
now build the service with connect info, and
`the_sign_in_rate_limit_is_per_source_address` burns one loopback source's
budget and asserts a second source's first attempt is still served.

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
| Authorization code | `OpConfig::authorization_code_ttl_secs`, **60 s** | Single use, enforced by a compare-and-swap on `oauth_authorization_codes.consumed_at`. A second exchange is refused whatever else about it is right, and a *failed* exchange spends the code too — so a captured code cannot be probed against candidate verifiers |
| Access token | `OpConfig::access_token_ttl_secs` | Bearer, presented on every `/dash/v1/*` call |
| Refresh token | **Not issued** | vpay does not use `RefreshTokenStore` for this flow. Staff re-run authorization-code + PKCE when the access token expires — a short-TTL access token with no refresh token, rather than a long-lived refresh token that `authkestra-op` has no endpoint to revoke |
| ID token | **Not issued** | The dashboard reads who is signed in from `GET /dash/v1/staff/session`, which answers from the session row rather than from a claim — so an id token would be a second, staler copy of the same fact. `openid` is not in the registration's single scope, so the grant does not mint one |

**No revocation endpoint exists in `authkestra-op`.** A stolen or misused
access token cannot be revoked mid-lifetime through the OP itself — see the
Consequences section of ADR-0009. Not issuing a refresh token narrows this
exposure rather than closing it: there is no long-lived refresh token to
also protect, but the access token itself is still a live bearer credential
for the whole of its TTL. This flow's mitigation is a short access-token TTL and a deny-list.
~~**which one vpay implements is not yet decided.**~~ **Decided 2026-09-07,
[ADR-0017](../adr/0017-staff-authentication.md) decision 2, and it is both:**
the access token lives in the session row, the dashboard's own server reads it
back on every render, and signing out deletes the row. So the *obtainability*
of the token is revoked even though the JWT stays cryptographically valid for
the rest of its TTL — see "The session, and what signing out actually does".

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
| `/dash/v1/oauth/authorize` | `authkestra_op::handlers::authorize::handle_authorize`, called by `vpay_api::staff::oauth::authorize` — which supplies the `Identity` authkestra takes as a parameter and authenticates nobody for |
| `/dash/v1/oauth/token` | **vpay's own** (`vpay_api::staff::oauth::token`). Not `handle_token`'s dispatch: the mint has to stamp `vpay_config::DASHBOARD_MERCHANT_CLAIM`, and `default_handle_authorization_code`'s last step *is* the mint. Every check the default performs is performed there, in the same order, each with its own test |
| `/userinfo`, discovery, `/jwks.json` on `/dash/v1` | **Not served.** One OP, one issuer (`{public_base_url}/v1/oauth`) and one JWKS: a dashboard token's `iss` is the merchant surface's, and `/v1/oauth/jwks.json` is where its key is published. A second discovery document would be a second issuer identity for one signer |
| Client registration (dashboard's own `client_id`, its bound `merchant_id`, redirect URIs, PKCE requirement, single read-only scope) | vpay configuration (ADR-0003 — YAML, not the dashboard). `merchant_id` since 2026-09-06: the one tenant `/dash/v1` reads, refused at boot if unregistered |
| Authorization codes | `oauth_authorization_codes` (migration `0035`), a **vpay-owned** table modelled in `schemas/vpay.cstack` and reached through CrateStack, replacing `RefusingAuthorizationCodeStore` for this one grant. Not `authkestra.oauth_codes`: that table exists for `SqlxOpStore`, which is behind a feature pinning `sqlx ^0.8`, and this workspace is on `=0.9.0` so CrateStack and `vpay-db` can share a transaction. It also carries two columns authkestra's shape has nowhere to put — `session_id` (so signing out kills a code in flight) and `merchant_id` (so a staff row edited between issue and exchange cannot move a token to another tenant) |
| Staff identity and credentials | `staff_members` (migration `0035`): argon2id with a deployment pepper, RFC 6238 TOTP sealed with AES-256-GCM under a second deployment key, and `last_totp_step` — the replay guard, a compare-and-swap. `vpay_api::staff_auth` owns the cryptography; `vpay-db` never learns what any of the strings mean |
| Sessions | `staff_sessions` (migration `0035`), keyed by the SHA-256 of an opaque token. See "The session, and what signing out actually does" |
| Device codes | **Nothing.** The schema exists (`backends/migrations/0006_create-authkestra-op-tables.sql`) and its four tables are unread and unwritten by any code path. This row said `authkestra_op::sqlx_store::SqlxOpStore` against vpay's Postgres, and that was true of the type `/v1`'s OP put in three unreachable slots; those slots hold `vpay_api::op::refusing_stores`' fail-closed types now, and the `sqlx-postgres` feature that gated `SqlxOpStore` is off in every manifest. A `/dash/v1` that ever serves the authorization-code grant has to choose a store, and `SqlxOpStore` is no longer a free choice: it pins `sqlx ^0.8`, and this workspace has moved to 0.9. See "The three OP stores that pinned sqlx 0.8" in [status.md](../status.md) |
| `oauth_refresh_tokens`, `oauth_device_codes` | Created by the same migration (`authkestra-op`'s fixed DDL is transcribed wholesale, not column-by-column selected) but structurally unused by this flow: refresh tokens are not issued (Token lifetimes, above) and the device grant is not offered on any client this deployment registers |
| Signing keys and rotation | vpay operational tooling. Storage schema exists (`backends/migrations/0007_create-oauth-signing-keys.sql`: `oauth_signing_keys`, at most one active key enforced by a partial unique index); key generation and rotation logic itself is not yet designed or written |
| Session → per-record authorization (which staff member may view which merchant's records) | vpay's own layer on top of the validated token; not Authkestra's concern. `vpay_api::require_dashboard_token` authorizes by **registration**: the tenant is `dashboard_client.merchant_id`, and no claim in any token changes it. Since ADR-0017 the token must also *carry* that tenant as a `vpay_merchant_id` claim, which is a second lock on the same door — a forged claim buys a `403`, never another merchant's rows — and is what makes a `client_credentials` token structurally unable to read this surface. Which *staff member* is which is `staff_members.id`, and it is the token's `sub`; it authorises nothing, and `/authorize` refuses a staff member whose `merchant_id` is not the binding one round trip earlier. Scoped to *view* — see Scope, above |
| Audit log row per write | [ADR-0008](../adr/0008-dashboard-scope.md) — one row per dashboard action, independent of the auth mechanism. Not yet applicable: there is no write path to log (Scope, above) |

## Status

**A staff member can sign in.** That sentence has never been in this document
before, and it is the one a reader should carry away.

~~**No login has ever been performed.**~~ Corrected 2026-09-07
([ADR-0017](../adr/0017-staff-authentication.md)). Thirteen cases in
`backends/tests/integration/tests/staff_sign_in.rs` drive the real
`vpay_api::router` on a real socket over a real Postgres, and **that suite
mints no token at all**: every token it presents came out of
`POST /dash/v1/oauth/token` after a password, a TOTP code and a PKCE exchange,
and the server verified it through its own published JWKS over the same
socket.

What is built, in the order a request meets it:

- `staff_members`, `staff_sessions` and `oauth_authorization_codes`
  (migration `0035`), all three born with a `schemas/vpay.cstack` model and
  shaped so that **every** repository method runs through CrateStack — no
  `jsonb`, no `bytea`, no native enum, no `DEFAULT` on any column a writer
  names. `docs/reference/vpay-db.md` § CrateStack has the account.
- `vpay-server staff add --merchant … --email … --name …`, the **only** way a
  staff member is created. No HTTP endpoint creates one and there is no
  self-service sign-up. It prints a one-time password on stdout alone —
  a subcommand's logs go to stderr, which
  `staff_add_creates_a_staff_member_and_prints_a_one_time_password_on_stdout`
  found the hard way.
- `POST /dash/v1/staff/login`, `/totp`, `/password`, `/session`, `/logout`,
  and `GET /dash/v1/oauth/authorize` + `POST /dash/v1/oauth/token`. Seven
  unauthenticated routes, mounted beside the protected reads rather than
  inside the bearer-token layer, because they exist to produce the credential
  that layer checks.
- argon2id at OWASP's first parameter set with a deployment pepper; RFC 6238
  TOTP (HMAC-SHA1, 30 s, ±1 step) whose codes are pinned against Appendix B's
  own vectors; AES-256-GCM for the stored secret; the PKCE check pinned
  against RFC 7636 Appendix B's own vector.
- `Surface::Dashboard.audience()` is **gone**. The audience is the registered
  `dashboard_client.client_id`, because that is what
  `default_handle_authorization_code` mints; `vpay:dash/v1` is retired and
  ADR-0017 supersedes that part of ADR-0009.
- `require_dashboard_token` no longer compares the token's `sub` to the
  client id. That check was exp23's finding F7 — right for
  `client_credentials` and wrong for every token a real login produces — and
  the maintainer decision it reserved is taken: the credential is `aud` plus
  the merchant claim, and `sub` names the staff row — and while `sub`
  authorises nothing, the staff row it names must still be `active`, or the
  request is refused (finding F1).
- **No machine client may read `/dash/v1`.** A `client_credentials` token
  carries no merchant claim and nothing but this grant stamps one, so the
  refusal is a property of the mint. It is a tightening over what stood
  before, and `a_client_credentials_token_is_refused_on_dash_v1` pins it.

**What is still not built, and none of it is implied by the above:**

1. **The pages.** `frontends/apps/dashboard` is unchanged: still the scaffold,
   still saying so on screen, still zero tests, and `dashboard.cy.ts` still
   asserts the scaffold notice. Everything above is reachable over HTTP and by
   nothing a person can click. See [dashboard.md](dashboard.md).
2. **No sweep.** Nothing deletes an expired `staff_sessions` or
   `oauth_authorization_codes` row on a schedule. Expired rows are refused on
   read and removed by the sign-out cascade; the indexes a sweep would need
   exist, and the sweep does not.
3. **No `audit_log`.** ADR-0008 wants one row per dashboard *write*, and this
   surface mounts none — `require_dashboard_token` refuses every non-read
   method before the router matches.
4. **No way to disable the dashboard client.** `disabled_clients` revokes a
   *merchant* credential; the dashboard registration can only be removed from
   YAML and the process restarted. What can be disabled per person is
   `staff_members.status`, which is the granularity that matters — and which
   now takes effect on the **next request** for both credentials a sign-in
   produces, not just for the session.

   *Corrected 2026-09-07 (exp24 review, finding F1).* As first delivered,
   setting `status = 'disabled'` refused the session routes and left the
   already-minted `/dash/v1` bearer token reading payment intents for the
   rest of its 15-minute TTL. `require_dashboard_token` now reads the
   `staff_members` row its `sub` names — one primary-key read, after every
   cheaper check, failing closed on a database error — and
   `disabling_a_staff_member_refuses_their_live_session` asserts both halves.
   A break-glass control that a payments dashboard honours a quarter of an
   hour late is not a break-glass control.
5. **Key rotation has still never happened.** ADR-0009's fourth blocker is
   untouched. `TokenManager` holds one key for the life of the process,
   rotation is restart-based, and nothing re-reads the key file.
6. **The rate limit is per replica** — see "Every refusal is one answer".

**What the two blockers this document recorded turned out to be:**

~~1. **No login route.** … And writing them needs a decision that has never
been taken: how does a staff member prove who they are?~~ Taken, as ADR-0017
decision 1: a vpay-owned `staff_members` table, argon2id with a pepper, and
mandatory TOTP.

~~2. **No session store.** `authkestra-engine` is pinned without
`sql-postgres`.~~ Still pinned without it, and it is **not** a blocker: vpay's
sessions are its own rows in its own table through its own data layer, not
`authkestra-engine`'s `SessionStore`. Enabling that feature would pull
`sqlx/chrono` and `sqlx/json` into the graph for a store this deployment does
not use.

~~3. **An audience problem that must be solved before any of the above.**~~
Solved by changing the *validator* rather than the grant — ADR-0017 decision
3, above.

4. **Key rotation** — see item 5 of what is not built. Unchanged.

What is proven about the dashboard **validator** is now proven about the whole
path: `backends/tests/integration/tests/dashboard_read_surface.rs` (15 cases)
covers which rows a validly-minted token may read and which credentials are
refused, and `staff_sign_in.rs` (13 cases) covers how one is obtained. The
first file's own header still opens by saying it proves nothing about signing
in, and that remains true *of that file*.

`hmac`, `sha2`, `subtle` and `aes-gcm` were listed here as unused workspace
pins. `sha2` gained its first consumer on 2026-09-02; **`hmac`, `subtle` and
`aes-gcm` gained theirs on 2026-09-07** — the TOTP HMAC, the constant-time
code comparison, and the sealed TOTP secret respectively. None is unused now.

This flow is tracked as **Phase 2b** in [`docs/roadmap.md`](../roadmap.md).
See [../status.md](../status.md) for the full, row-by-row picture.
