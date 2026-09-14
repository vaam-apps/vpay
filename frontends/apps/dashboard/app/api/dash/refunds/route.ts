import type { NextResponse } from "next/server";

import { dashboardConfig } from "../../../../src/config/runtime";
import { procedureListResponse } from "../../../../src/server/bff";
import { REFUNDS } from "../../../../src/dash/resource-name";

/**
 * `GET /api/dash/refunds` — one page of this merchant's rows.
 *
 * The route and nothing else, which is the rule `README.md` states for
 * `app/`: the origin check, the session gate, the upstream call and the
 * projection all live in `src/server/bff.ts` as plain functions over a
 * `Request`, so "a caller with no cookie reaches vpay not at all" is
 * something a unit test can observe.
 *
 * **Only `GET` is exported.** The resource behind it is a CrateStack
 * procedure, which the generated transport mounts as `POST`, but that
 * `POST` is made server-side by `procedureListResponse`. On this surface
 * `middleware.ts` answers `405` to every method but `GET`/`HEAD`, so the
 * browser-facing half stays structurally read-only exactly as
 * `payment_intents/route.ts` is.
 *
 * `force-dynamic` because it reads a cookie, like every page in this app.
 */
export const dynamic = "force-dynamic";

export function GET(request: Request): Promise<NextResponse> {
  return procedureListResponse(request, REFUNDS, dashboardConfig().config);
}
