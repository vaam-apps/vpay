"use client";

import { useList } from "@refinedev/core";
import { Card, InlineEmptyState } from "@vaam-apps/ui";

import { failureFromError } from "../dash/failure";
import {
  initialFailure,
  initialQueryOptions,
  type Initial,
} from "../dash/initial";
import type { ProcedureListPage } from "../dash/procedure-list";
import { PAGE_SIZE } from "../dash/procedure-paging";
import type { ProcedureResource } from "../dash/resource-name";
import { TableCardSkeleton } from "./list-skeleton";
import { ProcedurePager } from "./procedure-pager";
import { ProcedureTable, type ProcedureColumn } from "./procedure-table";
import { ReadFailure } from "./read-failure";

/**
 * What a procedure-backed list's body needs from its Server Component.
 *
 * Every field is serialisable on purpose: this crosses the server/client
 * boundary, so the columns — which carry render functions — are supplied by
 * each list's own client module (`app/(dash)/<list>/<list>-list.tsx`), not
 * passed down from the page.
 */
export interface ProcedureListBodyProps {
  readonly resource: ProcedureResource;
  /** The table's caption — the screen's title. */
  readonly title: string;
  /** This screen's own route, e.g. `/refunds`. */
  readonly basePath: string;
  /** The procedure's filter field and the value chosen, or `''`. */
  readonly filterField: string | null;
  readonly chosen: string;
  /** The query string the pager keeps, e.g. `status=failed`, or `''`. */
  readonly query: string;
  /** The offset the page was asked for — only used if the read failed. */
  readonly offset: number;
  /** What an unfiltered empty list means, e.g. "No refunds. …". */
  readonly emptyMessage: string;
  /**
   * The page the Server Component read through `@cratestack/refine`, or the
   * refusal it met. Optional so a component test can mount this against a
   * stub provider with nothing handed in.
   */
  readonly initial?: Initial<ProcedureListPage<unknown>>;
}

/**
 * A procedure-backed list, through Refine — the shape
 * `app/(dash)/payments/payments-screen.tsx` has had since 2026-09-12, now on
 * `@cratestack/refine` rather than a hand-written provider.
 *
 * # The page is the SERVER's read
 *
 * `initial` was read by the Server Component, with the token that never
 * leaves that process, and is handed to `useList` as `initialData`.
 * `staleTime: Infinity` (`providers.tsx`) means the browser does not read it
 * again; paging and filtering are navigations, and the server reads those.
 * The `procedures` data provider this hook would fall back to is
 * `@cratestack/refine` over this app's own `GET /api/dash/<list>` routes
 * (`src/dash/procedure-provider.ts`), never over vpay.
 *
 * # The pager reads the procedure's page, not refine's `total`
 *
 * `@cratestack/refine` reports `items.length` where a procedure answers
 * `totalCount: null` (past the end of the set), which would print "0" for
 * "unknown". The count and `hasNextPage` therefore come from the server's
 * read (`ProcedureListPage`), which is the same read the rows came from.
 *
 * # A refused read is never an empty list
 *
 * `ReadFailure` with the request id, the session untouched — the rule every
 * list here follows.
 */
export function ProcedureListView<Row>({
  resource,
  title,
  basePath,
  filterField,
  chosen,
  query,
  offset,
  emptyMessage,
  initial,
  columns,
  idOf,
}: ProcedureListBodyProps & {
  readonly columns: readonly ProcedureColumn<Row>[];
  readonly idOf: (row: Row) => string;
}) {
  const serverFailure = initialFailure(initial);
  const page = initial !== undefined && initial.ok ? initial.value : null;
  const readOffset = page?.offset ?? offset;
  const { result, query: listQuery } = useList<Row & { id: string }>({
    resource,
    filters:
      filterField === null || chosen === ""
        ? []
        : [{ field: filterField, operator: "eq", value: chosen }],
    pagination: {
      currentPage: Math.floor(readOffset / PAGE_SIZE) + 1,
      pageSize: PAGE_SIZE,
      mode: "server",
    },
    queryOptions: initialQueryOptions<{
      data: (Row & { id: string })[];
      total: number;
    }>(
      initial === undefined
        ? undefined
        : initial.ok
          ? {
              ok: true,
              value: initial.value.list as {
                data: (Row & { id: string })[];
                total: number;
              },
            }
          : initial,
    ),
  });

  if (serverFailure !== null) {
    // Rendered rather than re-read from the browser — see `initial.ts`: a
    // `401` re-read through `/api/dash` reaches `authProvider.onError`, which
    // signs the person out of a session the server had just decided to keep.
    return <ReadFailure failure={serverFailure} />;
  }
  if (listQuery.isLoading) {
    return <TableCardSkeleton />;
  }
  if (listQuery.isError) {
    return <ReadFailure failure={failureFromError(listQuery.error)} />;
  }

  const rows = result?.data ?? [];
  // Only reached without a server read (a component test): refine's own
  // `total`, which `@cratestack/refine` fills from the procedure's count.
  const refineTotal = result?.total ?? null;
  if (rows.length === 0) {
    return (
      <Card>
        <InlineEmptyState
          variant="standalone"
          message={
            chosen === ""
              ? emptyMessage
              : `Nothing matches this filter. Choose "All" to see every row.`
          }
        />
      </Card>
    );
  }
  return (
    <Card className="overflow-hidden">
      <ProcedureTable
        rows={rows}
        columns={columns}
        idOf={idOf}
        caption={title}
      />
      <div className="border-t border-edge px-4 py-3">
        <ProcedurePager
          basePath={basePath}
          offset={readOffset}
          returned={rows.length}
          totalCount={page === null ? refineTotal : page.totalCount}
          hasNext={
            page === null
              ? refineTotal !== null && readOffset + rows.length < refineTotal
              : page.hasNextPage
          }
          query={query}
        />
      </div>
    </Card>
  );
}
