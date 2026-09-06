# The dashboard

What the staff dashboard is, what slice 1 built, and — at greater length —
what it did not.

Authentication has its own document ([dashboard-auth.md](dashboard-auth.md));
this one is about the surface and the product. The decision that the
dashboard _observes and does not administer_ is
[ADR-0008](../adr/0008-dashboard-scope.md); the decision that vpay runs its
own OP for it is [ADR-0009](../adr/0009-dashboard-oidc-provider.md).

## The invariant

> **A `/dash/v1` request reads exactly one tenant's rows: the one
> `dashboard_client.merchant_id` names, fixed in YAML and checked at boot.**

Not a tenant resolved from the caller's credential, the way `/v1` resolves
one — the value comes from configuration, and no claim in any token can
change it. A deployment whose staff must see two merchants registers two
dashboard clients; the field does not become a list.

## Slices

The dashboard is built in slices, and the navigation is only ever allowed to
link to slices that exist. A menu entry for a page nobody wrote is the same
lie as an empty table.

| Slice | What it is                                 | State                                                                |
| ----- | ------------------------------------------ | -------------------------------------------------------------------- |
| 1     | Payments — list and detail                 | **This slice.** The API half is built; the pages are not — see below |
| 2     | Webhooks — deliveries, retries, signatures | Not started                                                          |
| 3     | Checkout sessions                          | Not started                                                          |
| 4     | Balances and the ledger                    | Not started                                                          |
| 5     | Settings                                   | Not started                                                          |
| 6     | Rail health                                | Not started                                                          |

## What slice 1 built (2026-09-06)

Two `GET` routes, behind an authentication and authorisation boundary, over
the same rows `/v1` serves:

| Route                               | Answers                                                                                                |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `GET /dash/v1/payment_intents`      | The bound merchant's intents, newest first, cursor-paged; `?status=`, `?created_gte=`, `?created_lte=` |
| `GET /dash/v1/payment_intents/{id}` | One intent, plus its charge, its refunds and its event timeline                                        |

Both are read-only, and that is structural rather than a promise: any method
but `GET`/`HEAD` is refused by the boundary _before_ the router matches — with
a `403`, the boundary's answer ("you may not write here at all"), rather than
the route table's `405` — so a write mounted here later cannot arrive unlogged
(ADR-0008 requires an `audit_log` row per dashboard write, and none exists).
Pinned by
`a_write_method_is_refused_by_the_boundary_not_by_the_route_table`, which
distinguishes the two answers; before it was written the claim was true and
untested.

Four checks stand between a request and a row, each in a different place so
that no single edit removes the boundary:

1. the token validates for `Surface::Dashboard` — signature, expiry, issuer
   and audience (`vpay:dash/v1`), against vpay's own published JWKS;
2. its `client_id` is the registered dashboard client's. The audience says
   which _surface_; only the `sub` says which _credential_. **This check does
   not survive a real login, and that is recorded rather than fixed** — see
   "The `client_id` check is written for the grant we have" below;
3. it carries the registration's single scope;
4. every query filters by the bound `merchant_id`.

Boot adds two refusals that make the first three worth having:

- a `dashboard_client.merchant_id` no `merchant_clients` entry registers is
  fatal (`ConfigError::DashboardUnknownMerchant`). Without it the failure has
  no runtime symptom at all: every query filters by a tenant no row carries,
  so the list is empty and the detail read is a 404 — exactly what a merchant
  with no payments looks like;
- a **merchant** registration listing `vpay:dash/v1` in `allowed_audiences`
  is fatal (`ConfigError::MerchantClaimsDashboardAudience`). This was a real
  hole, not a hypothetical one: `handle_client_credentials` mints a token for
  any requested audience `allowed_audiences` permits, so one YAML line would
  have let a merchant credential obtain a token the `/dash/v1` validator
  accepts.

A deployment that registers no `dashboard_client` mounts **no `/dash/v1`
nest at all**, so every path under it is the honest 404 rather than a 401
promising a credential would help.

## What slice 1 did NOT build

### Nobody can sign in

**This is the headline, and it is a blocker rather than a shortfall.**
`/dash/v1` requires a token whose audience is `vpay:dash/v1`. The only grant
vpay serves is `client_credentials`, registered for merchant clients alone —
and, since this slice, explicitly forbidden from claiming that audience. The
authorization-code + PKCE grant that would mint one is not built.

