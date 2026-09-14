import { callDashProcedure, type ProcedurePage } from "../server/dash-read";
import type { ApiResult } from "../server/api";
import type { StaffContext } from "../server/session";
import { PROCEDURE_OF } from "./resource-name";

/** How many rows a procedure-backed screen asks for. */
export const PAGE_SIZE = 20;

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
 */
export async function readProcedurePage<Row>(
  staff: StaffContext,
  resource: string,
  offset: number,
): Promise<ApiResult<ProcedurePage<Row>>> {
  const procedure = PROCEDURE_OF[resource];
  if (procedure === undefined) {
    throw new Error(`no procedure serves ${resource}`);
  }
  return callDashProcedure<ProcedurePage<Row>>(
    staff.config,
    procedure,
    { page: { limit: PAGE_SIZE, offset }, filter: {} },
    staff.accessToken,
    staff.sessionToken,
  );
}

/** `?offset=` as a non-negative integer, or 0. */
export function offsetFrom(
  params: Record<string, string | string[] | undefined>,
): number {
  const raw = params["offset"];
  const parsed = Number(Array.isArray(raw) ? raw[0] : raw);
  return Number.isInteger(parsed) && parsed >= 0 ? parsed : 0;
}

/**
 * An RFC 3339 timestamp, as an operator reads it.
 *
 * **Not `format.ts`'s `formatInstant`, which takes unix seconds.** The REST
 * list serialises times the way the merchant API does — an integer — and a
 * CrateStack procedure serialises a `DateTime` as a string. Passing one to
 * the other's formatter yields `Invalid Date` or, worse, a plausible date in
 * 1970. The two formatters exist because the two wire shapes exist.
 *
 * An unparseable value is returned verbatim rather than replaced: if vpay
 * ever answers something this cannot read, an operator should see what it
 * actually said.
 */
export function formatIsoInstant(value: string | null | undefined): string {
  if (value === null || value === undefined) {
    return "—";
  }
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) {
    return value;
  }
  return new Date(parsed).toISOString().replace("T", " ").slice(0, 19) + " UTC";
}
