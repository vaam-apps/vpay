# ADR-0017: How a staff member signs in to `/dash/v1`

- **Status:** Accepted
- **Date:** 2026-09-07
- **Deciders:** vpay maintainers (delegate, 2026-09-06)
- **Supersedes:** the audience half of
  [ADR-0009](0009-dashboard-oidc-provider.md) — the literal `vpay:dash/v1`
  is retired, see decision 3. Everything else in ADR-0009, including "vpay
  runs Authkestra as its own OP", stands.

## Context

[ADR-0008](0008-dashboard-scope.md) decided the dashboard authenticates with
OIDC sessions and never an API key. [ADR-0009](0009-dashboard-oidc-provider.md)
decided vpay runs the OP itself. Its login diagram says "(staff
authenticates)" and never says against what, and nothing in this repository
had ever recorded that as an open question.

On 2026-09-06 a slice went looking for the answer and found none
([docs/plans/exp23-dashboard-notes/opus.md](../plans/exp23-dashboard-notes/opus.md),
"Blocker 1"). It built the tenancy boundary instead — a real, tested,
fail-closed `/dash/v1` read surface — and reported plainly that **no grant
this deployment serves can mint a token for it**. A resource server with no
issuer.

Three things had to be decided before that could change, and none of them is
a default a slice may pick in passing:

1. **How does a human prove who they are?** `authkestra_op::handlers::authorize::handle_authorize`
   takes an already-authenticated `Identity` **as a parameter**; it
   authenticates nobody. vpay had no staff table, no credential store, no
   password hashing and no `AuthenticationStrategy`.
2. **Where does the session live?** `authkestra-engine` is pinned without
   `sql-postgres`, so no SQL-backed session store is compiled into the
   workspace at all.
3. **The audience problem.** `default_handle_authorization_code` mints with
   `aud = <client_id>` and has no requested-audience path, so a token from
   that grant would never carry `vpay:dash/v1` — the value
   `Surface::Dashboard`'s validator demanded. ADR-0009's own flow doc and
   `docs/roadmap.md` both reserved this as "a maintainer decision, not a
   default to pick in passing".

## Decision

### 1. Staff authenticate against a vpay-owned `staff_members` table

Two factors, both mandatory:

- **argon2id password**, OWASP's first listed parameter set (19 MiB, t=2,
  p=1), with a deployment **pepper** as argon2's secret input
  (`staff_auth.password_pepper`). The pepper is not in the database, so a
  stolen `staff_members` table is not by itself an offline cracking target.
- **RFC 6238 TOTP**, HMAC-SHA1, 30-second step, ±1 step. **Enrolment is
  mandatory at first sign-in**: a session never reaches `authenticated`
  while `staff_members.totp_secret` is `NULL`. The secret is sealed with AES-256-GCM
  under a second deployment key (`staff_auth.totp_encryption_key`) —
  encrypted rather than hashed because verification recomputes an HMAC over
  the secret itself, and there is no one-way form of it that still works.

**Replay of the last accepted code is refused.** `staff_members.last_totp_step`
records the time step of the last accepted code and
`Staff::record_totp_step` is a compare-and-swap that admits only a strictly
greater one. The ±1 window and this guard are one design: the window without
the guard admits replay for up to 90 seconds, and the guard without the
window refuses a phone whose clock is four seconds fast.

A staff member **belongs to exactly one merchant**. A staff surface that
could name any tenant would be an authorisation decision taken per request
against a list nothing validates; a deployment whose staff must see several
tenants registers several dashboard clients, exactly as
`DashboardClient::merchant_id` already says.

**No self-service sign-up, and no HTTP endpoint creates a staff member.**
`vpay-server staff add --merchant … --email … --name …` inserts the row and
prints a one-time password that must be changed at first sign-in
(`staff_members.password_change_required`). Every authenticated route refuses a
session whose staff row still carries that flag, so a printed password
cannot become a long-lived credential by being ignored.

