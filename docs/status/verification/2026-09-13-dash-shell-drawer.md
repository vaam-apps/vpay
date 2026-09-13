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

## CORRECTED BY THE REVIEW, 2026-09-13 — the section below was wrong

**The bottom sheet was reachable, and it is what ships now.** The section
that follows is left in place verbatim because the reasoning it records is
the reasoning that produced the right-hand panel, and striking it out would
hide the mistake rather than correct it. What it got wrong:

- It is true that check 7a-iii caps an app's `className` at 60 characters
  (and check **7a-i** is stricter still — it forbids a computed `className`
  in an app at all, so `cn()` is not a way round it either). Both were
  re-read in the `justfile` and both are real.
- But neither gate had to be satisfied, because **the responsive split
  never needed to be written at the call site.** `MoreDetailDrawer` already
  carries it: `drawer.js`'s `DetailDrawerContent` opens `direction="bottom"`
  with `inset-x-0 bottom-0 rounded-t-box border-t`, and resets every one of
  those at `md:` (`md:inset-x-auto md:inset-y-0 md:right-0 md:h-full
md:rounded-t-none md:rounded-l-box md:border-t-0 md:border-l`).
- The "semantic mismatch" argument below — that `MoreDetailDrawer` is "for a
  record's full detail view" — does not survive its own type. `DetailDrawerProps`
  is `open`, `onOpenChange`, `title`, `description`, `children`, `footer?`,
  `className?`. Nothing in it is record-shaped, and `footer` (the
  "destructive actions" the argument names) is optional and is not passed.

Measured on the shipped rewrite, a real jsdom render with the drawer open —
the panel's own class attribute, read back off the DOM:

```text
fixed z-50 flex flex-col bg-surface-2 shadow-[var(--shadow-dialog)] outline-none
inset-x-0 bottom-0 rounded-t-box border-edge border-t
md:inset-x-auto md:inset-y-0 md:right-0 md:h-full md:max-h-none
md:w-full md:rounded-t-none md:rounded-l-box md:border-t-0 md:border-l
max-h-[92vh] md:max-w-[680px]
```

`just verify-ui` is silent and exits 0 on that file, and the longest
`className` written in it is 47 characters.

One thing the 60-character cap **does** still block, stated rather than
buried: a _visual_ active style on a drawer row. `aria-current="page"` is
the only current-entry signal the drawer carries. See `more-menu.tsx`'s own
doc comment for the two measured strings (79 and 65 characters).

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

## The review's own gates, 2026-09-13

Run from the repo root on `claude/dash-shell-drawer-review`, under the
toolchain CI pins (`.nvmrc` — Node **22.23.2** via nvm, not the host's
24.20.0) and `pnpm install --frozen-lockfile`:

- `pnpm --filter @vpay/dashboard typecheck` — exit 0, no errors.
- `just verify-ui` — silent, exit 0.
- `just lint-web` — exit 0 across all 14 buildable workspace projects.
- `just test-web` — exit 0. `frontends/apps/dashboard`: **311 tests in 30
  files, 0 skipped, 0 todo** (was 303 in 29 — the +8 are the seven new
  `more-menu.test.tsx` cases and the one new open-drawer case in
  `a11y.test.tsx`). The other thirteen projects unchanged and green:
  checkout 523, shop 108, nodejs SDK 211, stripe-js SDK 146, config 60,
  tokens 10, api-client 4.

No assertion was weakened and no test was deleted to reach that.

### What the new tests are worth: every one was made to fail first

Each mutation was applied to the source, the suite was run, the named case
went red, and the source was restored:

