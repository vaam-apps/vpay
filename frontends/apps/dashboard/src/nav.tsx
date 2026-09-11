import NextLink from "next/link";

import { Heading, Link, PageShell, Stack } from "@vpay/ui";

/**
 * Every internal link the persistent chrome renders, in one array.
 *
 * > "The navigation is only ever allowed to link to slices that exist. A menu
 * > entry for a page nobody wrote is the same lie as an empty table."
 * > — `docs/flows/dashboard.md`
 *
 * A **constant** rather than JSX the test has to scrape, because the gate has
 * to hold for a link that is rendered conditionally as well as for one that
 * is always there: `layout.test.tsx` resolves each `href` here against
 * `app/**\/page.tsx` on disk, and a link added to a branch a test never
 * rendered would otherwise slip past. It still checks the rendered markup
 * too — that half catches a link written straight into the JSX instead of
 * added here.
 *
 * One entry today. `/login` is deliberately not in it: it is where an
 * unauthenticated visitor is *sent*, not somewhere anyone navigates to, and a
 * "Sign in" tab beside "Payments" on a signed-in page would be a control that
 * does nothing. Signing out is on the pages behind the session
 * (`SignedInBar`), for the same reason.
 */
export const NAV_LINKS: readonly {
  readonly href: string;
  readonly label: string;
}[] = [{ href: "/payments", label: "Payments" }];

/**
 * The brand and the nav, as one `<nav>` landmark.
 *
 * The brand is the app's only `<h1>` and is rendered here rather than
 * per-page, because every screen carries it.
 */
export function PrimaryNav() {
  return (
    <nav>
      <PageShell>
        <Stack justify="between" align="center" gap="md" wrap>
          <Heading level={1}>vpay dashboard</Heading>
          <Stack gap="md" as="div">
            {NAV_LINKS.map((entry) => (
              <Link
                key={entry.href}
                render={<NextLink href={entry.href} />}
                tone="hover"
              >
                {entry.label}
              </Link>
            ))}
          </Stack>
        </Stack>
      </PageShell>
    </nav>
  );
}
