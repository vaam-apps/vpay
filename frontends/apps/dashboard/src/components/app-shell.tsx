"use client";

import { SideNav, ThemeSwitcher } from "@vaam-apps/ui";

import { SignedInBar } from "./signed-in-bar";
import { CreditCard } from "lucide-react";
import { usePathname } from "next/navigation";

import { NAV_ENTRIES } from "../dash/resources";

/**
 * The signed-in chrome: the rail, its "More" sheet, who is signed in, and
 * the way out.
 *
 * # The rail is rendered from the resource registry, not from a second list
 *
 * `NAV_ENTRIES` is derived from `DASH_RESOURCES` — the same array Refine
 * routes on — so a resource with no page, or a page missing from the rail,
 * is a failing test rather than a dead link. That is Lane 4's whole point,
 * and it replaces `nav.tsx`'s hand-kept `NAV_LINKS`. The More sheet
 * `SideNav` opens below 1280px draws its rows from the same `groups`, so
 * nothing in this app keeps a second copy of the tree.
 *
 * # `currentPath` comes from the caller
 *
 * `@vaam-apps/ui` has no dependency on Next, so `SideNav` takes the active
 * path rather than reading a router. `usePathname()` is that value here.
 *
 * # Sign-out stays a POST to the existing Server Action
 *
 * It is a form that submits, not a link. Issue #88 item 4: a sign-out
 * reachable by `GET` is the action an `<img src>` or a link scanner fires,
 * and `signed-in-bar.tsx` already made it a POST for that reason. The
 * action itself is unchanged and still revokes before it clears, in every
 * place it renders: `<main>`, the sidebar and the More sheet.
 *
 * # History: the Menu pill, 2026-09-13 to 2026-09-24
 *
 * Until `@vaam-apps/ui` 0.4.0, `SideNav` rendered `accountSlot` only in
 * the ≥1280px sidebar, so below it this app had nowhere in the library's
 * chrome for sign-out, the theme control or a labelled nav list. It
 * floated its own "Menu" pill beside the rail instead (`more-menu.tsx`,
 * a drawer holding those three), and 0.3.0's 288px bottom bar then drew
 * over it on phones: by 26.5px at 375px and 54px of its 58px at 320px.
 * On vaam-apps/vpay#258's branch, while it carried 0.3.0, the pill was
 * lifted above the bar (`bottom-24`), with `<main>` at `pb-40` to clear
 * both, as an interim while vaam-apps/ui#36 asked for the slot to be
 * reachable from the toolbar itself. 0.4.0 shipped that before the branch
 * merged, so `master` went from the 0.2.x pill (`bottom-3 left-3`, beside
 * `pb-20`) straight to none, and 0.4.0's release notes name this pill as
 * the chrome to remove. The pill and its drawer are gone; the account
 * block is `accountSlot` at every width, and the sheet behind the
 * toolbar's "More" control carries it.
 *
 * One thing the drawer gave that the sheet does not: below 640px there is
 * no longer a full nav list with visible labels. The phone bar shows four
 * of the five destinations as icons, named for a screen reader by
 * `sr-only` text, and its sheet lists only the one that does not fit.
 * That is the library's M3 toolbar as designed; it is recorded here
 * because the pill's drawer used to be the answer. From 640px the rail's
 * drawer lists all five, labelled.
 */
export interface AppShellProps {
  readonly email: string;
  readonly merchantId: string;
  readonly signOut: () => Promise<void>;
  readonly children: React.ReactNode;
}

