/**
 * The pure half of procedure paging: the page size and the query-string
 * readers. Split from `procedure-list.ts` (2026-10-21) because that module
 * reaches the server-only token code through `callDashProcedure`, and the
 * pager is rendered by client components and stories too.
 */

/** How many rows a procedure-backed screen asks for. */
export const PAGE_SIZE = 20;

/** `?offset=` as a non-negative integer, or 0. */
export function offsetFrom(
  params: Record<string, string | string[] | undefined>,
): number {
  const raw = params["offset"];
  const parsed = Number(Array.isArray(raw) ? raw[0] : raw);
  return Number.isInteger(parsed) && parsed >= 0 ? parsed : 0;
}

/**
 * `?<name>=` as one string, or `''` when absent or repeated.
 *
 * A repeated parameter is not a filter this app ever writes, so it is read
 * as no filter rather than as whichever copy came first.
 */
export function singleParam(
  params: Record<string, string | string[] | undefined>,
  name: string,
): string {
  const raw = params[name];
  return typeof raw === "string" ? raw : "";
}
