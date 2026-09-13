# Verification log — 2026-09-13, the dashboard shell's "More" drawer

Last verified: 2026-09-13, on `claude/dash-shell-drawer`, off `8e58195`. Lane
A of `docs/plans/2026-09-13-dashboard-nav-notes/plan.md`.

## What this claims, and what it does not

It claims: `AppShell` gained a "More" drawer
(`frontends/apps/dashboard/src/components/more-menu.tsx`) built on
`@vaam-apps/ui`'s generic `Drawer` composition, holding the nav tree rendered
from the same `NAV_ENTRIES` array `SideNav` reads, a `ThemeSwitcher`, and a
second `SignedInBar`. The theme switcher is now reachable at every viewport
width — before this change, `SideNav` only ever rendered `accountSlot` in
the ≥1280px in-flow sidebar, so a phone or a tablet had no theme control on
the page at all. The signed-in identity (email, merchant id, sign-out) stays
exactly where it was, always visible in `<main>`, unconditionally — it did
**not** move into the drawer, because `dashboard.cy.ts` asserts
`cy.contains(staffEmail()).should("be.visible")` with eight further tests
chained after it in a `testIsolation: false` sequence, and hiding the
identity behind a closed drawer would fail that assertion and all eight.

It does **not** claim the rail grew. `resources.ts`'s `NAV_ENTRIES` is
unchanged — one entry, `Payments` — and the drawer's nav tree reads that same
array rather than a second hand-kept list, so growing the rail later (a
different lane) is a data change to `resources.ts` and nothing in this file.
It does not claim a real browser has looked at the drawer: no Cypress case
opens it, drags it, or checks its contents at a real viewport. It does not
claim the drawer is a bottom sheet — see "What was measured and rejected"
below for why it is a right-hand panel instead.

## Sign-out is still a `<form>` POST

Unchanged, in both places it now renders (`<main>`'s `SignedInBar` and the
drawer's second `SignedInBar` — the same component, not a second
implementation). `app-shell.test.tsx`'s
`"puts sign-out in a form, never behind a link"` case is untouched and still
passes.

## What was measured and rejected: a custom bottom-sheet layout

The plan's own framing of the maintainer's ask was "theme switcher in a
bottom-sheet drawer". `@vaam-apps/ui`'s `MoreDetailDrawer` already ships
exactly that responsive split (bottom sheet below `md`, right panel at
`md`+) for record detail, but it is written for a record's full detail view
(title = the record, footer = destructive actions) — this drawer holds
navigation and settings, not a record, so reusing it would be a semantic
mismatch, not a shortcut.

Reproducing the same responsive split by hand on the **generic** `Drawer`
composition (`direction="bottom"` plus overriding `DrawerContent`'s default
right-edge CSS to reposition it at the bottom on narrow screens and back to
the right edge at `md`+) was tried and measured against `just lint-web`'s
`verify-ui` gate: check 7a-iii caps a `className` written in an app at 60
characters, and no literal string carrying the sheet-on-phone,
panel-on-desktop rule set (inset overrides, rounding, border side, and their
`md:` counterparts) fits under that cap. The generic composition's own
documented pairing — `direction="right"` with `DrawerContent`'s default
CSS — was used instead. On a phone, `DrawerContent`'s default
`w-full max-w-[560px]` already covers the viewport edge-to-edge, so the
practical difference from a bottom sheet is which edge it slides from, not
how much of the screen it covers.

## Gates run

From the repo root, on this branch's head:

- `pnpm --filter @vpay/dashboard typecheck` — clean, no errors.
- `just lint-web` (`pnpm -r typecheck` + `pnpm -r lint` across all 14
  buildable workspace projects) — exit 0, no findings.
- `just verify-ui` — silent, exit 0 (the gate's own contract: it prints
  nothing on success).
- `just test-web` (`pnpm -r test`, all workspace projects) — every project
  passed: `frontends/apps/dashboard` **303 tests in 29 files, 0 skipped**;
  the other thirteen projects (checkout, the shop example, both SDKs, the
  shared packages) unaffected by this change, also green. No test file was
  edited and no assertion was weakened to reach this.

## What this did not do

- No Cypress run. `frontends/tests/e2e/cypress/e2e/dashboard.cy.ts` was
  read (to confirm the identity-visibility constraint above) but not
  executed against a real browser and a real stack — that needs
  `compose.e2e.yml`, which this pass did not stand up. The drawer's nav
  tree, its `DrawerTitle`/`DrawerDescription` wiring, and its behaviour at
  the real 375px/1000px/1440px bands are therefore unverified in a browser,
  only in jsdom (`a11y.test.tsx`, `app-shell.test.tsx`) and by `tsc`.
- No visual check of the right-hand panel at any real viewport width — no
  screenshot was taken of this change.
- No change to `resources.ts`, the backend, or any nav entry — out of
  scope for this lane by the plan's own text.
