import { callDashProcedure, type ProcedurePage } from "../server/dash-read";
import type { ApiResult } from "../server/api";
import type { StaffContext } from "../server/session";
import { PAGE_SIZE } from "./procedure-paging";
import { PROCEDURE_OF } from "./resource-name";

export { offsetFrom, PAGE_SIZE, singleParam } from "./procedure-paging";

/**
 * Read one page of a procedure-backed resource, server side.
 *
 * The screens that use this are Server Components, so the read happens where
 * the token is — the same property `app/(dash)/payments/page.tsx` documents
 * at length. `callDashProcedure` carries the 80 %-of-TTL re-mint and the
 * single-`401` retry; nothing here reimplements them.
 *
 * Returns the refusal rather than an empty page when the read fails. An
 * empty list and a refused read look identical on screen unless the screen
 * is told which it got, and rendering a refusal as "no rows" is the failure
 * `CLAUDE.md` names.
 *
 * `filter` is passed through as the procedure's own `filter` argument — the
 * named fields its `*ListFilter` type declares (`status` for refunds and
 * checkouts, `state` for deliveries). A value the procedure does not know is
 * refused by vpay with a `400` naming the field, and that refusal is what
 * the screen renders: never an empty page that reads as "none".
 */
export async function readProcedurePage<Row>(
  staff: StaffContext,
  resource: string,
  offset: number,
  filter: Readonly<Record<string, string>> = {},
  limit: number = PAGE_SIZE,
): Promise<ApiResult<ProcedurePage<Row>>> {
  const procedure = PROCEDURE_OF[resource];
  if (procedure === undefined) {
    throw new Error(`no procedure serves ${resource}`);
  }
  return callDashProcedure<ProcedurePage<Row>>(
    staff.config,
    procedure,
    { page: { limit, offset }, filter },
    staff.accessToken,
    staff.sessionToken,
  );
}

/**
 * `format.ts`'s `formatIsoInstant`, re-exported where the procedure lists
 * have always imported it from. It lives in `format.ts` since 2026-10-21 so
 * a client component can format an instant without importing this module,
 * which reaches the server-only token code through `callDashProcedure`.
 */
export { formatIsoInstant } from "../format";
