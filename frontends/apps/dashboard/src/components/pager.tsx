import Link from "next/link";

import { Stack, Text } from "@vpay/ui";

export interface PagerProps {
  /** Href for the previous page, or `null` on the first one. */
  previousHref: string | null;
  /** Href for the next page, or `null` when `has_more` was false. */
  nextHref: string | null;
}

/**
 * Cursor paging, as two links.
 *
 * Cursors and not page numbers, because that is what `/dash/v1` serves:
 * `starting_after` and `ending_before` name a row, `has_more` says whether
 * another page exists, and there is no count anywhere — the same cursors,
 * default and ceiling as `/v1`, shared rather than re-derived so that an
 * operator and a merchant reading the same rows cannot disagree about
 * whether there are more (`vpay_api::v1::paging`).
 *
 * So there is no "page 3 of 12" here, and inventing one would mean either a
 * `COUNT(*)` this API does not offer or a number this app made up.
 *
 * Both links are `null` when there is nothing to go to; a disabled-looking
 * control that does nothing is worse than no control.
 */
export function Pager({ previousHref, nextHref }: PagerProps) {
  if (previousHref === null && nextHref === null) {
    return null;
  }
  return (
    <Stack as="nav" justify="between" gap="md" aria-label="Payments paging">
      {previousHref === null ? (
        <Text as="span" tone="muted" size="sm">
          Newest first
        </Text>
      ) : (
        <Link href={previousHref} rel="prev">
          Previous
        </Link>
      )}
      {nextHref === null ? (
        <Text as="span" tone="muted" size="sm">
          End of results
        </Text>
      ) : (
        <Link href={nextHref} rel="next">
          Next
        </Link>
      )}
    </Stack>
  );
}
