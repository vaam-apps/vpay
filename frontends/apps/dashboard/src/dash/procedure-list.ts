import { createCratestackDataProvider } from "@cratestack/refine";
import type { CrudFilter } from "@refinedev/core";

import type { StaffContext } from "../server/session";
import { listProcedurePage } from "./cratestack-client";
import { failureFromError } from "./failure";
import type { Initial } from "./initial";
import { PAGE_SIZE } from "./procedure-paging";
import { procedureResources, ProcedureRefusal } from "./procedure-resources";
import type { ProcedureResource } from "./resource-name";

export { offsetFrom, PAGE_SIZE, singleParam } from "./procedure-paging";

/**
 * One page of a procedure-backed list, as the screen needs it.
 *
 * `list` is exactly what `@cratestack/refine`'s `getList` resolved, so it is
 * what the client screen's `useList` cache holds for this key — a second
 * shape here would be a first render that disagreed with every render after
 * it (the rule `app/(dash)/payments/page.tsx` documents). `totalCount` and
 * `hasNextPage` come from the procedure's own `Page`, because refine's
 * `total` cannot say "unknown": past the end of the set a procedure answers
 * `totalCount: null` and the provider reports `0`. The pager reads these.
 */
export interface ProcedureListPage<Row> {
  readonly list: { readonly data: Row[]; readonly total: number };
  readonly totalCount: number | null;
  readonly hasNextPage: boolean;
  /** The offset actually read — the requested one, floored to a page. */
  readonly offset: number;
}

/**
 * Read one page of a procedure-backed resource, server side, **through
 * `@cratestack/refine`** over the generated client.
 *
 * The screens that use this are Server Components, so the read happens where
 * the token is — the same property `app/(dash)/payments/page.tsx` documents
 * at length. `callProcedure` carries the 80 %-of-TTL re-mint's fallback, the
 * single-`401` retry; nothing here reimplements it.
 *
 * Returns the refusal rather than an empty page when the read fails. An
 * empty list and a refused read look identical on screen unless the screen
 * is told which it got, and rendering a refusal as "no rows" is the failure
 * `CLAUDE.md` names.
 *
 * `filter` is the procedure's own named filter fields (`status` for refunds
 * and checkouts, `state` for deliveries), each sent as refine's `eq`. A value
 * the procedure does not know is refused by vpay with a `400` naming the
 * field, and that refusal is what the screen renders.
 *
 * **The offset is floored to a page boundary.** `@cratestack/refine` speaks
 * refine's `currentPage`, not a raw offset, so a hand-edited `?offset=7`
 * reads the page that starts at `0`, and {@link ProcedureListPage.offset}
 * says so. Every link this app writes is already on a boundary.
 */
export async function readProcedureList<Row>(
  staff: StaffContext,
  resource: ProcedureResource,
  offset: number,
  filter: Readonly<Record<string, string>> = {},
  limit: number = PAGE_SIZE,
): Promise<Initial<ProcedureListPage<Row>>> {
  let page:
    | { readonly totalCount: number | null; readonly hasNextPage: boolean }
    | undefined;
  const provider = createCratestackDataProvider(
    procedureResources(async (r, input, f) => {
      const result = await listProcedurePage(staff, r, input, f);
      if (!result.ok) {
        throw new ProcedureRefusal(result.failure);
      }
      page = {
        totalCount: result.value.totalCount,
        hasNextPage: result.value.pageInfo.hasNextPage,
      };
      return result.value;
    }),
  );

  const currentPage = Math.floor(offset / limit) + 1;
  const filters: CrudFilter[] = Object.entries(filter).map(
    ([field, value]) => ({ field, operator: "eq", value }),
  );
  try {
    const list = await provider.getList<Row & { id: string }>({
      resource,
      pagination: { currentPage, pageSize: limit, mode: "server" },
      filters,
    });
    if (page === undefined) {
      // Unreachable: the lister above is the only way `getList` resolves.
      throw new Error("the procedure answered no page");
    }
    return {
      ok: true,
      value: {
        list: { data: [...list.data], total: list.total },
        totalCount: page.totalCount,
        hasNextPage: page.hasNextPage,
        offset: (currentPage - 1) * limit,
      },
    };
  } catch (error) {
    return { ok: false, failure: failureFromError(error) };
  }
}
