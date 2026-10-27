/**
 * Procedure paging: the page size, the query-string readers and the shape of
 * one page. Pure, so the pager can be rendered by client components and
 * stories alike.
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

/**
 * One page of a procedure-backed list, as the screen needs it.
 *
 * `list` is what `@cratestack/refine`'s `getList` resolves, so it is what the
 * client screen's `useList` cache holds for the key. `totalCount` and
 * `hasNextPage` come from the procedure's own `Page`, because refine's `total`
 * cannot say "unknown": past the end of the set a procedure answers
 * `totalCount: null` and the provider reports `0`. The pager reads these.
 */
export interface ProcedureListPage<Row> {
  readonly list: { readonly data: Row[]; readonly total: number };
  readonly totalCount: number | null;
  readonly hasNextPage: boolean;
  /** The offset actually read — the requested one, floored to a page. */
  readonly offset: number;
}
