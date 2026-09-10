# exp23 — dashboard slice 1, implementation notes

Branch `claude/exp23-dashboard-slice1`, base `3694e34`. Written 2026-09-06.

These are working notes: what the brief asked for, what was built, what was
not, and — the part that matters most — the two decisions this task refused
to take on the maintainer's behalf.

## The brief, and where it broke

The brief's slice 1 was: sign-in (authorization code + PKCE against vpay's
own OP), a `/dashboard/v1` read surface bound to one merchant, three pages
(sign-in, payments list, payment detail), vitest + Rust + Cypress tests, and
docs. It anticipated that _serving the grant_ might be larger than a slice
and said so: "deliver sign-in end to end first and report the rest as not
done rather than stubbing it."

Sign-in could not be delivered, and the reason is not that it was large.

## Blocker 1 — nobody has decided how a staff member authenticates

`authkestra_op::handlers::authorize::handle_authorize` has this signature
(`authkestra-op-0.7.1/src/handlers/authorize.rs`):

```rust
pub async fn handle_authorize(
    req: AuthorizeRequest,
    identity: Identity,        // <- already authenticated, by someone else
    config: &OpConfig,
    op_store: &dyn OpStore,
) -> AuthorizeOutcome
```

It validates the client, the redirect URI, the scope, the response type and
PKCE, and issues a code **for an identity it is handed**. It authenticates
nobody. vpay must produce that `Identity`, and vpay has:

- no staff/user table — 33 migrations, none of them;
- no credential store and no password hashing;
- no `authkestra_engine::auth::strategy::AuthenticationStrategy`
  implementation, and no `CredentialStore`;
- no session store compiled in — `authkestra-engine` is pinned
  `features = ["rustls-no-provider", "token", "session"]`, without
  `sql-postgres`;
- no `authkestra-axum` dependency (deliberately — its bundled router mounts
  endpoints this deployment must not serve).

Choosing among a staff table with password hashes, WebAuthn, TOTP, or
federating the _human_ step to an external IdP in front of vpay's own OP is
an ADR. It touches ADR-0009's central claim ("vpay runs Authkestra as its own
OP") and it is a security-critical credential model that would be shipped
unreviewed if a slice invented one in passing.

**Nothing in the repository records this as an open question.** ADR-0009's
login diagram says "(staff authenticates)" and never says against what;
`docs/roadmap.md`'s Phase 2b lists "`/login`, `/authorize` and `/userinfo`"
as scope without saying what `/login` checks. It is written down now, in
`docs/flows/dashboard-auth.md`'s blocker 1 and in `docs/status.md`.

## Blocker 2 — the audience problem, already reserved

`default_handle_authorization_code` mints the access token with
`aud = <client_id>` and has no requested-audience path, so a token from that
grant would not carry `vpay:dash/v1` at all. `docs/flows/dashboard-auth.md`
(blocker 3) and `docs/roadmap.md` (Phase 2b, item 3) both already say, in
so many words, that resolving it is "a maintainer decision, not a default to
pick in passing." This task did not pick one.

## What was built instead

The half that is decision-free and is the security-critical half: **the
tenancy boundary, before a login exists rather than after.**

- `DashboardClient::merchant_id` — required, the one tenant `/dash/v1` reads.
- Two boot refusals (`DashboardUnknownMerchant`,
  `MerchantClaimsDashboardAudience`).
- `vpay_config::DASHBOARD_AUDIENCE`, so `Surface::Dashboard.audience()` stops
  being a local literal.
- `vpay_api::require_dashboard_token` and the `/dash/v1` nest.
- `GET /dash/v1/payment_intents` and `GET /dash/v1/payment_intents/{id}`.
- `PaymentIntents::list_page_filtered`, `Refunds::list_for_intent`,
  `Events::list_for_objects` in `vpay-db`.
- Ten integration tests over a booted server on real Postgres.

Documented in `docs/flows/dashboard.md` (new) and
`docs/reference/vpay-api.md` § the dashboard surface.

## The hole this found

`validate_merchant_client` required `vpay:v1` to be _present_ in a merchant
registration's `allowed_audiences` and restricted nothing else it could
contain. `authkestra_op`'s `handle_client_credentials` mints a token for any
requested audience the registration permits. So a merchant whose YAML listed
`vpay:dash/v1` could have obtained a token that
`Surface::Dashboard`'s validator accepts.

It was harmless for exactly as long as `/dash/v1` mounted nothing, and it was
closed on the day that stopped being true. `require_dashboard_token`'s
`client_id` check refuses such a token a second time, but a boundary that
depends on one check is a boundary one edit removes.

## Mutations run

| Mutation                                                                  | Result                        |
| ------------------------------------------------------------------------- | ----------------------------- |
| Remove `validate_dashboard_binding`'s call                                | 1 config test fails           |
| Remove the merchant-claims-dashboard-audience check                       | 1 config test fails           |
| `MerchantScope` from `claims.client_id` instead of the binding            | 3 integration tests fail      |
| Delete the `claims.client_id != binding.client_id` arm                    | 1 integration test fails      |
| Build the dash nest's validator with `Surface::Merchant`                  | 8 integration tests fail      |
| Mount the nest unconditionally                                            | **nothing fails** — see below |
| Unconditional mount **and** the middleware's `None` guard weakened to 401 | 1 integration test fails      |

The sixth is recorded rather than smoothed over. The conditional mount in
`router` and `require_dashboard_token`'s own `None` guard produce the same
404 independently, so removing either alone is invisible. Both are kept: a
router assembled by a future binary is not obliged to consult the first, and
the second is what holds if it does not. This is noted in the test's own doc
comment and in `docs/reference/vpay-api.md`.

## Not done

- **Sign-in.** No `/login`, no `/authorize`, no `/userinfo`, no callback, no
  session. Blockers 1 and 2.
- **Every page.** `frontends/apps/dashboard` is byte-identical to the base
  commit: still the scaffold, still saying so on screen, still zero tests.
  Pages behind a session that cannot exist cannot be reached, screenshotted,
  or driven by Cypress, so building them would have produced UI nobody has
  seen work.
- **Screenshots.** There is nothing to screenshot. The directory this file
  is in contains no images, deliberately.
- **`dashboard.cy.ts`.** Unchanged. It still asserts the scaffold notice,
  which is still what the app renders.
- **vitest unit tests for the app.** Nothing to test — session handling,
  masking and paging cursors are all frontend code that was not written.
  The Rust side's equivalents are covered.
- **Phone masking and phone search.** `charges.payer_ref_masked` is never
  written; the detail read renders the column (null on every row) rather
  than deriving a mask from the unmasked `payer_ref`, and there is no phone
  filter because one over an always-`NULL` column answers "no results" for
  every payer who ever paid.
- **Id search.** A prefix search needs an index `payment_intents` does not
  have. An exact-id lookup is the detail route.
- **Writes, and slices 2–6.**

## Naming: `/dash/v1`, not `/dashboard/v1`

The brief said `/dashboard/v1`. Everything already in the repository says
`/dash/v1`: ADR-0008 ("a separate `/dash/v1` API"), ADR-0009 (its title),
`docs/flows/dashboard-auth.md` throughout, the audience literal
`vpay:dash/v1`, and every `dashboard_client.redirect_uris` in
`config/application.yml` and `application-sandbox.yml`. Two of those are
accepted, immutable ADRs. `/dash/v1` was used; the discrepancy is flagged
rather than resolved unilaterally, and renaming later is one constant
(`DASH_NEST`) plus the audience string plus two ADR supersessions.
