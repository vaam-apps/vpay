# The dashboard — Status: what is built and proven, and what is not built

_Split out of [docs/flows/dashboard.md](../dashboard.md) on 2026-09-11 by exp57, which broke a 865-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

**Built and proven, 2026-09-06:** the two routes, the boundary, the two boot
refusals, and the `Surface::Dashboard` audience constant moved into
`vpay-config` beside the merchant one.
`backends/tests/integration/tests/dashboard_read_surface.rs` drives all of it
over a real booted server on a real Postgres — 13 tests, 0 ignored — and its
own header states what it cannot claim.

**Built and proven, 2026-09-07 (ADR-0017):** the staff sign-in — a
`staff_members` table, argon2id with a deployment pepper, mandatory TOTP with
a replay guard, server-side sessions, and the authorization-code grant with
PKCE. [dashboard-auth.md](../dashboard-auth.md) owns it.

**Built and proven, 2026-09-07 (exp28): the pages.** `/login`, `/login/totp`,
`/login/password`, `/payments` and `/payments/{id}`, on `@vpay/ui`, with the
Next.js app as the OAuth client running the code leg server-side. 136 vitest
cases in `frontends/apps/dashboard`, 0 skipped, covering the cookie
attributes, PKCE against RFC 7636 Appendix B's own vector, the session gate,
the masked-payer dash, the paging cursors and axe-core's structural rules over
every screen. `dashboard.cy.ts` signs in through the real OP against the real
stack — password, mandatory enrolment, a code the spec computes from the
secret the enrolment screen displayed, the forced password change, then the
code exchange — lists this merchant's payments, opens one, signs out and is
refused afterwards.

**Amended 2026-09-11 (exp53): three of this app's components were primitives,
and are `@vpay/ui`'s now.** `EmptyState`, `Pager` and `DetailTimeline` had
nothing dashboard-specific in them — the checkout could not have used one
without copying it — so they are `EmptyState`, `Pagination` and `Timeline` in
the shared package. What is left in the app is `PaymentsPager`: the two
`next/link` elements `Pagination` renders, because the router is the app's.

Three more things this app wrote by hand are primitives too: nineteen bare
`<code>` tags are `Code`, five unstyled `next/link` anchors are `Link`, and
this app's five bare `<section>` elements are `Section` — which matters beyond
tidiness, because a `<section>` with no accessible name is not a landmark at
all, and a heading merely sitting inside one does not give it a name. All
five were `<div>`s with extra steps. (Six more, all in the checkout's
`screens.tsx`, were found by the same audit and are NOT converted — see
[the exp53 notes](../../plans/exp53-ui-package-notes/opus.md) §7.)
`payment-detail.tsx` fell from 264
lines to 207 with nothing removed from the screen, `DataList`/`DataListRow`
having taken over the twenty rows of `<th scope="row">` markup it repeated
twice.

The suite is **178** cases in 19 files, down four from 182, and every one is
accounted for: `empty-state.test.tsx` (1) and `detail-timeline.test.tsx` (2)
are deleted because both components moved, and have 3 and 2 cases
respectively in `@vpay/ui`; the a11y suite dropped its two cases for the same
pair and gained one for the pager composition, which it did not have before.

Four decisive mutations, each measured: drop `httpOnly` from the session
cookie and `cookies.test.ts` fails; exchange a freshly generated PKCE verifier
instead of the one the challenge was derived from and `oauth.test.ts` fails;
add a nav link to a page nobody wrote and `layout.test.tsx` fails twice; and
in the browser, `dashboard.cy.ts` asserts the session cookie is `httpOnly`,
that `document.cookie` cannot see it, and that no JWT appears anywhere in the
rendered page.

**Not built:** every other slice (2–6); every write, and therefore no
`audit_log`; no sweep of expired sessions or authorization codes; no key
rotation. See "What slice 1 did NOT build" above and
[../status.md](../../status.md) for the row-by-row picture.

**Built and proven, 2026-09-11 (exp56): the BFF's method policy and its first
browser evidence.** `frontends/apps/dashboard/middleware.ts` closes the
`OPTIONS` answer that Next gave before this app's own checks ran, and five
cases in `dashboard.cy.ts` drive `/api/dash/**` from a real browser. Both are
described in full further down this document; what belongs here is the count:
the dashboard suite is **243 vitest cases in 22 files, 0 skipped**, and
`just test-e2e` is **27 Cypress tests, 27 passing**, of which `dashboard.cy.ts`
holds 16.

**Deliberately NOT built in exp56: Lane 3 of
[the Refine plan](../../plans/exp55-refine-seam-bff-notes/refine-plan.md), so the
BFF still has no consumer.** It was in that change's brief and was declined
rather than started, for reasons that are the plan's own and are worth naming
so the next attempt does not rediscover them:

- **The plan's file map says `src/dash/provider.ts` is "Lane 1 → Lane 3
  dataProvider", and it cannot be.** That module takes a `DashSession`
  carrying the `/dash/v1` bearer and calls `readDash`; a browser holds
  neither. Lane 3 needs a **second**, client-side provider over
  `/api/dash/**`, which is a module and a test suite the plan does not
  account for.
- **`notFound()` is a server-only API, and the detail page's use of it is a
  security property.** `app/payments/[id]/page.tsx` maps vpay's `404` to
  `notFound()` precisely because `/dash/v1` answers the identical body for
  another merchant's id and for one that never existed
  (`dashboard_read_surface.rs:531`). Lane 3's own decisive check 3 requires
  that mapping to survive — but a `useOne` in a client component cannot call
  `notFound()` on a resolved fetch, and the plan does not say what replaces
  it. Choosing is a decision about a cross-tenant id oracle on a payment
  system, not a plumbing detail.
- Around those two: two new dependencies on independent majors
  (`@refinedev/core` 5, `@refinedev/nextjs-router` 7), a route-group move of
  both pages, R4 (filters stay in the URL and work with JS off), R11 (no
  `useTable` page count against a surface that returns no total), R10's
  before/after bundle measurement, and rework of `layout.test.tsx`,
  `a11y.test.tsx` and both component suites once the table is client-fetched.

Nothing was half-done: no Refine package is installed, no page moved, and the
BFF is exactly as unused as the Lane 2 row says. **The `OPTIONS` fix and the
browser evidence stand on their own** — they are about a surface that exists
either way.
