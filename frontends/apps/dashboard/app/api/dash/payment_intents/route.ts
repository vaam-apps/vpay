import type { NextResponse } from "next/server";

import { dashboardConfig } from "../../../../src/config/runtime";
import { paymentIntentsListResponse } from "../../../../src/server/bff";

/**
 * `GET /api/dash/payment_intents` — the BFF's list read.
 *
 * The route and nothing else, which is the rule `README.md` states for
 * `app/`: `src/server/bff.ts` holds the origin check, the session gate, the
 * upstream call and the projection, and holds them as plain functions over a
 * `Request` so that "a caller with no cookie reaches vpay not at all" is
 * something a unit test can observe rather than something only a browser
 * could.
 *
 * **Only `GET` is exported, and that is the whole of the method policy.**
 * Next answers `405` for every other method against a route file that does
 * not export it, which mirrors `/dash/v1` itself — `dash::required_scope`
 * returns `None` for any non-`GET` and the boundary refuses it before the
 * router matches. This surface is structurally read-only rather than
 * read-only by omission.
 *
 * `force-dynamic` because it reads a cookie, like every page in this app.
 */
export const dynamic = "force-dynamic";

export function GET(request: Request): Promise<NextResponse> {
  return paymentIntentsListResponse(request, dashboardConfig().config);
}
