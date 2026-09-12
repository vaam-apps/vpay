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

| Slice | What it is                                 | State                                                           |
| ----- | ------------------------------------------ | --------------------------------------------------------------- |
| 1     | Payments — list and detail                 | **Built.** The two `/dash/v1` reads, the sign-in, and the pages |
| 2     | Webhooks — deliveries, retries, signatures | Not started                                                     |
| 3     | Checkout sessions                          | Not started                                                     |
| 4     | Balances and the ledger                    | Not started                                                     |
| 5     | Settings                                   | Not started                                                     |
| 6     | Rail health                                | Not started                                                     |

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
   the mint rather than of a list somebody maintains. The claim is _compared_,
   never used: the tenant still comes from the binding, so a forged claim buys
   a `403` and never another merchant's rows;
3. it carries the registration's single scope;
4. every query filters by the bound `merchant_id`.

Check 2 replaced a comparison of the token's `sub` with the registered client
id. That was right for `client_credentials` — where `sub` _is_ the client id —
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

That section was 322 lines of gaps and dated corrections — three of its four
entries are struck through and re-answered — and it is
[dashboard/slice-1-gaps.md](dashboard/slice-1-gaps.md) now, verbatim. **Read it
before you read the Status below**: it is where "nobody can sign in", "the
`client_id` check is written for the grant we have" and "there are still no
pages" were each retired, with the date and the evidence, and it ends with the
two columns the list cannot show and the fact that there are no writes.

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

**The short answer, and the long one is on three pages.** The two `/dash/v1`
reads, the boundary and the two boot refusals are built and proven
(2026-09-06); a staff member can sign in (2026-09-07,
[ADR-0017](../adr/0017-staff-authentication.md)); the pages exist and a real
browser signs in through the real OP and reads this merchant's payments
(2026-09-07). **Not built: every other slice (2–6); every write, and therefore
no `audit_log`; no sweep of expired sessions or authorization codes; no key
rotation.** ~~The BFF added on 2026-09-11 has no consumer, and Lane 3 of the
Refine plan was declined rather than started.~~ **Corrected 2026-09-12:**
lanes 3 and 4 are built — the payments list and detail render through Refine
hooks, and `src/dash/resources.ts` replaced `nav.tsx`'s second list.
~~the payments list and detail read through Refine hooks against that BFF,
which is now its consumer~~ **— corrected again the same day, after
`just test-e2e` was run for the first time on this work and failed eight legs
of `dashboard.cy.ts`.** The **reads did not move to the browser**: both pages
are Server Components that call `requireStaff()` and then read `/dash/v1`
themselves, exactly as before Refine, and hand the answer to the hooks as
`initialData`. The BFF is wired to the data provider and still has no read a
page issues. Evidence, and the two defects the browser-side read produced, on
[dashboard/status-read-seam-and-bff.md](dashboard/status-read-seam-and-bff.md).

**The one thing a reader must not conclude from this document is that the
dashboard is finished.** A staff member can sign in and read this merchant's
payments. What the dashboard still cannot do is anything at all to them.

The measurements, the dates and the corrections behind each of those sentences:

- [dashboard/status-built-and-not-built.md](dashboard/status-built-and-not-built.md)
  — what is built and proven, by which suite, and the "Not built" list
- [dashboard/status-styling-and-demo.md](dashboard/status-styling-and-demo.md)
  — the exp26 restyle and its review, the demo stack's port and tenant, and the
  second payments list nothing serves
- [dashboard/status-read-seam-and-bff.md](dashboard/status-read-seam-and-bff.md)
  — the read seam, the browser-reachable surface, the maintainer's question it
  raises, the security review, and what the browser run still does not cover
