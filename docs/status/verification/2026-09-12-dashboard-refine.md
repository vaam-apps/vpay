# Verification log — 2026-09-12, the dashboard on Refine

Last verified: 2026-09-12, on `claude/refine-lane3`, off `99580522`. Lanes 3
and 4 of `docs/plans/exp55-refine-seam-bff-notes/refine-plan.md`; Lanes 1 and
2 (the seam and the BFF) landed in #123.

## What this claims, and what it does not

It claims the payments list and detail render through Refine hooks against
the BFF, that the rail and Refine read one array, and that the auth stack is
unchanged and still server-side.

It does **not** claim the dashboard grew a screen. `/dash/v1` serves one
resource — `payment_intents`, a cursor-paged list and a get-by-id — so there
is one section in the rail. **A console with fifteen sections is what a
backend exposes, not what a framework provides**, and entries for routes
nobody serves would be a rail full of dead links. It does not claim any
write works: `/dash/v1` answers `403` to every non-`GET` and the provider
throws rather than discovering that at submit time.

## The gate did not move to the browser

Both pages are still Server Components and both still call `requireStaff()`
before anything renders. The 80 %-of-TTL re-mint, the
`X-Vpay-Staff-Session` header and the single-`401` retry all still run on the
server, and `/api/dash` re-does the same gate per request — so the credential
is enforced **twice server-side and never once in a browser**. No file in
`src/dash/` has seen a bearer token.

`app/(dash)/layout.tsx` is a client component and its children are still
server components: Next passes them as a slot.

## The decisive rule, and the mutation that proves it

**`401` and only `401` is a sign-out.** `403` is an outage — on this surface
it means vpay is behind something that refused this app, a deployment problem
and not a fact about the person. `authProvider.onError` mirrors `refusalFor`
exactly rather than approximating it, and `checkSession` — the Server Action
behind Refine's `check()` — applies the same rule instead of reusing
`alreadySignedIn()`, which answers `false` for an outage as well and would
have signed everyone out of a rolling deploy.

That is issue #88 item 2, and widening `onError` to `>= 400` is the mutation
the plan names:

| Mutation                                          | Result             |
| ------------------------------------------------- | ------------------ |
| `onError` widened to `>= 400`                     | **9 of 15 failed** |
| `onError` widened to `!== 200`                    | 10 of 15 failed    |
| `logout` stops calling the existing Server Action | 1 failed           |
| repeated query param joined instead of first-wins | 1 failed           |
| unparseable date forwarded                        | 1 failed           |
| both cursors sent at once                         | 1 failed           |
| `total` guessed from `data.length`                | 1 failed           |

## Lane 4 — one array, not two lists

`src/dash/resources.ts` replaces `src/nav.tsx`'s `NAV_LINKS`, which is
deleted. Refine routes from `REFINE_RESOURCES` and the rail renders from
`NAV_ENTRIES`, derived from the same array — so the two cannot disagree.
`layout.test.tsx`'s gate moved onto it: a `resources` entry for a page nobody
wrote fails, which is the same mutation that used to fail against
`NAV_LINKS`.

`pageExists` had to learn about route groups. `app/(dash)/payments` serves
`/payments`, and a naive join looked for `app/payments/page.tsx`, found
nothing, and reported every signed-in route as dangling — which is exactly
what it did the moment the routes moved. It tries the direct path and then
each top-level group, and `/webhooks` still answers `false`.

## What changed in the chrome, and what deliberately did not

The rail is `@vaam-apps/ui`'s `SideNav`, with `ThemeSwitcher` (0.1.2's light
theme is opt-in and the app still pins `dark` on `<html>`). The root layout
is the document and nothing else: it used to render the nav for **every**
route, so the sign-in form carried a link to a page nobody at that point
could open. `app/login/layout.tsx` is the signed-out wrapper.

**The heading hierarchy is unchanged, and that was a decision.**
`ScreenHeader` renders an `<h1>`, and using it for page titles would have
made "Payments" an `<h1>` and left the app with no brand heading. Thirteen
assertions in `dashboard.cy.ts` pin `h2` + "Payments", and the existing
shape — one `<h1>` naming the app, an `<h2>` naming the section — already
passes the axe suite. Changing a tested contract because a component prefers
a different level is not a reason, so the screens keep `<h2>` and the brand
`<h1>` moved into the shell as `sr-only`. It is **not** inside `accountSlot`:
`SideNav` drops that slot below `lg`, which would take the document's only
`<h1>` away on a narrow screen.

**Paging stays `<a href>`.** `@vaam-apps/ui`'s `Pagination` renders
callback-driven buttons and turns an absent control into a _disabled_ button
rather than omitting it; two Cypress legs assert on anchors. `PaymentsPager`
is unchanged, and `pagerHrefsFromCursors` is new beside `pagerHrefs` so the
backward-page `has_more` inversion is resolved in the seam once rather than
re-derived in a screen.

## A build failure worth recording

A client component importing the resource constant from `src/dash/provider.ts`
dragged the whole server graph — `readDash`, the config, `node:crypto` —
into the browser bundle, and `next build` failed with _"Reading from
`node:crypto` is not handled by plugins"_. A constant is not a reason to
bundle a credential path: `src/dash/resource-name.ts` holds the string and
`provider.ts` re-exports it, so there is still one spelling.

