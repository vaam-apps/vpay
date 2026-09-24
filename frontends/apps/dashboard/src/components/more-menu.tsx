"use client";

import {
  Button,
  DrawerClose,
  MoreDetailDrawer,
  ThemeSwitcher,
} from "@vaam-apps/ui";
import { Menu } from "lucide-react";
import Link from "next/link";
import { useState } from "react";

import { NAV_ENTRIES, isNavActive } from "../dash/resources";
import { SignedInBar } from "./signed-in-bar";

export interface MoreMenuProps {
  readonly email: string;
  readonly merchantId: string;
  readonly signOut: () => Promise<void>;
  /** Same value `AppShell` reads from `usePathname()` — used only for
   * `aria-current`, never for routing: every href here still comes from
   * `NAV_ENTRIES`. */
  readonly currentPath: string;
  /**
   * Render the trigger as the icon alone, with `aria-label="Menu"`.
   *
   * Used where the trigger floats beside `SideNav`'s own rails rather than
   * sitting in the content column. It has to be narrow there: below `xl`
   * `<main>` carries `sm:pl-24`, a 96px gutter the vertical rail occupies,
   * and a labelled button is wider than that — it would sit on top of the
   * first column of the screen it is meant to navigate away from. The icon
   * alone is 48px, which is both inside the gutter and the touch target
   * the rail's own items use (M3's 48dp, since `@vaam-apps/ui` 0.3.0).
   *
   * The accessible name is unchanged, so every test and every screen reader
   * still finds one control named "Menu".
   */
  readonly compact?: boolean;
}

/**
 * The "Menu" drawer: everything the rail and the sidebar's `accountSlot`
 *
 * **Named "Menu" and not "More", and the rename is load-bearing.** `SideNav`
 * renders its OWN "More" overflow button below 640px once the rail has more
 * destinations than fit — four plus an overflow, per its own documentation.
 * The dashboard crossed that threshold when Lane E grew the nav from one
 * entry to four, and two different controls both called "More" appeared in
 * the same pill: one opening the rail's overflow list, one opening this
 * drawer. `more-menu.test.tsx` and `a11y.test.tsx` found it as "found
 * multiple elements with the role button and name /more/i", which is the
 * accessibility tree describing the same collision a person would hit.
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
 * # A bottom sheet on a phone, a right-hand panel from `md` up
 *
 * That split is what the maintainer asked for, and `MoreDetailDrawer`
 * already **is** it — `drawer.js`'s `DetailDrawerContent` opens
 * `direction="bottom"` and carries `inset-x-0 bottom-0 rounded-t-box
 * border-t` for the phone, then resets every one of those explicitly at
 * `md:` (`md:inset-y-0 md:right-0 md:h-full md:rounded-l-box
 * md:border-l`). None of that is written here; the component owns it.
 *
 * An earlier revision of this file used the generic `Drawer` /
 * `DrawerContent` composition instead, pinned to `direction="right"`, and
 * its comment said a bottom sheet was unreachable because `verify-ui`
 * caps an app's `className` at 60 characters (justfile check 7a-iii) and
 * the responsive rules do not fit. **That reasoning had the right
 * constraint and the wrong conclusion.** The cap is real — and so is
 * check 7a-i, which is stricter still and forbids a computed `className`
 * in an app at all, so `cn()` is not a way round it either. But neither
 * bites here, because reaching the split never required writing those
 * classes at the call site: it required calling the component that
 * already ships them. The longest `className` in this file is 47
 * characters, and the drawer's placement is in none of them.
 *
 * The cost of that swap, stated: `MoreDetailDrawer` is controlled, so the
 * open state is `useState` here and the trigger is our own `<Button>`
 * rather than a `DrawerTrigger`. Its `max-w-[680px]` at `md`+ is wider
 * than the generic panel's `560px`.
 *
 * # The current entry is marked for a screen reader and not for an eye
 *
 * `aria-current="page"` is the whole signal; there is no visual active
 * style on the drawer's rows, and that IS a gap rather than a preference.
 * Ternary-in-a-class-attribute is the shape check 7a-i bans by name, so
 * that route is closed. The static form that would survive it — a Tailwind
 * `aria-[current=page]:` variant — does not fit check 7a-iii's 60-
 * character budget beside the row's layout: measured, `flex items-center
 * gap-2 rounded-field px-3 py-2 aria-[current=page]:font-medium` is 79
 * characters, and 65 even with `rounded-field` dropped. The rail carries
 * the visible current-page state at every width this drawer opens at, so
 * what is missing here is a second rendering of it, not the only one.
 *
 * (That comment itself tripped check 7a-i on its first draft: the grep is
 * over the file's TEXT, prose included, so quoting the banned shape
 * literally fails the gate. Left as a note for the next person to describe
 * it in words rather than in code.)
 *
 * # The nav tree is `NAV_ENTRIES`, not a second list
 *
 * Rendered from the exact same array `AppShell` passes to `SideNav`'s
 * `groups` — `resources.ts`'s whole point is that there is one place a nav
 * destination is declared. Growing the rail later (another lane) is a data
 * change to that file and nothing here. `aria-current` is decided by
 * `isNavActive`, that file's one copy of `SideNav`'s own rule, so the rail
 * and this drawer cannot disagree about which entry is current — they did,
 * on every detail route, until that function existed.
 *
 * # The account block is a SECOND `SignedInBar`, and it is safe
 *
 * `dashboard.cy.ts`'s `cy.contains(staffEmail()).should("be.visible")`
 * runs at Cypress's default 1000×660 viewport, with eight tests chained
 * after it in a `testIsolation: false` sequence — so the identity cannot
 * live only behind this drawer's trigger. `AppShell` keeps its own
 * always-visible `SignedInBar` in `<main>` for that reason, and this is a
 * second mount of the same component.
 *
 * Two mounts of a sign-out form sounds like a duplication bug. Measured on
 * a real render rather than argued: **only one is ever in the
 * accessibility tree.** `MoreDetailDrawer` is the dimmed, modal variant
 * (`dimmed: true` → vaul's `modal`), and Radix marks the rest of the
 * document `aria-hidden` while it is open — so
 * `getAllByRole("button", { name: /sign out/i })` returns exactly **one**
 * element with the drawer shut and exactly **one** with it open, the
 * `<main>` copy having left the tree as the drawer's copy entered it.
 * `more-menu.test.tsx` pins that count in both states. axe over the open
 * drawer reports no `duplicate-id`, `duplicate-id-aria` or `region`
 * violation either (`a11y.test.tsx`'s open-drawer case), and neither
 * `SignedInBar` renders an `id` at all.
 */
