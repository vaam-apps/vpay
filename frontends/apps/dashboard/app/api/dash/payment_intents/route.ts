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
 * **Only `GET` is exported, and no write can reach this file**, which mirrors
 * `/dash/v1` itself — `dash::required_scope` returns `None` for any non-`GET`
 * and the boundary refuses it before the router matches. This surface is
 * structurally read-only rather than read-only by omission.
 *
 * **That is not the same as "Next answers `405` for every other method",
 * which this comment claimed until the exp55 security review read
 * `next/dist/server/route-modules/app-route/helpers/auto-implement-methods`
 * (Next 16.3.4) instead of assuming it.** Two methods are implemented for us:
 *
 * - **`HEAD` runs `GET`** — literally the same handler, with the body
 *   discarded (`methods.HEAD = handlers.GET`). A `HEAD` therefore costs the
 *   same session read, the same possible token mint and the same upstream
 *   call as a `GET`, and answers the status and the headers. That is
 *   consistent with `/dash/v1`, which admits `GET` **and** `HEAD`; it just
 *   means the reachable method set here is three, not one.
 * - **`OPTIONS` answers `204` with `Allow: GET, HEAD, OPTIONS`** — and
 *   answers it *before* this file runs, so before the origin check and before
 *   the cookie is read. It carries no data, but it tells an unauthenticated
 *   caller that this route exists where a `404` would not. Suppressing it
 *   would take a middleware and this app has none, so it is recorded rather
 *   than fixed: the route names are in this repository anyway.
 *
 * Every method that is not one of those three is Next's `405`.
 *
 * `force-dynamic` because it reads a cookie, like every page in this app.
 */
export const dynamic = "force-dynamic";

export function GET(request: Request): Promise<NextResponse> {
  return paymentIntentsListResponse(request, dashboardConfig().config);
}