## The filter row — what the review measured, not what it assumed

The payments filters became one row and the two date inputs became
`@vaam-apps/ui`'s `DateRangePicker`. The adversarial review of that change
found five things the suite would not have caught, each with the mutation
that proved it; all five are fixed and all five mutations now fail.

| Mutation                                                   | Before    | After   |
| ---------------------------------------------------------- | --------- | ------- |
| `DateRangePicker`'s `onValueChange` → `() => {}`           | 5 green   | 2 red   |
| the `<select>`'s `onChange` → `() => {}`                   | 5 green   | 1 red   |
| four of the five statuses deleted from the `<option>` list | 5 green   | 2 red   |
| `` `/payments?${query}` `` → `` `/wrong?${query}` ``       | 5 green   | 5 red   |
| the status field's `<label htmlFor>` pointed at nothing    | axe green | axe red |

The first two are the same shape, and it is the one this component's own
doc warns about: the old `method="get"` form needed **no** change handler —
the browser read the DOM at submit — so moving the picks into client state
created two pieces of wiring that, deleted, leave a control that renders,
looks right, is clickable and silently does nothing. The cases that replaced
the `FormData` ones all seeded `values` and asserted the seed came back out,
which is exactly what that failure also does. They now drive the real
controls: `fireEvent.change` on the select, and two clicks in the calendar
the popover actually opens.

The last one is an axe gap, not a component defect: **axe-core 4.13's `label`
rule has the selector `input, textarea`** — a `<select>` is covered by the
separate `select-name` rule, which `a11y.test.tsx`'s `STRUCTURAL_RULES` did
not list. The one `<select>` in this app had its accessible name checked by
nothing. `select-name` is now in the list.

The date range control's accessible name was **verified against the package's
compiled source rather than taken on trust**, and it is real: `PickerTrigger`
renders `placeholder` in an `sr-only` span, so the trigger computes
`"Created between Created between"` empty and
`"2026-09-01 → 2026-09-07 Created between"` with a range. Emptying that
`placeholder` makes `button-name` fire in the axe suite, so the coverage is
not vacuous either. The residual cost is recorded in the component: once a
range is picked, nothing **visible** says what those dates mean.

### One line, measured in a real browser

`flex` with no `flex-wrap` is why it never wraps. It is not why it fits:
rendered against this app's own compiled `globals.css` at a range of widths,
the three controls are **211 px, 262 px and 71 px at every container width
from 1104 px down to 320 px** — nothing shrinks. The `<select>`'s intrinsic
minimum is its widest option (`requires_payment_method`), and the picker's
`truncate` never engages. So below ~568 px of content the row spilled, and as
a direct child of `ScreenStack` — whose `min-w-0` protects the column, not
its children — it took the **whole document** into horizontal scroll.

| Content column | Row, as first written                           | Row, with `min-w-0 overflow-x-auto`        |
| -------------- | ----------------------------------------------- | ------------------------------------------ |
| 1024 px        | one line, no scroll                             | identical — nothing to scroll              |
| 420 px         | document `scrollWidth` 591 vs `clientWidth` 420 | 420 vs 420; the row scrolls in its own box |

One line in both cases (all three controls share one bottom edge). Safe with
the calendar, checked and not assumed: `PopoverPanel`'s `anchor` prop forces
`portal` in `@headlessui/react`'s `popover.js`, so the panel is not inside
the scroll container.

### Two things found and deliberately NOT fixed here

- **`AppShell` hands `SideNav` `smallScreen="off-canvas"` without owning a
  drawer.** That mode is documented as "for a consumer who already owns a
  drawer"; below `lg` the `<nav>` is `w-full` and, as a flex sibling of
  `<main>`, takes the entire viewport — measured at 375 px: `main` is 0 px
  wide, the content column is 48 px of padding, and the page scrolls
  sideways whatever the filter row does. That is the shell's decision
  (floating rail, or own the drawer), not this component's.
- **The axe suite's landmark case renders `LoginLayout`, the signed-out
  shell, and `AppShell` is in no axe case at all.** The `(dash)` composition
  — the one an operator uses — has no `region` gate. Adding one would put
  upstream `SideNav` markup under this app's gate, which is a decision above
  a review of these three files.

## Numbers

`just test-web` **1 360 passing, 0 skipped** — dashboard 258 → 289 → **298**
(the last nine from the filter-row review above: four new cases, plus the
five existing ones strengthened). `just verify-ui` exit 0 — and proven to
bite rather than merely pass, by adding a 61-character `className` to
`payments-filters.tsx` and watching it exit 1. `pnpm --filter @vpay/dashboard
typecheck`, `lint` and `next build` all exit 0.

Bundle: `/payments` and `/payments/[id]` are **306 kB** First Load JS, from
235 kB. The three `/login` routes stay at **234 kB** — the route group scopes
Refine to the signed-in pages, so a visitor who cannot sign in does not
download a data framework. That is the plan's R10 measurement, taken.
