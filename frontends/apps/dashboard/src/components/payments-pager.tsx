import NextLink from "next/link";

export interface PaymentsPagerProps {
  /** Href for the previous page, or `null` on the first one. */
  previousHref: string | null;
  /** Href for the next page, or `null` when `has_more` was false. */
  nextHref: string | null;
}

/**
 * `pagerHrefs`' two nullable hrefs, rendered as `next/link` anchors.
 *
 * Imports nothing from `@vaam-apps/ui`: that package's `Pagination` renders
 * `<button>` elements driven by `onPrevious`/`onNext` callbacks — a
 * client-side component with no notion of an href — and turns an absent
 * control into a **disabled button** rather than omitting it. Two things
 * make that decisive rather than a matter of taste:
 *
 * 1. `frontends/tests/e2e/cypress/e2e/dashboard.cy.ts` looks for
 *    `cy.contains("a", /next/i)` and `cy.contains("a", /previous|back/i)`.
 *    A `<button>` would stop being found by either.
 * 2. The decision this file has always encoded: *"An absent control is a
 *    sentence, never a disabled-looking one that does nothing."* A
 *    `Pagination` that renders `Previous`/`Next` as permanently-disabled
 *    buttons on the first/last page reverses that.
 *
 * `next/link` keeps prefetch, `rel="prev"`/`rel="next"`, middle-click, and
 * the a11y test's `link-name` expectations — none of which a callback-driven
 * `<button>` would carry.
 */
export function PaymentsPager({ previousHref, nextHref }: PaymentsPagerProps) {
  if (previousHref === null && nextHref === null) {
    return null;
  }

  return (
    <nav aria-label="Payments paging">
      {previousHref === null ? (
        <span>Newest first</span>
      ) : (
        <NextLink href={previousHref} rel="prev">
          Previous
        </NextLink>
      )}
      {nextHref === null ? (
        <span>End of results</span>
      ) : (
        <NextLink href={nextHref} rel="next">
          Next
        </NextLink>
      )}
    </nav>
  );
}
