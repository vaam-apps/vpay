import type { NextResponse } from "next/server";

import { dashboardConfig } from "../../../../../src/config/runtime";
import { paymentIntentResponse } from "../../../../../src/server/bff";

/**
 * `GET /api/dash/payment_intents/{id}` — the BFF's detail read.
 *
 * The route and nothing else; `src/server/bff.ts` carries the reasoning, and
 * the sibling `route.ts` the note on why only `GET` is exported.
 *
 * The id is a path segment Next parsed out of the URL, and it is handed on
 * unexamined: shape-checking it here would put a second, weaker opinion about
 * what a `pi_…` looks like in front of vpay's, and `/dash/v1` answers the
 * same `404` for a malformed id, a foreign merchant's and one that never
 * existed. The seam encodes it as one path segment before it is sent, so a
 * `../staff/session` cannot become a different upstream route.
 */
export const dynamic = "force-dynamic";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<NextResponse> {
  const { id } = await params;
  return paymentIntentResponse(request, id, dashboardConfig().config);
}
