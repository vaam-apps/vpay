import type { NextResponse } from 'next/server';

import { signedOutResponse } from '../../src/server/signed-out';

/**
 * `GET /signed-out` — the one place a *page* can get a dead session cookie
 * removed from a browser.
 *
 * A Route Handler because `cookies().set(…)` throws anywhere else but here and
 * a server action, and a page that called it answered `500` instead of the
 * sign-in form. `src/server/signed-out.ts` has the whole of the reasoning and
 * the measurement; this file is the route and nothing else, which is the rule
 * `README.md` states for `app/`.
 *
 * There is no loop: `/login` renders a form for a browser whose session is
 * dead rather than redirecting it anywhere.
 */
export const dynamic = 'force-dynamic';

export function GET(): NextResponse {
  return signedOutResponse();
}
