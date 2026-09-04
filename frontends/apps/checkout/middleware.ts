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

export async function middleware(request: NextRequest): Promise<NextResponse> {
  let origins: readonly string[] = [];

  if (EMBEDDED_PATH.test(request.nextUrl.pathname)) {
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
  response.headers.set('Content-Security-Policy', contentSecurityPolicy(origins));
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
