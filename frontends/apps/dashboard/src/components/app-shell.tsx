"use client";

import { Badge, SideNav, ThemeSwitcher } from "@vaam-apps/ui";
import { usePathname, useRouter } from "next/navigation";

import { inAppNavigationTarget } from "../client-navigation";
import { NAV_GROUPS, OVERVIEW_NAV } from "../dash/resources";
import { Brand } from "./brand";
import { SignedInBar } from "./signed-in-bar";
import { ThemeToggle } from "./theme-toggle";

/**
 * The signed-in chrome: the rail, the top app bar, who is signed in, and
 * the way out. Rendered once by `app/(dash)/layout.tsx`, which Next keeps
 * mounted across navigations — see that file for why it is not per page.
 *
 * # Material 3 Expressive, as `@vaam-apps/ui` 0.5.0 draws it
 *
 * - **The rail** is `SideNav` in its default `floating` mode: the 64px
 *   floating toolbar along the bottom below 640px, the vertical one from
 *   640 to 1279px, and the full sidebar, in flow, from 1280px. Its top item
 *   is the Overview; the resources are sectioned (`NAV_GROUPS`, derived
 *   from the registry). `toolbarVariant="vibrant"`: the Overview scrolls
 *   stat tiles and cards under the toolbar, and the standard bar's
 *   `surface-2` fill is the same as theirs, so its edge disappeared over
 *   them — the case the library's own guidance names for `vibrant`.
 * - **The top app bar** is sticky above every screen: the mark, which
 *   merchant this dashboard reads (always on screen — an operator looking
 *   at an empty list must be able to tell "no payments" from "the wrong
 *   merchant"), the theme shortcut, and — from 640 to 1279px, where the
 *   rail has no room for an address — who is signed in and Sign out.
 *
 * # The rail's links are plain anchors; this file makes them client-side
 *
 * `SideNav` renders `<a href>` (the package has no Next dependency), and
 * until 2026-10-21 every rail click was a full document load. The wrapper
 * around it routes an ordinary click through the app router instead; see
 * `src/client-navigation.ts`. React's synthetic click bubbles here from the
 * portalled rails and the More sheet too, after the library's own handlers
 * (closing the sheet) have run.
 *
 * # Sign-out stays a POST to the existing Server Action
 *
 * It is a form that submits, not a link (issue #88 item 4: a sign-out
 * reachable by `GET` is what an `<img src>` or a link scanner fires). It
 * renders in the account block — the sidebar, and the More sheet below
 * 1280px — and in the top app bar from 640 to 1279px.
 *
 * # The account block is mounted twice while the More sheet is open
 *
 * The sidebar's copy stays in the DOM under `display: none`, first. So it
 * hard-codes no `id`, and `dashboard.cy.ts` scopes its identity and
 * sign-out queries to `<main>`, where the top app bar's copy is the
 * visible one at its 1000px viewport. A confirmation inside the block
 * would have to be `InlineConfirm`, never a `Dialog`: below 1280px the
 * block sits in a modal sheet, and a dialog opened from it opens under the
 * sheet's scrim.
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
  const router = useRouter();
  const routeRailClick = (event: React.MouseEvent) => {
    const to = inAppNavigationTarget(event, window.location.origin);
    if (to !== null) {
      event.preventDefault();
      router.push(to);
    }
  };

  return (
    <div className="flex min-h-dvh">
      {/*
        The app's one <h1>, `sr-only`, in a bare top-level <header> so it is
        inside a landmark (axe's `region` rule does not exempt visually
        hidden content). Every screen's own title is an <h2> under it — the
        hierarchy `dashboard.cy.ts` relies on (`cy.contains("h2", …)`). The
        visible mark is `Brand`, in the app bar, and is not a heading.
      */}
      <header>
        <h1 className="sr-only">vpay dashboard</h1>
      </header>
      {/*
        `contents` below `xl`, where the rails are `position: fixed` and
        portalled to `document.body`, so a box here would be an empty
        column. From `xl` the in-flow sidebar is full height and pinned:
        measured on a real render, without `xl:h-screen` its `xl:h-full`
        resolved against a `min-height` ancestor to `auto` and the nav was
        407px tall and scrolled away with the page.
      */}
      <div
        className="contents xl:sticky xl:top-0 xl:block xl:h-screen"
        onClick={routeRailClick}
      >
        <SideNav
          topItem={OVERVIEW_NAV}
          groups={NAV_GROUPS.map((group) => ({
            label: group.label,
            items: [...group.items],
          }))}
          footerItems={[]}
          currentPath={pathname}
          toolbarVariant="vibrant"
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
        The rails float over this column and cannot reserve their own space,
        so the padding is this file's: `pb-24` clears the phone bar (64px
        tall, 16px off the bottom, plus 16px to spare), `sm:pl-24` the
        vertical one (16 + 64 + a 16px gutter), and `xl:pl-0` gives the
        gutter back where the sidebar is in flow. This app sets no
        `viewport-fit=cover`, so the bar's `env(safe-area-inset-bottom)` is
        0 and needs no allowance here.
      */}
      <main className="min-w-0 flex-1 pb-24 sm:pb-0 sm:pl-24 xl:pl-0">
        <TopAppBar email={email} merchantId={merchantId} signOut={signOut} />
        <div className="mx-auto flex w-full max-w-7xl flex-col gap-6 p-4 sm:p-6">
          {children}
        </div>
      </main>
    </div>
  );
}

function TopAppBar({
  email,
  merchantId,
  signOut,
}: Omit<AppShellProps, "children">) {
  return (
    <div className="sticky top-0 z-20 bg-base-100">
      <div className="flex h-16 items-center gap-3 px-4 sm:px-6">
        <Brand />
        <span className="min-w-0 truncate" title={`Merchant ${merchantId}`}>
          <Badge variant="outline">
            <span className="sr-only">Merchant </span>
            {merchantId}
          </Badge>
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-2">
          <span className="hidden sm:block xl:hidden">
            <SignedInBar
              email={email}
              merchantId={merchantId}
              signOut={signOut}
              layout="inline"
            />
          </span>
          <ThemeToggle />
        </span>
      </div>
    </div>
  );
}

/**
 * What `SideNav`'s `accountSlot` holds: who is signed in with the way out,
 * then the three-way theme control (System, Light, Dark). The caption is
 * for the eye; the radio group keeps its own accessible name ("Theme").
 * Both copies read the theme from `ThemeSwitcher`'s module-level store, so
 * the sheet's copy mounting fresh on every open does not reset it.
 */
function AccountBlock({
  email,
  merchantId,
  signOut,
}: Omit<AppShellProps, "children">) {
  return (
    <div className="flex flex-col gap-4">
      <SignedInBar email={email} merchantId={merchantId} signOut={signOut} />
      <div className="flex flex-col gap-2">
        <span className="text-label-medium text-muted-foreground">Theme</span>
        <ThemeSwitcher />
      </div>
    </div>
  );
}
