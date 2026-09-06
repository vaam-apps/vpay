/**
 * Every security header this app sends, in one place.
 *
 * Three of them are constants (`Referrer-Policy: no-referrer`,
 * `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`) and go on
 * every response. The fourth, `Content-Security-Policy: frame-ancestors …`,
 * is the reason this is middleware rather than a static `headers()` table in
 * `next.config.ts`: for `/e/{id}` its value is the merchant's registered
 * origin list, which only vpay's API knows, and it has to be on the HTML
 * response itself — before a single byte of script runs.
 *
 * **Fail-closed, in four ways.** No `key` in the URL, no `VPAY_API_URL`
 * configured, a lookup that failed, and a lookup that returned an empty
 * list all produce the same header as the hosted page:
 * `frame-ancestors 'none'`. There is no branch in which an unknown answer
 * widens the policy.
 *
 * The resolved list is forwarded to the route on a request header
 * ({@link EMBED_ORIGINS_HEADER}) so the page's own `postMessage` check uses
 * the same list the browser was given, rather than looking it up a second
 * time and possibly getting a different answer.
 *
 * Since 2026-09-06 the **hosted** page gets that header too, because it may
 * be in a popup and needs an origin to post to. Its CSP does not change: see
 * {@link HOSTED_PATH}.
 */
import { NextResponse, type NextRequest } from 'next/server';

import { fetchCheckoutOrigins } from './src/lib/api';
import {
  EMBED_ORIGINS_HEADER,
  SECURITY_HEADERS,
  contentSecurityPolicy,
  encodeOriginsHeader,
} from './src/lib/csp';
import { serverApiBaseUrl } from './src/lib/env';
import { normalizeOrigins } from './src/lib/origins';

/** `/e/{cs_id}` and nothing else. The hosted and return pages are never framed. */
const EMBEDDED_PATH = /^\/e\/[^/]+\/?$/;

/**
 * `/c/{cs_id}` and nothing else — **not** `/c/{cs_id}/return`.
 *
 * The hosted page needs the origin list too, since 2026-09-06, for a reason
 * that has nothing to do with framing: it may be running in a **popup** the
 * merchant's page opened, and the origin it may `postMessage` to has to come
 * from the same server-side lookup the embedded page's does. Its CSP is
 * unaffected and stays `frame-ancestors 'none'` — a hosted page is never
 * framed, popup or not, and the two uses of this list are kept apart below
 * so that widening one cannot widen the other.
 *
 * The **return** page needs it too, since the maintainer's decision of
 * 2026-09-06, but for a third reason again: its referrer is the *rail's*
 * origin, so it resolves an opener by a different rule — the merchant's
 * single registered origin, where there is exactly one (`soleOrigin`).
 */
const HOSTED_PATH = /^\/c\/[^/]+\/?$/;

/** `/c/{cs_id}/return`. Same lookup, `soleOrigin`'s rule, still `frame-ancestors 'none'`. */
const RETURN_PATH = /^\/c\/[^/]+\/return\/?$/;

export async function middleware(request: NextRequest): Promise<NextResponse> {
  const path = request.nextUrl.pathname;
  const embedded = EMBEDDED_PATH.test(path);
  let origins: readonly string[] = [];

  if (embedded || HOSTED_PATH.test(path) || RETURN_PATH.test(path)) {
    const key = request.nextUrl.searchParams.get('key');
    const baseUrl = serverApiBaseUrl();
    if (key !== null && key.length > 0 && baseUrl !== null) {
      origins = normalizeOrigins(await fetchCheckoutOrigins(baseUrl, key, fetch));
    }
  }

  const requestHeaders = new Headers(request.headers);
  // Overwritten, never appended to: a caller cannot smuggle an origin in by
  // sending this header itself.
  requestHeaders.set(EMBED_ORIGINS_HEADER, encodeOriginsHeader(origins));

  const response = NextResponse.next({ request: { headers: requestHeaders } });
  for (const [name, value] of Object.entries(SECURITY_HEADERS)) {
    response.headers.set(name, value);
  }
  // **Only the embedded path's list ever reaches the CSP.** A hosted page is
  // `frame-ancestors 'none'` whatever the lookup returned; the list it
  // carries is for `postMessage`, and conflating the two would let a
  // merchant's popup registration make its hosted page framable.
  response.headers.set('Content-Security-Policy', contentSecurityPolicy(embedded ? origins : []));
  return response;
}

/**
 * Every path, static assets included.
 *
 * `no-store` on `/_next/static` costs a cache hit per asset. On a page that
 * exists for one payment at a time, on a shared or borrowed handset, that is
 * the right trade: the alternative is a matcher whose exclusions are the one
 * part of the security headers nobody re-reads.
 */
export const config = {
  matcher: ['/:path*'],
};
