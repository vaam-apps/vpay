import { ChevronLeft, ChevronRight } from "lucide-react";

import { PAGE_SIZE } from "../dash/procedure-paging";
import { OutlinedLink } from "./layout";

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
  /**
   * The rest of the query to keep while paging — a filter, e.g.
   * `status=failed`, already encoded. Empty for an unfiltered list.
   */
  readonly query?: string;
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
 *
 * # Drawn as the card's footer (2026-10-21)
 *
 * The range on the left, the two controls on the right as M3 outlined
 * buttons that are still `<a rel>` links. An absent control is still
 * omitted rather than drawn disabled — "an absent control is a sentence,
 * never a disabled-looking one that does nothing" (`payments-pager.tsx`).
 * A filter in the URL is carried on both links: paging a filtered list must
 * not quietly drop the filter.
 */
export function ProcedurePager({
  basePath,
  offset,
  returned,
  totalCount,
  hasNext,
  query = "",
}: ProcedurePagerProps) {
  const previousOffset = Math.max(0, offset - PAGE_SIZE);
  const hasPrevious = offset > 0;
  const first = offset + 1;
  const last = offset + returned;
  const href = (at: number) => {
    const search = new URLSearchParams(query);
    if (at > 0) {
      search.set("offset", String(at));
    }
    const rest = search.toString();
    return rest.length > 0 ? `${basePath}?${rest}` : basePath;
  };

  return (
    <nav aria-label="Pagination" className="flex flex-wrap items-center gap-3">
      <span className="flex-1 text-body-medium text-muted-foreground">
        <span className="font-mono tabular-nums">
          {first}–{last}
          {totalCount === null ? "" : ` of ${totalCount}`}
        </span>{" "}
        so far
      </span>
      {hasPrevious ? (
        <OutlinedLink href={href(previousOffset)} rel="prev">
          <ChevronLeft size={18} aria-hidden="true" />
          Previous
        </OutlinedLink>
      ) : null}
      {hasNext ? (
        <OutlinedLink href={href(offset + PAGE_SIZE)} rel="next">
          Next
          <ChevronRight size={18} aria-hidden="true" />
        </OutlinedLink>
      ) : null}
    </nav>
  );
}