The table is born on CrateStack: `model Staff` in `schemas/vpay.cstack`, in
the shape CrateStack projects — no DB default on any column a writer names,
`TEXT` + a hand-named CHECK for the status, and no `bytea`. Every one of the
six repository methods runs through the generated data layer, which no
previous vpay table has managed, and migration `0035` was shaped so it
could.

### 2. Sessions are server-side, with two bounds and a real revocation

An opaque session token, 256 bits from the OS CSPRNG, held by the dashboard
app in an **httpOnly, Secure, SameSite=Lax** cookie. The row lives in
Postgres through CrateStack (`staff_sessions`) and is keyed by the token's
SHA-256 — never the token, so a dump of the table yields no usable session.

- **Absolute** expiry 12 hours from creation, never extended.
- **Idle** expiry 30 minutes since the last accepted request.
- **Sign-out deletes the row**, which cascades onto any authorization code
  that session issued.

The row also carries the `/dash/v1` access token the code exchange minted for
it. That is what makes deleting the row a **revocation** rather than a
sign-out, and it is the deny-list ADR-0009's Consequences section explicitly
left open ("which of these vpay will actually build is not decided by this
ADR"). This ADR decides it.

**Sign-in is rate limited per email and per IP**, in-process, fixed window,
failing closed with `429`. In-process rather than durable, and a fixed window
rather than a lockout, because a durable lockout is a denial of service an
attacker triggers by guessing at somebody else's address. The limits are per
replica; the honest reading of that is in Consequences.

### 3. The OP serves the authorization-code grant with PKCE, for the dashboard client only

vpay supplies the authenticated `Identity` from the session established by
the login endpoints, and `handle_authorize` does the rest — client lookup,
exact redirect-URI match, scope check, `response_type`, PKCE. The code store
is a **vpay-owned table** (`oauth_authorization_codes`), born on CrateStack,
replacing `RefusingAuthorizationCodeStore` **for this grant only**. The
refresh and device stores stay refusing, and `/v1`'s three slots are
untouched.

Not `authkestra.oauth_codes`: that table exists for `SqlxOpStore`, which is
behind a feature pinning `sqlx ^0.8`, and this workspace moved to `=0.9.0`
so CrateStack and `vpay-db` could share a transaction.

**The audience is the dashboard client's own `client_id`, and `vpay:dash/v1`
is retired.** `default_handle_authorization_code` mints with
`aud = <client_id>`; rather than fork the handler to honour a requested
audience, the *validator* is changed to expect what the grant actually
produces. `Surface::Dashboard.audience()` is gone; `JwtValidator::new` takes
the audience as a parameter, and `vpay-server` passes
`dashboard_client.client_id`.

Three consequences follow, and each is a change this ADR owns:

- **`require_dashboard_token` is redesigned.** It compared the token's `sub`
  to the dashboard client id. Under `client_credentials` that was right
  (`sub` *is* the client id); under this grant `sub` is the **staff member**
  and the client id is the audience — so the check as written refused every
  token a real login would issue. It was
  [exp23's finding F7](../plans/exp23-dashboard-notes/opus-review.md), and
  the maintainer decision it reserved is taken here: **the credential is
  identified by `aud`** (the client id, config-bound to a merchant) **plus a
  `merchant_id` claim that must equal the binding**; `sub` names the staff
  row and authorises nothing.
- **No machine client may read the dashboard.** A `client_credentials` token
  cannot satisfy the merchant claim, because nothing but this grant stamps
  one. That is a tightening, and it is deliberate: `/dash/v1` is a staff
  surface and a machine caller on it would be a service account nobody
  registered as a person.
- **`MerchantClaimsDashboardAudience` moves to whole-document scope.** It was
  checked per merchant registration against a constant; the value is now the
  dashboard client's id, which is only knowable once the whole document is
  read. It moves beside `validate_dashboard_binding`
  ([exp23's F-list](../plans/exp23-dashboard-notes/opus-review.md), the
  second carried item).

`a_dashboard_audience_token_is_refused_on_v1` stays true, and a merchant
token on `/dash/v1` stays refused.

### 4. The dashboard app is the OAuth client, and it runs the code leg server-side

`frontends/apps/dashboard` holds the session cookie and calls vpay
server-side. It generates the PKCE verifier, requests the code, follows the
`302` itself and exchanges the code — a browser never sees a code, a
verifier or a token.

This is a **deviation from the shape a reader would assume** and is recorded
rather than glossed: in a browser-driven flow the user agent follows the
redirect and a callback *page* completes the exchange. Here the callback is a
route handler in the same app, and the redirect is followed by the app's own
server. Everything the grant checks is unchanged — the redirect URI is still
matched byte for byte against the registration, PKCE is still mandatory, the
code is still single-use — and what changes is only who follows the `302`.

The alternative was for vpay to set the session cookie on its **own** origin
so a top-level navigation to `/authorize` would carry it, which needs a
credentialed CORS allow-list for the login POST and puts a second cookie
domain in play. It buys nothing this design does not have, because the
dashboard is a first-party app vpay's own configuration registers.

## Consequences

**Losing `staff_auth.password_pepper` invalidates every stored password
hash.** It belongs in the same Secret as the RS256 signing key and has the
same backup story. Losing `staff_auth.totp_encryption_key` invalidates every
enrolled second factor; both are stated in migration `0035`'s own comments,
where an operator will look.

**Both secrets are refused at boot in livemode when absent.** In a sandbox
deployment they may be absent, and the consequence is stated rather than
hidden: `/dash/v1` mounts no login at all, exactly as a deployment with no
`dashboard_client` mounts no dashboard.

**A session row holds a live bearer token for the length of its TTL.** A
database dump therefore yields usable `/dash/v1` tokens until they expire.
This is a real residual and the reason it is accepted is that the
alternative — hashing it — does not work: the dashboard's own server has to
present the token, so there is no one-way form of it. The mitigations are the
access-token TTL and the fact that deleting the row revokes it.

**The rate limit is per replica.** Three replicas admit three times the
attempts a single one does. That is the honest cost of in-process limiting,
and the alternative — a shared counter in Postgres — puts a write on the
unauthenticated path, which is a denial-of-service amplifier of a different
kind. It is recorded in `docs/flows/dashboard-auth.md` and is the first thing
to revisit if a deployment runs many replicas.

**The per-IP budget counts the transport peer, so behind a proxy every staff
member shares one.** vpay reads no `X-Forwarded-For` and no `Forwarded`
header: both are caller-supplied on an unauthenticated route, and honouring
either without an authenticated trusted-proxy list is a fresh bucket per
request for an attacker. Under an Ingress the peer is the Ingress, and the
per-IP half then bounds the deployment rather than the caller. The per-email
half is unaffected and still bounds guessing at one account. Closing it needs
a trusted-proxy allow-list, which this slice does not have.

*Corrected 2026-09-07 (exp24 review, finding F2).* As first delivered the
per-IP half did not exist at all: the peer address reaches a handler only
through axum's `ConnectInfo`, and neither `vpay-server` nor the test harness
built its service with `into_make_service_with_connect_info`. Every attempt
was counted under the limiter's one `ip:unknown` key, so ten unauthenticated
requests locked every staff member out of the dashboard for five minutes.
Both call sites are fixed and
`the_sign_in_rate_limit_is_per_source_address` is the end-to-end guard; the
module's own unit tests could not catch it, because they call `check`
directly with an address the router never supplied.

**Nothing sweeps `staff_sessions` or `oauth_authorization_codes`.** Expired
rows are refused on read and removed by the sign-out cascade; there is no
periodic delete. The indexes a sweep would need exist; the sweep does not,
and `docs/status.md` says so.

**No `audit_log`.** ADR-0008 wants one row per dashboard write and this slice
mounts no dashboard write at all — `require_dashboard_token` refuses every
non-read method before the router matches. A table with no writer is a claim
nothing checks.

**Key rotation has still never happened.** ADR-0009's fourth blocker is
untouched by this ADR.
