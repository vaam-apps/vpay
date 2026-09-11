import NextLink from "next/link";

import { Pagination } from "@vpay/ui";

export interface PaymentsPagerProps {
  /** Href for the previous page, or `null` on the first one. */
  previousHref: string | null;
  /** Href for the next page, or `null` when `has_more` was false. */
  nextHref: string | null;
}

/**
 * `pagerHrefs`' two nullable hrefs, as the two elements `Pagination` renders.
 *
 * The router is this app's, not `@vpay/ui`'s: the primitive owns the `<nav>`,
 * the naming and the "there is nothing that way" copy, and the caller
 * supplies whatever navigates. `next/link` prefetches the next page of
 * payments; a bare `<a>` would not.
 */
export function PaymentsPager({ previousHref, nextHref }: PaymentsPagerProps) {
  return (
    <Pagination
      label="Payments paging"
      previous={
        previousHref === null ? null : (
          <NextLink href={previousHref} rel="prev">
            Previous
          </NextLink>
        )
      }
      next={
        nextHref === null ? null : (
          <NextLink href={nextHref} rel="next">
            Next
          </NextLink>
        )
      }
    />
  );
}