It is not built because building it requires a decision nobody has taken:
**how does a human staff member prove who they are?** `authkestra-op`'s
`handle_authorize` takes an already-authenticated
`authkestra_engine::auth::state::Identity` **as a parameter** — it
authenticates nobody. vpay would have to supply one, and vpay has no staff
table, no credential store, no password hashing, no session store compiled
in (`authkestra-engine` is pinned without `sql-postgres`), and no
`AuthenticationStrategy` implementation. Choosing among the options (a staff
table with password hashes; WebAuthn; TOTP; federating the human step to an
external IdP in front of vpay's own OP) is an ADR, and it interacts with
ADR-0009's "vpay is its own OP" in ways a passing implementation must not
settle.

### The `client_id` check is written for the grant we have

Found by the 2026-09-06 review and deliberately **not** changed, because
changing it means choosing something reserved for the maintainer.

Check 2 above compares the token's `sub` with the registered dashboard
client's `client_id`. Under `client_credentials` that is right: Authkestra's
`TokenManager::issue_client_token` sets `sub` to the client id. Under the
authorization-code grant this slice is blocked on,
`default_handle_authorization_code` issues a **user** token — `sub` is the
staff member's identity and the client id goes into `aud`. So the check as
written would refuse every token a working dashboard login produced.

Which claim identifies the dashboard *credential* once a human is in the loop
(`azp`, an explicit `client_id` claim, or the audience itself under the rule
that the dashboard audience becomes the `DashboardClient`'s `client_id`) is
part of the same decision as blocker 2 below, and belongs with it. Nothing
else here makes that work harder: moving `vpay:dash/v1` into one
`vpay_config::DASHBOARD_AUDIENCE` constant makes the audience half of it a
single edit.

A second, smaller decision is already recorded as a maintainer's call and is
also unresolved: `authkestra-op`'s `default_handle_authorization_code` mints
the access token with `aud = <client_id>` and has no requested-audience path,
so a token from that grant would not carry `vpay:dash/v1` at all. See
[dashboard-auth.md](dashboard-auth.md)'s Status, blocker 3, and
[roadmap.md](../roadmap.md)'s Phase 2b, scope item 3.

**Consequence, stated plainly: the two routes above are a resource server
with no issuer.** They are real, tested and fail closed, and no client of
this deployment can reach them. That is the honest half of the slice to
build first — the tenancy boundary has to be right _before_ a login exists,
not after — but it is a half.

### There are no pages

`frontends/apps/dashboard` is unchanged: still the scaffold, still saying so
on screen. Sign-in, the payments table, the detail view and sign-out were all
in slice 1's brief and none was built, because a dashboard whose session
cannot exist is a set of pages that cannot be reached, screenshotted, or
tested end to end. `dashboard.cy.ts` still asserts the scaffold notice.

### Two columns the list cannot show

- **The payer's phone is always blank.** `charges.payer_ref_masked` is never
  written — the confirm path stores `None`
  (`vpay_api::v1::payment_intents`'s `open_attempt`). The detail read
  renders the column rather than deriving a mask from the unmasked
  `payer_ref`, so the field appears the day the column is written and not
  before. `docs/status.md` carries the gap.
- **There is no search by phone**, for the same reason: a filter over a
  column that is always `NULL` answers "no results" for every payer who ever
  paid, which reads as an answer.

### No writes, no other slices

ADR-0008's per-record write operations (re-poll a charge, replay a webhook,
issue a refund, annotate a charge) are not built and are not scoped —
`dashboard-auth.md`'s "Scope" section explains why the registration still
has exactly one scope. Slices 2–6 are untouched.

## Not an SDK surface

[`docs/sdks/parity.md`](../sdks/parity.md) covers merchant SDK surfaces and
deliberately does not cover these routes. The merchant SDKs speak `/v1`; a
`/dash/v1` method in one would be a merchant credential reaching for a staff
surface. The dashboard's own client is the Next.js app's server side, which
speaks HTTP directly. The detail response is therefore vpay's own shape
(`object: "dashboard.payment_detail"`) and does not pretend to be a Stripe
object — Stripe has no "everything about this payment" resource, and
inventing one under the merchant API's vocabulary would make an SDK author
reasonably expect it there.

## Status

**Built and proven, 2026-09-06:** the two routes, the boundary, the two boot
refusals, and the `Surface::Dashboard` audience constant moved into
`vpay-config` beside the merchant one.
`backends/tests/integration/tests/dashboard_read_surface.rs` drives all of it
over a real booted server on a real Postgres — 13 tests, 0 ignored — and its
own header states what it cannot claim.

**Not built:** login, of any kind; the dashboard's server-side session; every
page; every other slice; every write. See "What slice 1 did NOT build" above
and [../status.md](../status.md) for the row-by-row picture.

**The one thing a reader must not conclude from this document:** that the
dashboard works. Two `GET` routes exist that nobody can authenticate to.
