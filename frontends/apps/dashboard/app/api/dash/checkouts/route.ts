import type { NextResponse } from "next/server";

import { dashboardConfig } from "../../../../src/config/runtime";
import { procedureListResponse } from "../../../../src/server/bff";
import { CHECKOUT_SESSIONS } from "../../../../src/dash/resource-name";

/**
 * `GET /api/dash/checkouts` — one page of this merchant's checkout sessions.
 *
 * The route and nothing else; the origin check, the session gate, the
 * upstream call and the projection live in `src/server/bff.ts`. Only `GET`
 * is exported, and `middleware.ts` answers `405` to every method but
 * `GET`/`HEAD` on `/api/dash/:path*` — the `POST` to the procedure transport
 * is made server-side on behalf of this `GET`.
 *
 * `force-dynamic` because it reads a cookie, like every page in this app.
 */
export const dynamic = "force-dynamic";

export function GET(request: Request): Promise<NextResponse> {
  return procedureListResponse(
    request,
    CHECKOUT_SESSIONS,
    dashboardConfig().config,
  );
}
