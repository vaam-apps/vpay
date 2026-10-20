import type { NextResponse } from "next/server";

import { handleSsoStart } from "../../../../../src/server/sso";

/**
 * `POST /api/auth/sso/start` — what the "Sign in with Keycloak" button posts
 * to. A plain HTML form post, so it works without JavaScript.
 *
 * Only `POST` is exported on purpose: a `GET` that sets a cookie and redirects
 * to an identity provider is the link a chat message can fire. The middleware
 * is not involved — it matches `/api/dash/**` only, and this route has to be
 * reachable before any session exists. `src/server/sso.ts` has the rest.
 */
export const dynamic = "force-dynamic";

export function POST(): Promise<NextResponse> {
  return handleSsoStart();
}
