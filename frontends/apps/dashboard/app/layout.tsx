import type { Metadata } from "next";

import { ScreenStack } from "@vaam-apps/ui";

import { PrimaryNav } from "../src/nav";

import "./globals.css";

export const metadata: Metadata = {
  title: "vpay dashboard",
  description: "Observability for vpay. Administration is YAML.",
};

/**
 * The dashboard's persistent chrome: the brand, the nav, and the one-column
 * page frame every screen sits in.
 *
 * `@vpay/ui` was deleted on 2026-09-12; `ScreenStack` from `@vaam-apps/ui`
 * takes over the page frame. It supplies only the vertical rhythm
 * (`flex flex-col gap-6`) — no centring, no `max-w`, no padding — so this is
 * the one place in the app that carries them, in a plain literal `className`
 * the gate's check 7a-i and 7a-iii both permit: a centred column with a
 * generous width for a payments table, not the checkout's narrow
 * `max-w-md`, which this app never had a reason to inherit.
 *
 * The nav's links live in `src/nav.tsx` as an exported constant, and
 * `src/layout.test.tsx` resolves every one of them against `app/**\/page.tsx`
 * on disk — the rule that a menu entry for a page nobody wrote is the same
 * lie as an empty table, as a gate rather than as a comment. It was stated in
 * three places and enforced in none until that test
 * (`docs/plans/exp26-notes/lane-d-review.md`, finding 2).
 *
 * `<main>` is not decoration. An earlier draft of this composition dropped it
 * and axe-core's `region` rule went from 0 violations to 1 ("All page content
 * should be contained by landmarks"), with every other gate green.
 * `src/a11y.test.tsx` is what noticed, and stays.
 *
 * Nothing here reads the session. The layout renders on `/login` too, so a
 * control that only makes sense signed in — "Sign out", the staff member's
 * address — belongs to the pages behind the gate (`SignedInBar`) and not to
 * chrome shared with a page where nobody is signed in.
 */
export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" data-theme="dark">
      <body>
        <PrimaryNav />
        <main>
          <ScreenStack className="mx-auto w-full max-w-5xl p-6">
            {children}
          </ScreenStack>
        </main>
      </body>
    </html>
  );
}
