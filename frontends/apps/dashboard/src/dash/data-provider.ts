/**
 * Refine's `DataProvider`, over the BFF, for the one resource `/dash/v1`
 * serves.
 *
 * This is Lane 3 of `docs/plans/exp55-refine-seam-bff-notes/refine-plan.md`
 * and it is deliberately **hand-written** rather than `@cratestack/refine`:
 * §3.3 of that plan records why generation is foreclosed for this surface
 * today, and it is not a preference.
 *
 * # It talks to the BFF, never to `/dash/v1`
 *
 * Refine's hooks run in the browser. `/dash/v1`'s bearer token must not, and
 * nothing in this file has ever seen one — `getApiUrl` returns this app's own
 * `/api/dash` and every request goes there, on the same origin, carrying the
 * httpOnly session cookie the browser attaches by itself. The token is read
 * server-side by `src/server/bff.ts`, which is the only thing that holds it.
 *
 * # Reads only, and the refusals are honest
 *
 * `/dash/v1` answers `403` to every non-`GET` at its boundary. A provider
 * with a `create` that "works" would be a lie that only surfaces as a 403 at
 * submit time, so the three write methods throw, and `resources` declares
 * `create: false, edit: false, canDelete: false` so no button ever renders.
 * `getMany` is absent for the same kind of reason: there is no batch read,
 * and faking one as N round trips on a money surface buys nothing.
 */
import type { DataProvider } from "@refinedev/core";

import { refineCursor } from "./initial";
import { PAYMENT_INTENTS, type DashResource } from "./resource-name";
import type { PaymentIntentObject } from "../server/api";
import { PAGE_SIZE, type PageCursors } from "../payments-query";

/** The BFF's list envelope, as `paymentIntentsListResponse` serialises it. */
interface BffList {
  readonly object: "list";
  readonly data: readonly PaymentIntentObject[];
  readonly has_more: boolean;
  readonly cursor: PageCursors;
}

/**
 * The error a failed read throws, carrying the status Refine's `onError` and
 * the detail route both branch on.
 *
 * `statusCode` is the property name Refine's own `HttpError` uses, so
 * `authProvider.onError` can read it without this module and that one
 * agreeing on a second spelling.
 */
export class DashHttpError extends Error {
  readonly statusCode: number;

  constructor(statusCode: number, message: string) {
    super(message);
    this.name = "DashHttpError";
    this.statusCode = statusCode;
  }
}

function assertPaymentIntents(
  resource: string,
): asserts resource is DashResource {
  if (resource !== PAYMENT_INTENTS) {
    throw new DashHttpError(
      404,
      `/dash/v1 serves no resource "${resource}" — there is exactly one`,
    );
  }
}

/**
 * A repeated query parameter takes the **first** value, never the last and
 * never joined — the rule `one()` carries in `payments-query.ts`, restated
 * here because this provider builds a request rather than reading one. A
 * joined `succeeded,canceled` is a 400 naming a parameter the operator never
 * typed.
 */
function firstOf(value: unknown): string {
  if (Array.isArray(value)) {
    const head: unknown = value[0];
    return typeof head === "string" ? head.trim() : "";
  }
  return typeof value === "string" ? value.trim() : "";
}

/** `YYYY-MM-DD`, or nothing. An unparseable date is dropped, not forwarded. */
function asDate(value: string): string {
  return /^\d{4}-\d{2}-\d{2}$/.test(value) ? value : "";
}

async function readJson(url: string): Promise<BffList | PaymentIntentObject> {
  const response = await fetch(url, {
    headers: { accept: "application/json" },
    // Same origin, and the session cookie is httpOnly — the browser attaches
    // it and this code never reads it.
    credentials: "same-origin",
  });
  if (!response.ok) {
    throw new DashHttpError(
      response.status,
      `the dashboard's own /api/dash answered ${response.status}`,
    );
  }
  return (await response.json()) as BffList | PaymentIntentObject;
}

export function dashDataProvider(bffBase: string): DataProvider {
  const base = bffBase.replace(/\/$/, "");

  return {
    getApiUrl: () => base,

    async getList({ resource, filters, meta }) {
      assertPaymentIntents(resource);
      const search = new URLSearchParams();

      for (const filter of filters ?? []) {
        if (!("field" in filter)) {
          continue;
        }
        const value = firstOf(filter.value);
        if (value === "") {
          continue;
        }
        if (filter.field === "status") {
          search.set("status", value);
        } else if (filter.field === "created_from") {
          const date = asDate(value);
          if (date !== "") {
            search.set("created_from", date);
          }
        } else if (filter.field === "created_to") {
          const date = asDate(value);
          if (date !== "") {
            search.set("created_to", date);
          }
        }
      }

      // Both cursors present → `after` wins. The server would answer 400; the
      // client should not put it in that position.
      const after = firstOf(meta?.["after"]);
      const before = firstOf(meta?.["before"]);
      if (after !== "") {
        search.set("after", after);
      } else if (before !== "") {
        search.set("before", before);
      }

      // Refine fills `pagination.pageSize` even under the `mode: "off"` this
      // app's own `useList` call uses (`payments-screen.tsx`) — its own
      // default of 10, unrelated to anything vpay serves. The BFF's own
      // `paymentIntentsListResponse` reads no `limit` from this request at
      // all: `queryFrom` names five parameters and `limit` is not one of
      // them, and the BFF always asks vpay for `PAGE_SIZE` rows itself
      // (`payments-query.ts`'s `apiQueryString`). Putting Refine's number on
      // the wire would read as a request nothing honours while every answer
      // actually carries `PAGE_SIZE` rows — a request log would believe a
      // page size this code does not control and did not ask for. Send the
      // number that is actually true instead, from the one place it is
      // defined.
      search.set("limit", String(PAGE_SIZE));

      const query = search.toString();
      const page = (await readJson(
        `${base}/${PAYMENT_INTENTS}${query === "" ? "" : `?${query}`}`,
      )) as BffList;

      return {
        data: page.data,
        // `/dash/v1` is cursor-paged and reports no count. `0` is Refine's
        // documented shape for "unknown", and it is honest here in a way a
        // guess from `data.length` would not be — see the plan's §2.4.
        total: 0,
        // `refineCursor` rather than the inversion spelled out again: the
        // server-rendered first page (`initial.ts`) has to say the same
        // thing, and two transcriptions of "next is older" could disagree
        // about which way a pager link walks.
        cursor: refineCursor(page.cursor),
      } as never;
    },

    async getOne({ resource, id }) {
      assertPaymentIntents(resource);
      const one = await readJson(
        `${base}/${PAYMENT_INTENTS}/${encodeURIComponent(String(id))}`,
      );
      return { data: one as never };
    },

    create: () => {
      throw new DashHttpError(403, "/dash/v1 is read-only");
    },
    update: () => {
      throw new DashHttpError(403, "/dash/v1 is read-only");
    },
    deleteOne: () => {
      throw new DashHttpError(403, "/dash/v1 is read-only");
    },
  };
}

export type { BffList };
