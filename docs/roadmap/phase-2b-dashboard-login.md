# Roadmap — Phase 2b: Dashboard login (`/dash/v1`)

_Moved out of [docs/roadmap.md](../roadmap.md) on 2026-09-11 by exp57, which split a 1 504-line page into one page per phase. **The text below is the original, unedited** — every struck-through claim, every "Open —" maintainer question and every dated amendment is here as it was written, because on this page those are most of the content; only the relative links gained a `../` because the file moved one directory down._

## Phase 2b — Dashboard login (`/dash/v1`)

**Open — build the dashboard on CrateStack's refine integration?** (noted
2026-09-02.) `@cratestack/refine` ships a tested refine.dev `DataProvider`
over a CrateStack-generated REST/RPC client, and `cratestack
generate-typescript --refine` emits the resource manifest from a `.cstack`
schema — so most of an operator admin panel would be generated rather than
hand-written in the Next.js scaffold. The price is that `schemas/vpay.cstack`
would have to become an authoritative _service_ model for the staff surface
(it is a design sketch today, excluded from the build graph and already
diverged from the migrations on two `CHECK` constraints), served by a
CrateStack service beside the hand-written Stripe-shaped `/v1`, which stays
as it is. That is an ADR-level decision (it touches ADR-0008 and the
migrations' status as the schema of record) and is not made here.

**Split out of Phase 2 on 2026-09-02**, when the merchant half of that phase
landed and the dashboard half did not. This is a bookkeeping change, not a
re-plan: Phase 2's own **Unblocks** paragraph already said `/dash/v1` login
is a parallel deliverable and not a prerequisite for Phases 3–6. Giving it
its own heading stops "Phase 2 is done" from ever being read as "login
works". Nothing below is new scope; it is Phase 2's dashboard scope, moved.

**Goal.** A staff member completes an authorization-code + PKCE login
against `/dash/v1` and a subsequent authenticated call accepts the token; a
merchant-audience token is rejected on `/dash/v1`.

**Status.** ~~Not started. **No login has ever been performed.**~~ **Corrected
2026-09-07: the backend is built and the pages are not — see the amendment at
the end of this phase.** ~~and no
`/dash/v1` route exists~~ — corrected 2026-09-06 (exp23): the `/dash/v1`
_read_ surface exists (two `GET` routes, tenant-bound, ten integration tests
over a real server; see [flows/dashboard.md](../flows/dashboard.md)), and no
client of this deployment can obtain a token for it. Scope items 1, 2 and 3
below are all untouched. What Phase 2 left behind for it: the schema
(migrations `0006`/`0013`, proven compatible with the real
`SqlxOpStore<Postgres>`), the dashboard client modelled and validated in
config (`vpay_config::oauth::DashboardClient`), signing keys and a JWKS
endpoint, and `JwtValidator`/`AuthenticatedDashboard` pinned to
`Surface::Dashboard` and unit-proven to reject a merchant-audience token.
None of that is login.

**Scope.**

1. A `SessionStore`. `authkestra-engine` is pinned
   `features = ["rustls-no-provider", "token", "session"]` — **without
   `sql-postgres`** — so no SQL-backed session store is compiled into the
   workspace today. Enabling that feature is a supply-chain change
   (`sqlx/chrono`, `sqlx/json`) and needs `cargo deny` re-run.
2. `/login`, `/authorize` and `/userinfo`, plus the callback the dashboard
   needs. Phase 2 mounted none of them, deliberately: no merchant client can
   use the authorization-code grant, and `authkestra-axum` is not a
   dependency (its bundled router would mount them and would publish a
   one-key JWKS instead of the rotation window vpay serves).

   **This item has a prerequisite it never named, added 2026-09-06 (exp23):
   decide how a staff member authenticates.**
   `authkestra_op::handlers::authorize::handle_authorize` takes an
   already-authenticated `authkestra_engine::auth::state::Identity` **as a
   parameter** — it authenticates nobody — and vpay has no staff table, no
   credential store, no password hashing and no `AuthenticationStrategy`
   implementation to produce one. Choosing among a staff table with password
   hashes, WebAuthn, TOTP, or federating the _human_ step to an external IdP
   in front of vpay's own OP is an ADR that touches ADR-0009's central claim.
   It is a maintainer's call with the same standing as item 3, and it is the
   larger of the two.

3. **Resolve the audience problem first.** `authkestra-op`'s
   `default_handle_authorization_code` mints the access token with
   `Some(client_id)` as the audience and has **no requested-audience path at
   all** (`authkestra-op-0.7.1/src/handlers/token.rs`, step 7). A token from
   that grant would carry `aud = <client_id>`, and
   `Surface::Dashboard.audience()` (`vpay:dash/v1`) rejects every one of
   them. `handle_client_credentials` _does_ honour a requested audience,
   which is why `/v1` does not hit this. Options — a custom grant handler, a
   different `Surface::Dashboard` audience rule, or an upstream change —
   are a maintainer's call, not a default to pick in passing. **Taken
   2026-09-07: the second option.** See the amendment at the end of this
   phase.
4. The dashboard's own server-side session handling ([ADR-0008](../adr/0008-dashboard-scope.md):
   the dashboard never holds a merchant API key and calls `/dash/v1`
   server-side under an OIDC session).

