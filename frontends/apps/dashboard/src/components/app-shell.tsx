"use client";

import { SideNav, ThemeSwitcher } from "@vaam-apps/ui";

import { SignedInBar } from "./signed-in-bar";
import { LayoutDashboard } from "lucide-react";
import { usePathname } from "next/navigation";

import { NAV_ENTRIES } from "../dash/resources";

/**
 * The signed-in chrome: the rail, who is signed in, and the way out.
 *
 * # The rail is rendered from the resource registry, not from a second list
 *
 * `NAV_ENTRIES` is derived from `DASH_RESOURCES` — the same array Refine
 * routes on — so a resource with no page, or a page missing from the rail,
 * is a failing test rather than a dead link. That is Lane 4's whole point,
 * and it replaces `nav.tsx`'s hand-kept `NAV_LINKS`.
 *
 * # `currentPath` comes from the caller
 *
 * `@vaam-apps/ui` has no dependency on Next, so `SideNav` takes the active
 * path rather than reading a router. `usePathname()` is that value here.
 *
 * # Sign-out stays a POST to the existing Server Action
 *
 * It is a footer item that submits a form, not a link. Issue #88 item 4: a
 * sign-out reachable by `GET` is the action an `<img src>` or a link scanner
 * fires, and `signed-in-bar.tsx` already made it a POST for that reason. The
 * action itself is unchanged and still revokes before it clears.
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
        `accountSlot`: `SideNav` drops that slot below `lg`, which would take
        the document's only <h1> away on a narrow screen. The rail already
        shows the brand visually through `topItem`, so this carries the name
        for a screen reader without drawing it twice.

        Still an <h1>, with every screen's own title an <h2> under it — the
        hierarchy `dashboard.cy.ts` asserts in thirteen places.

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
      <SideNav
        /*
          A distinct icon from the group below. `topItem` and the one
          `Payments` entry both rendered `CreditCard` at first, so the rail
          showed the same glyph twice with nothing to tell them apart —
          visible in `03-payments.png` from the e2e run before this line
          existed. The top item is the console; the entry is the resource.
        */
        topItem={{
          label: "Console",
          href: first?.href ?? "/",
          icon: LayoutDashboard,
        }}
        groups={[
          {
            label: "Observe",
            items: NAV_ENTRIES.map((entry) => ({
              label: entry.label,
              href: entry.href,
              icon: entry.icon,
            })),
          },
        ]}
        footerItems={[]}
        currentPath={pathname}
        /*
          The theme control only. **The identity and the sign-out are NOT
          here, and that is a fix rather than a preference.**

          `SideNav` does not render `accountSlot` below `lg` — its own doc
          says so, and ~52px has no room for an email address. Putting the
          signed-in identity there meant that at the e2e viewport (1000px)
          it was not on screen at all, and `dashboard.cy.ts`'s
          `cy.contains(staffEmail()).should("be.visible")` failed — taking
          the eight tests that follow it in that `testIsolation: false`
          sequence with it. The rail is navigation; who is signed in and the
          way out belong somewhere always visible.
        */
        accountSlot={<ThemeSwitcher />}
      />
      {/*
        `smallScreen` is left at its default, `"floating"` — this app owns
        no drawer, and `side-nav.d.ts` names `"off-canvas"` for a caller
        that does. `"off-canvas"` was tried here and measured wrong: its
        `<nav>` is unconditionally `w-full`, and as a flex sibling of
        `<main>` that claimed the entire row below `lg` — `main` computed
        to 0px wide with the page scrolling sideways (a real render, real
        `app/globals.css`, 375px: `main` width 0, `document.scrollWidth`
        515 > `clientWidth` 375).

        `"floating"` never draws a box of its own below `xl` — by design,
        so a caller who does own a drawer gets no second empty band — so
        nothing here reserves room for `FloatingRailPortal`'s pill, which
        is `position: fixed` and portaled to `document.body`, outside this
        flex row entirely. Measured on the same real render with no padding
        here: the bottom pill (`<640px`) sits at `left 135–241, top
        830–888` inside a 375×900 viewport, inside `main`'s own box; the
        vertical pill (`640–1279px`) sits at `left 12–64`, to the left of
        `main`'s content, which starts at `x=16`. Both genuinely overlap
        `main`.

        `pb-20` (80px) clears the bottom pill's height (58px) plus its
        `bottom-3` offset (12px) below `sm`; `sm:pl-20` (80px) clears the
        vertical pill's right edge (`x=64`) with room to spare from `sm` up
        to `xl`, where the real in-flow sidebar takes over and `xl:pl-0`
        gives the padding back.
      */}
      <main className="min-w-0 flex-1 pb-20 sm:pb-0 sm:pl-20 xl:pl-0">
        <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 p-6">
          <SignedInBar
            email={email}
            merchantId={merchantId}
            signOut={signOut}
          />
          {children}
        </div>
      </main>
    </div>
  );
}
