"use client";

import {
  Button,
  Drawer,
  DrawerClose,
  DrawerContent,
  DrawerDescription,
  DrawerTitle,
  DrawerTrigger,
  ThemeSwitcher,
} from "@vaam-apps/ui";
import { Menu } from "lucide-react";
import Link from "next/link";

import { NAV_ENTRIES } from "../dash/resources";
import { SignedInBar } from "./signed-in-bar";

export interface MoreMenuProps {
  readonly email: string;
  readonly merchantId: string;
  readonly signOut: () => Promise<void>;
  /** Same value `AppShell` reads from `usePathname()` — used only for
   * `aria-current`, never for routing: every href here still comes from
   * `NAV_ENTRIES`. */
  readonly currentPath: string;
}

/**
 * The "More" drawer: everything the rail and the sidebar's `accountSlot`
 * cannot reach below the full ≥1280px sidebar.
 *
 * # Why this exists at all
 *
 * `SideNav` only renders `accountSlot` in the in-flow sidebar — never in
 * the 640–1279px floating rail or the sub-640px bottom pill (its own doc:
 * "not rendered below `lg`... `~52px` has no room for an email address").
 * Below the full sidebar there was, until this file, no theme control on
 * the page at all. This drawer is the one thing reachable at every width,
 * holding the three things that don't fit in the rail: the nav tree, the
 * theme switcher, and the account block.
 *
 * # The nav tree is `NAV_ENTRIES`, not a second list
 *
 * Rendered from the exact same array `AppShell` passes to `SideNav`'s
 * `groups` — `resources.ts`'s whole point is that there is one place a nav
 * destination is declared. Growing the rail later (another lane) is a data
 * change to that file and nothing here.
 *
 * # The account block is a SECOND `SignedInBar`, deliberately
 *
 * `dashboard.cy.ts`'s `cy.contains(staffEmail()).should("be.visible")` runs
 * at Cypress's default 1000×660 viewport, with eight tests chained after it
 * in a `testIsolation: false` sequence — so the identity cannot live only
 * behind this drawer's trigger. `AppShell` keeps its own always-visible
 * `SignedInBar` in `<main>` for that reason and this is a second mount of
 * the same component, not a competing account UI: one source of what the
 * block looks like, rendered in the one place that must never require a
 * click to reach and again in here for anyone who opened the drawer for
 * the nav tree or the theme switcher and wants sign-out right there too.
 *
 * # A right-hand panel, not a bottom sheet
 *
 * The generic `Drawer` composition's `DrawerContent` is CSS-positioned at
 * the right edge by default (`inset-y-0 right-0`), and `direction="right"`
 * is what the skill's own reference names as the correct pairing for that
 * placement — leaving it unset drags on the wrong axis. Reproducing the
 * bottom-sheet-on-phone/right-panel-on-desktop split `MoreDetailDrawer`
 * uses internally would need overriding most of its layout classes at the
 * call site, and `just lint-web`'s `verify-ui` caps a `className` in this
 * app at 60 characters — there is no literal string of the needed rules
 * that fits. The default panel is already full-width below its `max-w`
 * breakpoint (`w-full max-w-[560px]`), so on a phone it covers the screen
 * edge-to-edge exactly as a sheet would; it only differs in which edge it
 * slides from.
 */
export function MoreMenu({
  email,
  merchantId,
  signOut,
  currentPath,
}: MoreMenuProps) {
  return (
    <Drawer direction="right">
      <DrawerTrigger asChild>
        <Button type="button" variant="secondary" size="sm">
          <Menu size={16} aria-hidden="true" />
          More
        </Button>
      </DrawerTrigger>
      <DrawerContent className="flex flex-col gap-6 p-5">
        <div>
          <DrawerTitle>More</DrawerTitle>
          <DrawerDescription>
            Navigation, theme, and your account.
          </DrawerDescription>
        </div>
        <nav aria-label="More destinations">
          <ul className="flex flex-col gap-1">
            {NAV_ENTRIES.map((entry) => {
              const Icon = entry.icon;
              const active = entry.href === currentPath;
              return (
                <li key={entry.href}>
                  <DrawerClose asChild>
                    <Link
                      href={entry.href}
                      aria-current={active ? "page" : undefined}
                      className="flex items-center gap-2 rounded-field px-3 py-2"
                    >
                      <Icon size={16} aria-hidden="true" />
                      {entry.label}
                    </Link>
                  </DrawerClose>
                </li>
              );
            })}
          </ul>
        </nav>
        <div className="flex flex-col gap-2">
          <span className="text-caption text-muted-foreground">Theme</span>
          <ThemeSwitcher />
        </div>
        <SignedInBar email={email} merchantId={merchantId} signOut={signOut} />
      </DrawerContent>
    </Drawer>
  );
}
