import type { NextRequest, NextResponse } from "next/server";

import { handleSsoCallback } from "../../../../../src/server/sso";

/**
 * `GET /api/auth/sso/callback` — where the identity provider sends the browser
 * back to (the configured `redirect_uri`).
 *
 * Reachable without a session by design: it runs before one exists. The IdP's
 * `code` and `state` are read in `src/server/sso.ts`, sent to vpay once, and
 * never rendered or logged.
 */
export const dynamic = "force-dynamic";

export function GET(request: NextRequest): Promise<NextResponse> {
  return handleSsoCallback(request);
}
