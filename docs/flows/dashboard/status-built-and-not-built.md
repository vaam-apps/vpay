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

**Built and proven, 2026-09-13 (nav plan Lane A): the "More" drawer.**
`AppShell` gained a drawer (`src/components/more-menu.tsx`,
`@vaam-apps/ui`'s `MoreDetailDrawer`) holding the nav tree rendered from the
same `NAV_ENTRIES` array `SideNav` reads, `ThemeSwitcher`, and a second
`SignedInBar`. It is a **bottom sheet below `md` and a right-hand panel
above** — the component ships that split itself, so the call site writes no
placement class at all. (The first implementation shipped a right-hand panel
at every width, on the reasoning that `verify-ui`'s 60-character `className`
cap left no way to a bottom sheet; the review tested that claim and it did
not hold — the route was to call the component that already had the split.
`just verify-ui` is silent/exit 0 on the corrected file.) It closes a real gap: `SideNav` only ever renders its
`accountSlot` in the ≥1280px in-flow sidebar, so below that width there was no
theme control reachable at all before this. The signed-in identity did
**not** move into the drawer — `dashboard.cy.ts`'s
`cy.contains(staffEmail()).should("be.visible")`, with eight tests chained
after it in a `testIsolation: false` sequence, is why: `SignedInBar` stays
exactly where it was, unconditionally visible in `<main>`, and the drawer's
copy is a second mount of the same component rather than a competing one.
The rail is unchanged (still the one `Payments` entry, still
`smallScreen="floating"`), and the content column's padding
(`pb-20 sm:pb-0 sm:pl-20 xl:pl-0`) is untouched — the drawer floats over the
page like everything else `SideNav`'s rails do, and reserves no space of its
own.

`pnpm --filter @vpay/dashboard typecheck`, `just lint-web` (`verify-ui`
included) and `just test-web` all pass — the dashboard suite is **311 vitest
cases in 30 files, 0 skipped** (303 in 29 as first delivered; the review
added seven `more-menu.test.tsx` cases and one open-drawer `a11y.test.tsx`
case, the component having shipped with none) — with no test edited to pass
and no assertion weakened. Every added case was made to fail first by
breaking the thing it checks; the mutations and their output are on the
verification page.

**Not proven in a real browser, and this is the gap to read this row
against:** no Cypress case opens the drawer, and no screenshot was taken at
any width. jsdom applies no CSS, so the bottom-sheet claim is evidenced by
the class attribute on the rendered panel and by `MoreDetailDrawer`'s
source — not by anything that looked at a phone. A **pre-existing** React
duplicate-key warning (`SideNav`'s sub-640px rail keys `topItem` and the one
nav entry both `/payments`) is reported and left: it was reproduced on the
base commit with this drawer deleted, so it is not this change's, and fixing
it is nav IA (Lane E) or upstream.

Full detail is in
[../../status/verification/2026-09-13-dash-shell-drawer.md](../../status/verification/2026-09-13-dash-shell-drawer.md).

**Built and proven, 2026-09-13: a Storybook, mirroring the checkout's.** This
app had no visual-review surface and no browser-level accessibility check —
`docs/status/frontend.md` said so in two places, both struck through and
corrected in this same commit. `frontends/apps/dashboard/.storybook/` now
exists: 25 stories in `src/components/dashboard-screens.stories.tsx`, bound
to the same fixtures (`src/testing/fixtures.ts`) `a11y.test.tsx` renders, so a
screen cannot gain a story without a test already covering it. `just
build-storybook` exit 0; the built stylesheet defines `--color-base-100`
(referenced 22×, defined 3×); `just test-storybook` exit 0, **25 passed, 0
skipped, 0 unhandled errors**, both `@vaam-apps/ui` themes covered (21
stories `dark`, 4 `light` — `.storybook/preview.ts` names which stories carry
`light` and why only those are automated). _(Corrected on adversarial review:
three stories — `Shell`, `ShellLight`, `MoreMenuOpen` — rendered in whatever
theme the RUNNER's `prefers-color-scheme` said, because `@vaam-apps/ui`'s
`ThemeSwitcher` overwrites `data-theme` on mount from its own store. The
decorator now writes the package's storage key too; re-measured, all 25
stories render the theme they declare under both colour schemes.)_

**One real, third-party finding, not papered over.** At this suite's actual
browser viewport (1200×900 — below the `xl` breakpoint), `AppShell`'s `Shell`
story fails axe's `landmark-unique` rule: `@vaam-apps/ui@0.1.2`'s `SideNav`
renders its in-flow sidebar `<nav aria-label="Primary">` with only
`xl:`-prefixed utilities and no unprefixed `hidden`, so below `xl` it falls
back to a `<nav>`'s default `display: block` at the same time the
1024–1279px floating rail is also visible — two landmarks answering the same
name at once. Reproduced directly with `axe-core` at both viewports (zero
violations at 1280×800 and 1440×900, one at 1200×900 and at 1279×900), not
inferred from the error message, and traced to the exact upstream line
(`@vaam-apps/ui@0.1.2` `dist/components/primitives/side-nav.js:456`).
`Shell`/`Shell Light` disable **that one axe rule by id** and keep
`test: "error"` in force for everything else. _(Corrected on adversarial
review the same day: they first carried `parameters.a11y = { test: "todo" }`,
which switches the addon off for the whole story — measured, a
`#3a3a3a`-on-`#0a0b0d` probe placed inside `Shell` PASSED under it, so the
chrome around every screen in the app had no colour-contrast verdict at all.
With the narrowed suppression the same probe fails at 1.73 against `#0a0b0d`.
`src/a11y-gate.test.ts` now pins which stories may carry a suppression and
which rule id, verified under three mutations.)_ This is a defect in the
published package, not in this app's markup, and is not this task's to fix;
it has not been reported upstream.

**Two router hooks needed a stub, and `next/link` needed a `vite` `define`.**
`PaymentsFilters` (`useRouter`) and `AppShell` (`usePathname`) throw outside a
mounted Next app router; `.storybook/main.ts` aliases `next/navigation` to a
hand-written stub fixing `usePathname()` at `/payments` rather than
simulating routing. Separately, `PaymentsTable`/`PaymentsPager`'s `next/link`
is the first `next/link` usage in either app's Storybook: `next/dist/client/
has-base-path.js` throws `ReferenceError: process is not defined` in a real
browser without a `define` for the two `process.env.__NEXT_*` flags it reads
— caught by `test-storybook`, not by `build-storybook`, which bundles the
reference without executing it.

**Measured rather than carried over: no hand-written
`@vaam-apps/ui/styles/theme.css` alias.** The checkout's own `main.ts` needs
one (PR #135's finding). Removing the equivalent entry from a copy of this
app's config and rebuilding from a cold cache left `--color-base-100` defined
exactly as often (3×) as with it present — this app's build does not
reproduce that failure today. `src/a11y-gate.test.ts`'s stylesheet case
guards the OUTCOME either way — but only where a build exists, which in CI it
never does (`pnpm -r test` runs before `just build-storybook`), so on review
that case became an explicit skip and `just build-storybook` itself gained the
assertion. Verified by mutation: with `app/globals.css`'s `@import` moved
below `@plugin`, the recipe exits 1 printing
`dashboard --color-base-100 defined 0x, referenced 22x`.

Full detail, every gate's real numbers, the six-case mutation table and the
adversarial review's own findings are in
[../../status/verification/2026-09-13-dashboard-storybook.md](../../status/verification/2026-09-13-dashboard-storybook.md).