export function AppShell({
  email,
  merchantId,
  signOut,
  children,
}: AppShellProps) {
  const pathname = usePathname();
  const first = NAV_ENTRIES[0];

  return (
    <div className="flex min-h-screen">
      {/*
        The app's one <h1>, and it is `sr-only` on purpose.
        It lived in the root layout's <nav> and moved here with the rail,
        because that layout wraps `/login` too and a sign-in form should not
        carry a link to a page nobody signed in can open. It is NOT inside
        `accountSlot`: below `xl` that slot is `display: none` in the
        sidebar and mounted in the More sheet only while the sheet is open,
        which would take the document's only <h1> away on a narrow screen,
        and put a second one in while the sheet showed. The rail already
        shows the brand visually through `topItem`, so this carries the name
        for a screen reader without drawing it twice.

        Still an <h1>, with every screen's own title an <h2> under it — the
        hierarchy `dashboard.cy.ts` relies on: it finds each screen by its
        <h2> (`cy.contains("h2", …)`, fifteen times as of 2026-09-24).

        Wrapped in a bare `<header>` — a landmark (implicit `banner` role)
        because it is not nested in `<article>`/`<aside>`/`<main>`/`<nav>`/
        `<section>` — rather than left a direct child of this flex row: axe's
        `region` rule ("all page content should be contained by landmarks")
        does not exempt content that is visually hidden, only content outside
        the accessibility tree entirely, and `sr-only` keeps this in it. A
        bare `<div>` here is not a landmark and this h1 was the one thing on
        the signed-in shell sitting outside one — found by `a11y.test.tsx`'s
        case for this composition, which had no coverage at all until it was
        added.
      */}
      <header>
        <h1 className="sr-only">vpay dashboard</h1>
      </header>
      {/*
        The in-flow sidebar is full height and pinned to the top, and both
        halves of that need this wrapper because `SideNav` takes no
        `className`.

        **Measured before the change** (1440x900, real render): the sidebar
        carries `xl:h-full`, but its only ancestor is `flex min-h-screen` —
        a *min*-height, not a definite one — so `height: 100%` resolved to
        `auto` and the nav was **407px tall** with `position: static`. It
        ended partway down the viewport and scrolled away with the page.

        `xl:h-screen` gives the definite height `xl:h-full` needs, and
        `xl:sticky xl:top-0` pins it. The nav's own `xl:overflow-y-auto`
        then scrolls the nav's contents rather than the page when the list
        outgrows the viewport.

        `contents` below `xl` is deliberate: at those widths the rails are
        `position: fixed` and portalled to `document.body`, so a box here
        would be an empty column the rails never occupy. `display: contents`
        adds no box at all, which keeps every measurement in the comment on
        `<main>` below exactly as it was.
      */}
      <div className="contents xl:sticky xl:top-0 xl:block xl:h-screen">
        <SideNav
          /*
          A distinct icon from the group below. `topItem` and the one
          `Payments` entry both rendered `CreditCard` at first, so the rail
          showed the same glyph twice with nothing to tell them apart —
          visible in `03-payments.png` from the e2e run before this line
          existed. The top item is the console; the entry is the resource.
        */
          /*
            The first destination IS the top item, rather than a "Console"
            row pointing at it.

            `topItem` is required by `SideNav` and there is no overview page
            to give it, so it used to be a second row carrying the FIRST
            entry's href — `/payments` twice over. That was two things at
            once: a redundant row, and issue #163's React duplicate-key
            warning, because the sub-640px bar keys `[topItem, ...items]`
            by href.

            Taking that entry out of the group below leaves no repeat. This
            said "four distinct destinations", and that four was exactly
            what the phone bar shows before it needs an overflow; a fifth
            resource (Customers) made both stale. With five, the bar shows
            four and the fifth is a labelled row in its More sheet, above
            the account block.
          */
          topItem={
            first ?? { label: "Payments", href: "/payments", icon: CreditCard }
          }
          groups={[
            {
              label: "Observe",
              // `.slice(1)` — the first entry is `topItem` above, and a
              // destination in both places is the duplicate this just
              // removed.
              items: NAV_ENTRIES.slice(1).map((entry) => ({
                label: entry.label,
                href: entry.href,
                icon: entry.icon,
              })),
            },
          ]}
          footerItems={[]}
          currentPath={pathname}
          /*
            The account block, at EVERY width. `SideNav` shows it in the
            ≥1280px sidebar and, below that, in the sheet its toolbar's
            "More" control opens: a bottom sheet from the phone bar, a
            navigation drawer from the 640–1279px rail (0.4.0,
            vaam-apps/ui#36). Its release notes are explicit — pass the
            account markup as `accountSlot` at every width and remove your
            own floating account chrome — and this app's Menu pill is the
            example they name (see the module doc's history).

            Until 0.4.0 this slot was given content only while a
            `matchMedia("(min-width: 1280px)")` matched. The sidebar's copy
            is rendered at every width, hidden by CSS below `xl` and FIRST
            in the DOM, and `dashboard.cy.ts`'s `cy.contains(staffEmail())`
            at its 1000px viewport found that invisible copy rather than the
            one in `<main>` — two real CI failures, one of them a sign-out
            it could not click (2026-09-14). That gate would now empty the
            sheet as well: with no account block the phone bar's overflow
            goes back to a menu of links and the rail gains no control at
            all. So the gate is gone, the hidden copy is back, and
            `dashboard.cy.ts` scopes its identity and sign-out queries to
            `<main>`. The package's guidance asks a test that finds the
            account block by its text to scope the query; it names the
            sheet, and at 1000px `<main>`'s copy is the visible one.

            No hard-coded `id` anywhere in it: while the sheet is open the
            block is mounted twice (the sidebar's copy stays in the DOM
            under `display: none`). `SignedInBar` hard-codes none, and
            `ThemeSwitcher`'s radio group generates its own; a hard-coded one
            fails `a11y.test.tsx`'s open-sheet case on `duplicate-id`. And
            sign-out does not ask for confirmation; if it ever
            does, that has to be `InlineConfirm`, not a `Dialog` — below
            1280px the block sits in a modal sheet, and a dialog opened from
            inside it opens under the sheet's scrim, out of a pointer's
            reach.
          */
          accountSlot={
            <AccountBlock
              email={email}
              merchantId={merchantId}
              signOut={signOut}
            />
          }
        />
      </div>
      {/*
        `smallScreen` is left at its default, `"floating"`, and it is the
        only mode in which `accountSlot` reaches the More sheet at all.
        `side-nav.d.ts`'s `"off-canvas"` opt-in is for a caller that wraps
        `SideNav` ITSELF in a drawer to fill it — this app does not.
        `"off-canvas"` was also tried here and measured wrong: its `<nav>` is
        unconditionally `w-full`, and as a flex sibling of `<main>` that
        claimed the entire row below `lg` — `main` computed to 0px wide with
        the page scrolling sideways (a real render, real `app/globals.css`,
        375px: `main` width 0, `document.scrollWidth` 515 > `clientWidth`
        375).

        `"floating"` never draws a box of its own below `xl` — by design,
        so a caller who does own a drawer gets no second empty band — so
        nothing here reserves room for `FloatingRailPortal`'s pill, which
        is `position: fixed` and portaled to `document.body`, outside this
        flex row entirely. Measured on the same real render with no padding
        here, on `@vaam-apps/ui` 0.2.x: the bottom pill (`<640px`) sat at
        `left 135–241, top 830–888` inside a 375×900 viewport, inside
        `main`'s own box; the vertical pill (`640–1279px`) at `left 12–64`,
        to the left of `main`'s content, which starts at `x=16`. Both
        genuinely overlap `main`.

        **0.3.0 moved both rails** (M3 Expressive's floating toolbar: 64px
        across, 16px from the edge — its release notes: "the floating rails
        end 80px from their edge … content columns need `sm:pl-24` instead
        of `sm:pl-20`"). Re-measured 2026-09-24 in the built Storybook
        `Shell` story (the rails are `fixed` to the viewport, so the
        story's own padding does not move them): the bottom bar is 288px
        wide and 64px tall, `left 43.5–331.5, top 732–796` at 375×812 —
        five destinations are four slots and an overflow button, which is
        the full 288px the release notes tell a caller to clear — and the
        vertical rail is `left 16–80`.

        `sm:pl-24` (96px) clears the vertical rail's right edge (`x=80`)
        from `sm` up to `xl`, where the real in-flow sidebar takes over and
        `xl:pl-0` gives the padding back. Below `sm`, `pb-20` (80px) clears
        the bar — its 16px offset plus its 64px height — so the last row of
        a screen can be scrolled clear of it; the column's own `p-6` is the
        24px spare. It was `pb-20` against 0.2.x's pill too. `pb-40`
        (160px) cleared this app's Menu pill above 0.3.0's bar on
        vaam-apps/vpay#258's branch only and never reached `master` (the
        module doc's history).

        **0.4.0**, re-measured 2026-09-24 in the built Storybook `Shell`
        story (twenty rows since this change): the bar is the same 288px by
        64px — four destinations and the More control — at `left
        43.5–331.5, top 732–796` at 375×812 and `left 16–304, top 620–684`
        at 320×700, and the vertical rail is `left 16–80` at 640, 700 and
        1100px, More its last item. Scrolled to the end with the story's
        own 16px of padding zeroed, the last row ends 24px above the bar's
        top at both phone widths (708 against 732, 596 against 620), and
        nothing scrolls sideways at 320, 375, 640, 700, 1100 or 1280px.
        The `ShellPhone` and `ShellRail` stories assert the clearance (at
        least 16px) and the rail gutter in CI; before them nothing did.

        The bar also adds `env(safe-area-inset-bottom)` to its offset since
        0.3.0. That is 0 here, and deliberately not added to `pb-20`: this
        app sets no `viewport-fit=cover` (`app/layout.tsx` exports no
        `viewport`), so the browser keeps the page out of the unsafe area
        itself and the inset never reaches the page.
      */}
      <main className="min-w-0 flex-1 pb-20 sm:pb-0 sm:pl-24 xl:pl-0">
        <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 p-6">
          {/*
            `SignedInBar` stays here from `sm` to `xl`, although the More
            sheet carries it too. From 640 to 1279px the rail has no room
            for the identity, and this is the one copy that keeps the
            merchant being read on screen without a tap —
            `signed-in-bar.tsx` says why that is not a preference. From
            `xl` the sidebar's copy takes over.
          */}
          <div className="flex flex-wrap items-center gap-4 xl:hidden">
            {/*
              Hidden below `sm`. On a 375px phone this block wrapped to three
              lines — address, merchant id, then "Sign out" — and pushed the
              screen's own heading below the fold before a single row was
              read. The More sheet carries the same `SignedInBar`, so nothing
              is unreachable; it is one tap instead of zero.

              `sm` (640px) and not `md`, deliberately: `dashboard.cy.ts` runs
              at Cypress's default **1000px** viewport and asserts
              `cy.get("main").contains(staffEmail()).should("be.visible")`
              with eight tests chained behind it. 1000 is above `sm`, so that
              assertion still sees this copy. It is scoped to `<main>`
              because the sidebar's hidden copy comes first in the DOM (the
              comment on `accountSlot` above).
            */}
            <div className="hidden sm:block">
              <SignedInBar
                email={email}
                merchantId={merchantId}
                signOut={signOut}
              />
            </div>
          </div>
          {children}
        </div>
      </main>
    </div>
  );
}

/**
 * What `SideNav`'s `accountSlot` holds: who is signed in with the way out,
 * then the theme control — the sidebar's account block, and the More
 * sheet's.
 *
 * The visible "Theme" caption came with the Menu pill's drawer, which had
 * it for a reason that holds in the sheet too: three radios reading
 * System, Light and Dark under a sign-out row do not say what they switch.
 * The radio group keeps its own accessible name ("Theme", the package's
 * default), so the caption is for the eye.
 *
 * Both copies read the theme from `ThemeSwitcher`'s module-level store,
 * not from state in here, which is why the sheet's copy mounting fresh on
 * every open does not reset it.
 */
function AccountBlock({
  email,
  merchantId,
  signOut,
}: Omit<AppShellProps, "children">) {
  return (
    <div className="flex flex-col gap-3">
      <SignedInBar email={email} merchantId={merchantId} signOut={signOut} />
      <div className="flex flex-col gap-2">
        <span className="text-caption text-muted-foreground">Theme</span>
        <ThemeSwitcher />
      </div>
    </div>
  );
}
