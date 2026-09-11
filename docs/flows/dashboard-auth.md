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
_page_ completes the exchange; here the app's own server does, and a browser
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

| Bound    | Value                                  | Moved by                 |
| -------- | -------------------------------------- | ------------------------ |
| Absolute | 12 h from creation                     | nothing — never extended |
| Idle     | 30 min since the last accepted request | every accepted request   |

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

## The rest of this flow

Everything between the session above and the Status below was 464 lines until
2026-09-11. It is three pages now, in the order it was written, moved verbatim:

- [dashboard-auth/sessions-and-refusals.md](dashboard-auth/sessions-and-refusals.md)
  — changing a password ends every other session, a mistyped code does not, and
  why every refusal is one answer
- [dashboard-auth/rate-limiting.md](dashboard-auth/rate-limiting.md) — the
  budget is the deployment's, and which address the limiter counts
- [dashboard-auth/scope-and-tokens.md](dashboard-auth/scope-and-tokens.md) —
  scope, token lifetimes, replacing a token before it expires, JWKS publication
  and key rotation, and where each piece lives

## Status

**A staff member can sign in, and since 2026-09-07 they can do it in a
browser.** The first half of that sentence entered this document with
ADR-0017; the second half is exp28's, and until it was true everything below
was reachable over HTTP and by nothing a person could click.

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
- `POST /dash/v1/staff/login`, `/totp`, `/password`, `/logout`,
  `GET /dash/v1/staff/session` and `/staff/session/stage`, and
  `GET /dash/v1/oauth/authorize` + `POST /dash/v1/oauth/token`. **Eight**
  unauthenticated routes (seven until 2026-09-10 — the eighth is the stage
  read, "A mistyped code does not end a session either" above), mounted beside
  the protected reads rather than inside the bearer-token layer, because they
  exist to produce the credential that layer checks.
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

**Added 2026-09-10 (issue #79 items 1-3):**

- `rate_limit_windows` (migration `0038`), and with it a sign-in budget that
  is **the deployment's rather than each replica's** — see "The budget is the
  deployment's". `two_replicas_share_one_sign_in_budget` boots two vpay
  servers over one Postgres and reads `429` on the sixth attempt against a
  configured budget of five; with per-instance counters it reads `[401 × 6]`.
- `staff_auth.trusted_proxies`, so the per-address budget can count the
  **caller** behind a reverse proxy rather than the proxy — see "Which
  address the limiter counts". Two cases, because either alone would pass
  with the check deleted.
- `staff_auth.rate_limits`, one policy per action.
- `POST /staff/password` requires the **current** password and deletes every
  other session of that staff member — see "Changing a password ends every
  other session". The dashboard's form carries the field; its vitest case
  asserted the opposite until this pass, quoting the argument the endpoint
  itself carried.

**What is still not built, and none of it is implied by the above:**

1. ~~**The pages.**~~ **Built 2026-09-07 (exp28).** `frontends/apps/dashboard`
   serves `/login`, `/login/totp` (with the enrolment QR and the secret as
   text on a first sign-in), `/login/password`, `/payments` and
   `/payments/{id}`. It is the OAuth client decision 4 describes: the session
   cookie is httpOnly, Secure, SameSite=Lax on its own origin, the app's own
   server follows the `/authorize` `302` and exchanges the code, and the
   `/dash/v1` access token is read back out of the `staff_sessions` row on
   every render rather than kept in the app — which is what makes signing out
   a revocation in practice and not only in the schema.

   **There is no route at `redirect_uri`, and nothing is missing.** Decision 4
   has the app's own server follow the redirect, so that string is an
   identifier the two legs must spell identically rather than a page. It is a
   thing a reader will go looking for, so it is said here as well as in
   [dashboard.md](dashboard.md).

   `dashboard.cy.ts` drives the whole of the flow above through a browser
   against the real stack, and computes its TOTP codes from the secret **the
   enrolment screen displayed** — so what it proves is that an authenticator
   app enrolled from that QR would work, rather than that a test secret
   verifies against itself. `just demo-staff` is what creates the staff member
   it signs in as; `vpay-server staff add` is still the only way one is
   created.

2. **No sweep.** Nothing deletes an expired `staff_sessions` or
   `oauth_authorization_codes` row on a schedule. Expired rows are refused on
   read and removed by the sign-out cascade; the indexes a sweep would need
   exist, and the sweep does not.
3. **No `audit_log`.** ADR-0008 wants one row per dashboard _write_, and this
   surface mounts none — `require_dashboard_token` refuses every non-read
   method before the router matches.
4. **No way to disable the dashboard client.** `disabled_clients` revokes a
   _merchant_ credential; the dashboard registration can only be removed from
   YAML and the process restarted. What can be disabled per person is
   `staff_members.status`, which is the granularity that matters — and which
   now takes effect on the **next request** for both credentials a sign-in
   produces, not just for the session.

   _Corrected 2026-09-07 (exp24 review, findings F1 and F6)._ As first
   delivered, setting `status = 'disabled'` refused the session routes and
   left the already-minted `/dash/v1` bearer token reading payment intents
   for the rest of its 15-minute TTL — and reassigning a staff member to
   another merchant left them reading their old merchant's rows for the same
   window. `require_dashboard_token` never read `staff_members` at all:
   everything it checked was a statement about the _token_ and none of it was
   a statement about the _person_. It now reads the row its `sub` names and
   refuses a `disabled` one, a missing one, and one whose `merchant_id` is no
   longer the binding — one primary-key read, after every cheaper check,
   failing closed on a database error, and refusing nobody who was ever
   allowed in, since `/authorize` requires both before it will mint a code.
   `disabling_a_staff_member_refuses_their_live_session` and
   `moving_a_staff_member_to_another_merchant_refuses_their_existing_token`
   are the guards. A break-glass control that a payments dashboard honours a
   quarter of an hour late is not a break-glass control.

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
Solved by changing the _validator_ rather than the grant — ADR-0017 decision
3, above.

4. **Key rotation** — see item 5 of what is not built. Unchanged.

What is proven about the dashboard **validator** is now proven about the whole
path: `backends/tests/integration/tests/dashboard_read_surface.rs` (15 cases)
covers which rows a validly-minted token may read and which credentials are
refused, and `staff_sign_in.rs` (13 cases) covers how one is obtained. The
first file's own header still opens by saying it proves nothing about signing
in, and that remains true _of that file_.

`hmac`, `sha2`, `subtle` and `aes-gcm` were listed here as unused workspace
pins. `sha2` gained its first consumer on 2026-09-02; **`hmac`, `subtle` and
`aes-gcm` gained theirs on 2026-09-07** — the TOTP HMAC, the constant-time
code comparison, and the sealed TOTP secret respectively. None is unused now.

This flow is tracked as **Phase 2b** in [`docs/roadmap.md`](../roadmap.md).
See [../status.md](../status.md) for the full, row-by-row picture.
