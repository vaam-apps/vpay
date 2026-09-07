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

1. the token validates for the dashboard surface — signature, expiry, issuer
   and audience, against vpay's own published JWKS. **The audience is the
   registered `dashboard_client.client_id`** since
   [ADR-0017](../adr/0017-staff-authentication.md); `vpay:dash/v1` is retired,
   because `default_handle_authorization_code` mints `aud = <client_id>` and
   has no requested-audience path, so no token from the only grant that can
   produce a dashboard credential would ever have carried the constant;
2. it carries a `vpay_merchant_id` claim equal to the bound `merchant_id`.
   Nothing but the staff authorization-code grant stamps that claim, so **no
   `client_credentials` token can satisfy it** — the refusal is a property of
   the mint rather than of a list somebody maintains. The claim is *compared*,
   never used: the tenant still comes from the binding, so a forged claim buys
   a `403` and never another merchant's rows;
3. it carries the registration's single scope;
4. every query filters by the bound `merchant_id`.

Check 2 replaced a comparison of the token's `sub` with the registered client
id. That was right for `client_credentials` — where `sub` *is* the client id —
and wrong for every token a real login produces, where `sub` is the **staff
member**. It was the 2026-09-06 review's finding F7, left as a maintainer
decision; ADR-0017 takes it.

Boot adds two refusals that make the first three worth having:

- a `dashboard_client.merchant_id` no `merchant_clients` entry registers is
  fatal (`ConfigError::DashboardUnknownMerchant`). Without it the failure has
  no runtime symptom at all: every query filters by a tenant no row carries,
  so the list is empty and the detail read is a 404 — exactly what a merchant
  with no payments looks like;
- a **merchant** registration listing the dashboard client's own id in
  `allowed_audiences` is fatal (`ConfigError::MerchantClaimsDashboardAudience`).
  This was a real hole, not a hypothetical one:
  `handle_client_credentials` mints a token for any requested audience
  `allowed_audiences` permits, so one YAML line would have let a merchant
  credential obtain a token the `/dash/v1` validator accepts. The forbidden
  value was the constant `vpay:dash/v1` and the check ran per registration;
  ADR-0017 made it the dashboard client's own id, which one registration
  cannot know, so the check moved to whole-document scope beside
  `validate_dashboard_binding`.

A deployment that registers no `dashboard_client` mounts **no `/dash/v1`
nest at all**, so every path under it is the honest 404 rather than a 401
promising a credential would help.

## What slice 1 did NOT build

### ~~Nobody can sign in~~ — corrected 2026-09-07

**This was the headline of this section and it is no longer true.**
[ADR-0017](../adr/0017-staff-authentication.md) took the decision this
paragraph said nobody had taken — how a human staff member proves who they
are — and `vpay_api::staff` serves the grant.
[dashboard-auth.md](dashboard-auth.md) is the document that owns it; the short
form is: a vpay-owned `staff_members` table, argon2id with a deployment
pepper, mandatory RFC 6238 TOTP with a compare-and-swap replay guard,
server-side sessions with an absolute and an idle bound, and the
authorization-code grant with PKCE served for the dashboard client only.

`backends/tests/integration/tests/staff_sign_in.rs` (13 cases) drives it end
to end and **mints no token of its own**.

~~It is not built because building it requires a decision nobody has taken~~ —
and the paragraph that followed, about `authkestra-op` authenticating nobody,
is still accurate about `authkestra-op` and no longer a blocker: supplying the
`Identity` is exactly what `vpay_api::staff::oauth::authorize` does.

### ~~The `client_id` check is written for the grant we have~~ — resolved

Found by the 2026-09-06 review, recorded as finding F7 and deliberately not
changed then, because changing it meant choosing something reserved for the
maintainer. ADR-0017 decision 3 chose: **the credential is identified by
`aud`** — which the validator has already checked by the time
`require_dashboard_token` runs — **plus the merchant claim**, and `sub` names
the staff row and authorises nothing.

The second, smaller decision recorded beside it — that
`default_handle_authorization_code` mints `aud = <client_id>` with no
requested-audience path — is resolved in the same move, by changing the
*validator* to expect what the grant produces rather than forking the handler.

~~**Consequence, stated plainly: the two routes above are a resource server
with no issuer.**~~ They have an issuer now. What was true and stays true is
why the tenancy boundary was built first: it has to be right *before* a login
exists, not after.

### There are still no pages, and the reason changed

`frontends/apps/dashboard` is unchanged: still the scaffold, still saying so
on screen, still zero tests, and `dashboard.cy.ts` still asserts the scaffold
notice.

Slice 1's reason — "a dashboard whose session cannot exist is a set of pages
that cannot be reached, screenshotted, or tested end to end" — **stopped
being true on 2026-09-07**. A session can exist now, and the pages were in
ADR-0017's own scope. They were not built in that pass either, and the honest
statement of why is scope rather than a blocker: the backend slice
(three tables, the credential primitives, seven routes, the audience change
and two test suites) was as much as one pass delivered, and the brief it was
written to said in so many words to deliver the grant end to end first and
report the pages as not done rather than stubbing them.

So the whole of `/dash/v1` — the reads, the login and the grant — is reachable
over HTTP and by nothing a person can click.

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