**Definition of done.**

- An integration test drives a real authorization-code + PKCE round trip
  against `/dash/v1` and receives a token a subsequent authenticated call
  accepts.
- A merchant-audience token is rejected on `/dash/v1`, over a real mounted
  router — the missing half of the pair Phase 2 could only prove in one
  direction.
- A signing key is rotated at least once and a token minted under the old
  key still verifies for the whole of its lifetime. **Runtime rotation does
  not exist** (Phase 2, scope item 3): rotation is restart-based today, so
  this bullet needs either a rotation mechanism or an explicit decision that
  restart-based rotation is the answer, written down.

**Decisions and open questions.** Every one Phase 2 lists still applies here
— the dashboard scope ([ADR-0008](../adr/0008-dashboard-scope.md)), no refresh
tokens, and the Keycloak/ZITADEL comparison ADR-0009 asked for and nobody has
done. ~~and the revocation-endpoint gap~~ — **closed on 2026-09-07** by
[ADR-0017](../adr/0017-staff-authentication.md) decision 2, in the only form
available: the access token lives in the session row, the dashboard's own
server reads it back on every render, and signing out deletes the row. The
minted JWT stays cryptographically valid for the rest of its TTL and nothing
can change that; what is revoked is its _obtainability_.

### Amendment, 2026-09-07 — the login is built; the pages are not

Scope items 1–3 above are **done** and item 4 is done in a shape the paragraph
did not anticipate. [ADR-0017](../adr/0017-staff-authentication.md) took the
decision item 2 reserved (a `staff_members` table, argon2id with a deployment
pepper, mandatory RFC 6238 TOTP) and resolved item 3 by changing the
_validator_ rather than the grant: the audience is the registered
`dashboard_client.client_id`, because that is what
`default_handle_authorization_code` mints, and `vpay:dash/v1` is retired.

Item 4's session is vpay's own row in vpay's own table rather than
`authkestra-engine`'s `SessionStore`, which stays uncompiled — and the
dashboard app follows the `/authorize` redirect **itself**, server-side, so a
browser never sees a code, a verifier or a token.

**Definition of done, against the four bullets above:**

- ✅ _An integration test drives a real authorization-code + PKCE round trip
  and receives a token a subsequent authenticated call accepts_ —
  `backends/tests/integration/tests/staff_sign_in.rs`, 13 cases, and it mints
  no token of its own.
- ✅ _A merchant-audience token is rejected on `/dash/v1`, over a real mounted
  router_ — `a_merchant_audience_token_is_refused_on_dash_v1`, and since
  ADR-0017 a **`client_credentials`** token is refused too, which is stricter
  than this bullet asked for.
- ⛔ _A signing key is rotated at least once_ — **untouched.** Rotation is
  still restart-based and nothing re-reads the key file. This is the one
  bullet ADR-0017 does not move, and it is why this phase is not closed.

~~**Still open, and not implied by any of the above:** the pages.~~
**Built 2026-09-07 (exp28) — see the second amendment below.** Still open: no
sweep of expired sessions or authorization codes, no `audit_log`, and no way
to disable the dashboard _client_ short of removing it from YAML.

### Second amendment, 2026-09-07 — the pages exist

`frontends/apps/dashboard` is no longer the scaffold. `/login`, `/login/totp`
(with the mandatory enrolment ADR-0017 decision 1 requires — the `otpauth://`
QR and the base32 secret, and one valid code before anything is written to
`staff_members`), `/login/password` (the forced replacement of the printed
one-time password, which is the only page a session carrying
`password_change_required` can reach), `/payments` and `/payments/{id}`, plus
a sign-out that deletes the session row.

**The "OIDC callback" this phase's scope item 2 asked for is not a page, and
that is ADR-0017 decision 4 rather than an omission.** The Next.js app is the
OAuth client and its own server requests the code, follows the `302` and
exchanges it, so `dashboard_client.redirect_uris` names a string the two legs
must spell identically and not a route. A browser never sees a code, a
verifier or a token, and the `/dash/v1` access token is read back out of the
`staff_sessions` row on every render — which is what makes signing out a
revocation in practice.

`dashboard.cy.ts` drives the whole of it through a browser against the real
compose stack, computing its TOTP codes from the secret the enrolment screen
displayed; `just demo-staff` creates the staff member it signs in as.
136 vitest cases in `frontends/apps/dashboard`, 0 skipped.
[flows/dashboard.md](../flows/dashboard.md) has the surface, and
[plans/exp28-dashboard-pages-notes/opus.md](../plans/exp28-dashboard-pages-notes/opus.md)
the reasoning and the screenshots.

**This phase still does not close**, and the reason is unchanged: a signing
key has never been rotated.

The **refine.dev** question at the top of this phase is untouched by any of
this and is still an ADR nobody has written. What ADR-0017 changes about it is
one input: `schemas/vpay.cstack` is no longer "a design sketch, excluded from
the build graph" — it is compiled into `vpay-db` on every build, and three of
its models are now the only path to their tables.

---
