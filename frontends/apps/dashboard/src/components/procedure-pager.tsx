import NextLink from "next/link";

import { PAGE_SIZE } from "../dash/procedure-list";

export interface ProcedurePagerProps {
  /** The screen's own route, e.g. `/refunds`. */
  readonly basePath: string;
  /** The offset this page was read at. */
  readonly offset: number;
  /** How many rows came back — never how many were asked for. */
  readonly returned: number;
  /** `totalCount`, exact for the statement that returned it, or null. */
  readonly totalCount: number | null;
  /** `pageInfo.hasNextPage` — the server's own answer, not a guess. */
  readonly hasNext: boolean;
}

/**
 * Previous/next for an offset-paged procedure list.
 *
 * # Why this is not `payments-pager.tsx`
 *
 * That one is **cursor**-paged: it carries `starting_after`/`ending_before`
 * because `GET /dash/v1/payment_intents` shares `crate::v1::paging` with the
 * merchant API, so an insert cannot shift its window. A procedure is
 * offset-paged, and `OFFSET` is counted afresh on every call — a row created
 * between two pages shifts the window by one, so a reader walking forward
 * can see one row twice and miss another. `search_payment_intents.rs` states
 * that property at length and this pager does not pretend otherwise.
 *
 * What follows from it: **`total_count` is described as "so far", not as a
 * total.** It is exact for the statement that produced it and says nothing
 * about the next one.
 *
 * # "Next" comes from `pageInfo.hasNextPage`, not from arithmetic
 *
 * The server answers it, so this does not re-derive it from `offset +
 * PAGE_SIZE < totalCount` — that comparison offers a next page that answers
 * nothing whenever rows were deleted between the count and the read.
 *
 * `totalCount` is `Option<i64>` upstream and is rendered only when present:
 * a page that carries no count says "1–20 so far" rather than "1–20 of
 * undefined", which is what this rendered before the camelCase mismatch in
 * `dash-read.ts` was found.
 */
export function ProcedurePager({
  basePath,
  offset,
  returned,
  totalCount,
  hasNext,
}: ProcedurePagerProps) {
  const previousOffset = Math.max(0, offset - PAGE_SIZE);
  const hasPrevious = offset > 0;
  const first = offset + 1;
  const last = offset + returned;

  return (
    <nav aria-label="Pagination" className="flex items-center gap-4">
      <span className="text-caption text-muted-foreground">
        {first}–{last}
        {totalCount === null ? "" : ` of ${totalCount}`} so far
      </span>
      {hasPrevious ? (
        <NextLink href={`${basePath}?offset=${previousOffset}`} rel="prev">
          Previous
        </NextLink>
      ) : null}
      {hasNext ? (
        <NextLink href={`${basePath}?offset=${offset + PAGE_SIZE}`} rel="next">
          Next
        </NextLink>
      ) : null}
    </nav>
  );
}