export function MoreMenu({
  email,
  merchantId,
  signOut,
  currentPath,
  compact = false,
}: MoreMenuProps) {
  const [open, setOpen] = useState(false);

  return (
    <>
      {compact ? (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Menu"
          title="Menu"
          /*
            `h-12 w-12` is 48px, and it is read off the rail rather than
            chosen: `@vaam-apps/ui` 0.3.0's floating toolbar renders every
            item as a 48px target inside 8px of padding (`side-nav.tsx`'s
            `toolbarItemClasses` and `TOOLBAR_CONTAINER`), giving a 64px
            rail, and `app-shell.tsx` wraps this trigger in the same `p-2`,
            so matching the item size matches the rail. Icons there render
            at 24, so this one does too.

            It was `h-11 w-11` with a 20px icon until the 0.3.0 bump, for
            the same reason against 0.2.x's rail: 44x44 rows in 6px of
            padding, a 58px pill. Before that it was 42x32 in a 46px pill,
            and the two sat side by side at visibly different heights.

            48px is also M3's touch-target floor, so this is not only a
            visual match.
          */
          className="h-12 w-12"
          onClick={() => setOpen(true)}
        >
          <Menu size={24} aria-hidden="true" />
        </Button>
      ) : (
        <Button
          type="button"
          variant="secondary"
          size="sm"
          onClick={() => setOpen(true)}
        >
          <Menu size={16} aria-hidden="true" />
          Menu
        </Button>
      )}
      <MoreDetailDrawer
        open={open}
        onOpenChange={setOpen}
        title="Menu"
        description="Navigation, theme, and your account."
      >
        <div className="flex flex-col gap-6">
          <nav aria-label="Menu destinations">
            <ul className="flex flex-col gap-1">
              {NAV_ENTRIES.map((entry) => {
                const Icon = entry.icon;
                const active = isNavActive(entry.href, currentPath);
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
          <SignedInBar
            email={email}
            merchantId={merchantId}
            signOut={signOut}
          />
        </div>
      </MoreDetailDrawer>
    </>
  );
}
