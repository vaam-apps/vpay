import { ScreenStack, Skeleton } from "@vaam-apps/ui";

import { PaymentsFiltersSkeleton } from "./payments-filters";

/**
 * The line under the payments screen's heading. One string, used by the
 * loaded screen and by the placeholder below, so the placeholder wraps
 * where the text will.
 */
export const PAYMENTS_DESCRIPTION =
  "Every payment intent this merchant has created, newest first.";

/**
 * The payments screen while its first read is in flight, shaped so that
 * the heading, the description and the filter row land where their
 * placeholders were.
 *
 * **Why not `RouteSkeleton rows={8} withFilterBar` (2026-09-24, the
 * maintainer's call on vaam-apps/vpay#258's review).** Measured in the
 * built Storybook, headless Chromium, inside `AppShell`, at 320, 375, 640,
 * 700, 726 and 1280 px windows: its header is one 48 px block where this
 * screen has a 24 px `<h2>`, the stack's 24 px gap and a description of
 * one line, or two on a phone, so its filter bar sat 20 px above the real
 * row from 640 px up and 40 px above it at 320 and 375 px. The bar itself
 * was 30 to 116 px short (`PaymentsFiltersSkeleton` has the widths), so the
 * table started 50 px lower than the skeleton's rows at 1280 px and 156 px
 * lower at 375 px; at a 20 px root, 52.5 and 120 px. Its 320 px
 * description block also pushed the document sideways: 344 px wide in a
 * 320 px window, and 430 px in 320 and 375 px windows at a 20 px root.
 * With this component every box of the filter row, and the top of what
 * follows it, measured the same as the loaded screen's at all twelve of
 * those widths and roots, and nothing scrolled sideways.
 *
 * The heading's placeholder is `h-lh`: one line of the inherited body
 * text, which is what the unstyled `<h2>` is (24 px at a 16 px root, 30 px
 * at 20). The description's is sized by the description itself, invisible,
 * in its own `text-body`, so it is one line or two exactly when the text
 * is. No `<h2>` is rendered here: `dashboard.cy.ts` waits on
 * `cy.contains("h2", "Payments")` as the sign the list has loaded.
 *
 * The table rows below are `RouteSkeleton`'s own eight 44 px blocks. How
 * many rows a read returns is not known until it returns, so nothing below
 * the filter row is claimed to line up.
 *
 * Held by the `PaymentsLoading` and `PaymentsLoadingPhone` stories, which
 * compare it box by box with the loaded heading, description and filter
 * row at 1280 and 375 px, squeezed to the shell's columns, and at a 20 px
 * root. Each of these fails them on its own: the skeleton row without
 * `flex-wrap` (one line where the row has two); its label placeholders at
 * `h-5` rather than `h-lh` (only at the 20 px root, 5 px); the date
 * placeholder at `w-64` (9.9 px); the description's placeholder a fixed
 * `h-5 w-80` (20 px on a phone, where the text wraps); the select's copy
 * replaced by a fixed `w-52` (a copy short, and 11 px off); Apply's copy without `invisible`; and
 * the real row's `gap-3` changed to `gap-2` with the skeleton left alone
 * (8 px). `screens.test.tsx` fails if the screen's loading branch renders
 * anything without this filter row.
 */
export function PaymentsSkeleton() {
  return (
    <ScreenStack role="status" aria-busy="true">
      <span className="sr-only">Loading…</span>
      <Skeleton className="h-lh w-24" />
      <Skeleton className="w-fit text-body">
        <span className="invisible">{PAYMENTS_DESCRIPTION}</span>
      </Skeleton>
      <PaymentsFiltersSkeleton />
      <div className="flex flex-col gap-2">
        {Array.from({ length: 8 }, (_, row) => (
          <Skeleton key={row} className="h-11 w-full" />
        ))}
      </div>
    </ScreenStack>
  );
}
