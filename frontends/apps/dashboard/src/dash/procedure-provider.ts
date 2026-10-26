/**
 * Refine's `procedures` data provider, in the browser: `@cratestack/refine`
 * over this app's own `GET /api/dash/<list>` routes.
 *
 * # Never over vpay, and never a `POST`
 *
 * The generated client would `POST /dash/v1/$procs/<name>` with a bearer
 * token, and neither may happen from a browser: the token lives in this
 * app's server process, and `middleware.ts` answers `405` to every method
 * but `GET`/`HEAD` on `/api/dash`. So the same manifest
 * (`procedure-resources.ts`) is bound here to a transport that `GET`s the
 * BFF, which authenticates on the httpOnly session cookie and makes the
 * procedure call server side (`src/server/bff.ts`'s `procedureListResponse`).
 * No new browser-reachable surface exists because of this file.
 *
 * # It forwards no filter, and refuses one
 *
 * The BFF forwards no filter today — its own doc comment says why — so this
 * transport's allowlist is empty and a filtered `getList` throws a `400`
 * naming the key rather than rendering an unfiltered page as if it were
 * filtered. The screens never ask it to: they render the server's read
 * (`initialData`, with `staleTime: Infinity`), and filtering is a
 * navigation.
 */
import { createCratestackDataProvider } from "@cratestack/refine";
import type { DataProvider } from "@refinedev/core";

import {
  procedureResources,
  ProcedureRefusal,
  type FilterAllowlist,
  type ProcedureLister,
} from "./procedure-resources";
import {
  BFF_PATH_OF,
  CHECKOUT_SESSIONS,
  CUSTOMERS,
  REFUNDS,
  WEBHOOK_DELIVERIES,
} from "./resource-name";

/** The BFF's procedure-list envelope, as `procedureListResponse` writes it. */
interface BffProcedureList {
  readonly data: unknown[];
  readonly total_count: number | null;
  readonly limit: number;
  readonly offset: number;
}

/** No filter crosses the BFF, for any resource. */
const NO_FILTERS: FilterAllowlist = {
  [REFUNDS]: {},
  [WEBHOOK_DELIVERIES]: {},
  [CUSTOMERS]: {},
  [CHECKOUT_SESSIONS]: {},
};

function bffLister(base: string): ProcedureLister {
  return async (resource, page) => {
    const search = new URLSearchParams();
    if (page.limit !== null) {
      search.set("limit", String(page.limit));
    }
    if (page.offset !== null) {
      search.set("offset", String(page.offset));
    }
    const query = search.toString();
    const response = await fetch(
      `${base}/${BFF_PATH_OF[resource]}${query === "" ? "" : `?${query}`}`,
      {
        headers: { accept: "application/json" },
        // Same origin, and the session cookie is httpOnly — the browser
        // attaches it and this code never reads it.
        credentials: "same-origin",
      },
    );
    if (!response.ok) {
      throw new ProcedureRefusal({
        status: response.status,
        message: `the dashboard's own /api/dash answered ${response.status}`,
        requestId: null,
      });
    }
    const body = (await response.json()) as BffProcedureList;
    const returned = body.data.length;
    return {
      items: body.data,
      totalCount: body.total_count,
      pageInfo: {
        limit: body.limit,
        offset: body.offset,
        // The BFF does not carry the procedure's `pageInfo`; this is the
        // same arithmetic the pager does when the count is known, and is
        // read by nothing in `@cratestack/refine`.
        hasNextPage:
          body.total_count !== null &&
          body.offset + returned < body.total_count,
        hasPreviousPage: body.offset > 0,
      },
    };
  };
}

/**
 * The name `<Refine dataProvider={{…}}>` registers this provider under, and
 * that every procedure-backed resource's `meta.dataProviderName` carries.
 */
export const PROCEDURES_PROVIDER = "procedures";

/**
 * The browser's provider for every procedure-backed resource.
 *
 * @param bffBase this app's own `/api/dash`
 */
export function procedureDataProvider(bffBase: string): DataProvider {
  return createCratestackDataProvider(
    procedureResources(bffLister(bffBase.replace(/\/$/, "")), NO_FILTERS),
  );
}