| Mutation                                                                           | Case that went red                                                                                                                                                                                              |
| ---------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `isNavActive(...)` → `entry.href === currentPath` (the shipped rule)               | `marks the current entry the way the rail does` — _"the drawer and the rail must agree about /payments at /payments/pi_3Nk: expected false to be true"_                                                         |
| drawer's list hardcoded to one literal entry **and** `DASH_RESOURCES` grown to two | `renders one row per NAV_ENTRIES entry` (_expected length 2, got 1_) **and** the a11y open-drawer guard (_expected 1 to be 2_)                                                                                  |
| `description` prop removed                                                         | `carries the title and description vaul requires` — _expected 'Details panel.' to be 'Navigation, theme, and your account.'_ (the sr-only fallback, i.e. the wiring was checked, not the element)               |
| `title="More"` → `title="Something else"`                                          | the same case, **and** `leaves the document with exactly one <h1>`                                                                                                                                              |
| `<ThemeSwitcher />` removed                                                        | `puts the theme control inside the drawer` — _Unable to find an accessible element with the role "radiogroup"_                                                                                                  |
| `SignedInBar`'s `<form>`+submit button → `<a href>`                                | `keeps sign-out a form POST inside the drawer` **and** `never puts two sign-outs in the accessibility tree at once`                                                                                             |
| a third `SignedInBar` added to `<main>`                                            | `never puts two sign-outs…` — _expected length 1, got 2_                                                                                                                                                        |
| drawer's `<li>` → `<div>` inside the `<ul>`                                        | the a11y open-drawer case — axe reports `list: <ul class="flex flex-col gap-1">`. **The eight pre-existing cases in that file stayed green**, which is the proof that they never saw the drawer's markup at all |

### Two gaps in the harness the review had to close first

- **`window.matchMedia` was missing from `vitest.setup.ts`.** vaul calls
  `window.matchMedia('(display-mode: standalone)')` (`vaul/dist/index.mjs:855`)
  from a passive effect. jsdom has no such function, so mounting an open
  drawer threw — and vitest reports a throw from a passive effect as an
  _unhandled error beside a **passing** test_. Any drawer test written
  without this polyfill would have gone green while the drawer never opened.
- **`a11y.test.tsx`'s first two cases ASSIGN `document.body.innerHTML`**, and
  Testing Library's `cleanup` does not undo that. The new open-drawer case
  clears the body first; without it, it found the previous case's stale
  "More" button (`getByRole` threw "found multiple elements") and axe would
  have been scanning a second, stale copy of the shell.

## A pre-existing defect this change did not cause, and did not fix

React logs _"Encountered two children with the same key, `/payments`"_ on
every render of the signed-in shell. `SideNav`'s `HorizontalRail` (the
sub-640px pill) builds `[topItem, ...groups.flatMap(g => g.items)]` and keys
each by `item.href`; `app-shell.tsx` passes `topItem.href = first?.href ?? "/"`,
which is the same `/payments` as the one nav entry, so both children carry
the same key.

**Verified to predate this change, not assumed to:** `app-shell.tsx` was
checked out at `origin/claude/dashboard-nav-enhance-ba4560` with
`more-menu.tsx` deleted, and a render at `/payments/pi_3Nk` logged the same
warning. It is therefore left alone here — fixing it means changing what
`topItem` points at, which is nav IA and belongs to the plan's Lane E, or
changing how `@vaam-apps/ui` keys that array, which is upstream. **Filed for
the maintainer rather than fixed.** The implementer reported it and was right
about it.

## What this did not do

- No Cypress run. `frontends/tests/e2e/cypress/e2e/dashboard.cy.ts` was
  read (to confirm the identity-visibility constraint above) but not
  executed against a real browser and a real stack — that needs
  `compose.e2e.yml`, which this pass did not stand up. The drawer's nav
  tree, its `DrawerTitle`/`DrawerDescription` wiring, and its behaviour at
  the real 375px/1000px/1440px bands are therefore unverified in a browser,
  only in jsdom (`a11y.test.tsx`, `app-shell.test.tsx`, `more-menu.test.tsx`)
  and by `tsc`.
- **No visual check at any real viewport width, and this is the biggest
  remaining gap.** No screenshot was taken, at 375px or anywhere else. The
  bottom-sheet claim above rests on the class attribute read back off a
  jsdom render and on `MoreDetailDrawer`'s own source — jsdom applies no
  CSS and computes no layout, so _"the element carries `inset-x-0 bottom-0`
  below `md`"_ is what was measured, **not** _"it looks like a bottom sheet
  on a phone"_. Those are different claims and only the first one is
  evidenced here.
- No change to the backend, to `DASH_RESOURCES`, or to any nav entry — out
  of scope for this lane by the plan's own text. `resources.ts` did gain one
  function, `isNavActive`: the current-entry rule the rail and the drawer
  now share. No route, resource or label changed.
- The drawer's rows have no visual active style (see the correction at the
  top) — `aria-current` only.
- The React duplicate-key warning above is reported, not fixed.
