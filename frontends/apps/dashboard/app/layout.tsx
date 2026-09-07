import type { Metadata } from 'next';

import { Heading, PageShell } from '@vpay/ui';

import './globals.css';

export const metadata: Metadata = {
  title: 'vpay dashboard',
  description: 'Observability for vpay. Administration is YAML.',
};

/**
 * The dashboard's persistent chrome: a brand heading plus the one-column
 * page frame every screen sits in.
 *
 * The brand is the app's only `<h1>`, rendered here rather than per-page,
 * because every screen carries it. There is exactly one page today (`/`);
 * `docs/flows/dashboard.md`'s own rule is "the navigation is only ever
 * allowed to link to slices that exist" — a menu entry for a page nobody
 * wrote is the same lie as an empty table — so this `<nav>` carries no
 * links yet. Add one the day the page it points at exists (exp24's
 * sign-in, or slice 1's payments list), not before — `src/layout.test.tsx`
 * fails if this `<nav>` links anywhere `app/` has no page for.
 *
 * `<main>` is not decoration. The scaffold this pass replaced wrapped its
 * whole page in one; the first draft of this composition dropped it, and
 * axe-core's `region` rule went from 0 violations to 1 ("All page content
 * should be contained by landmarks") — measured in
 * `docs/plans/exp26-notes/lane-d-review.md`, finding 1. `src/a11y.test.tsx`
 * is the gate that noticed, and stays.
 */
export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" data-theme="bumblebee">
      <body>
        <nav>
          <PageShell>
            <Heading level={1}>vpay dashboard</Heading>
          </PageShell>
        </nav>
        <main>
          <PageShell>{children}</PageShell>
        </main>
      </body>
    </html>
  );
}
