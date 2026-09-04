/**
 * The response headers every vpay-served checkout page carries.
 *
 * Set in `middleware.ts` and nowhere else, so there is one place to read
 * and one place to change. `middleware.test.ts` asserts the exact strings on
 * each of the three routes.
 */
import { normalizeOrigins } from './origins';

/**
 * D6, plus the two that always travel with it.
 *
 * - `Referrer-Policy: no-referrer` — this page's own URL carries a
 *   credential in its fragment and the publishable key in its query. A
 *   referrer is how that leaves the browser to the rail, to an image host,
 *   to anything.
 * - `Cache-Control: no-store` — a payment page in a shared browser's cache
 *   is a payment page someone else can reopen.
 * - `X-Content-Type-Options: nosniff` — nothing here is served as a
 *   guessable type.
 */
export const SECURITY_HEADERS: Readonly<Record<string, string>> = Object.freeze({
  'Referrer-Policy': 'no-referrer',
  'Cache-Control': 'no-store',
  'X-Content-Type-Options': 'nosniff',
});

/** The CSP source list for `frame-ancestors`. Empty means `'none'`. */
export function frameAncestors(origins: readonly string[]): string {
  const normalized = normalizeOrigins(origins);
  return normalized.length === 0 ? "'none'" : normalized.join(' ');
}

/**
 * The whole `Content-Security-Policy` header value.
 *
 * `frame-ancestors` and nothing else, and that is a **stated gap** rather
 * than an oversight: a `script-src`/`default-src` policy strict enough to be
 * worth having needs a per-request nonce threaded through Next's inline
 * bootstrap scripts, which this lane did not build. Shipping a permissive
 * `default-src` here would read like a content policy while forbidding
 * nothing. See `docs/plans/step9-notes/lane-3.md`.
 */
export function contentSecurityPolicy(origins: readonly string[]): string {
  return `frame-ancestors ${frameAncestors(origins)}`;
}

/** `frame-ancestors 'none'` — the hosted page and the return page, always. */
export const CSP_FRAME_ANCESTORS_NONE = "frame-ancestors 'none'";

/**
 * The request header `middleware.ts` uses to tell the embedded route's
 * server component what the origins lookup found.
 *
 * A request header rather than a cookie or a searchParam: cookies are out
 * (this app sets none), and a query parameter would be attacker-controllable
 * — the whole point is that this value came from vpay's own API on the
 * server side of this request.
 */
export const EMBED_ORIGINS_HEADER = 'x-vpay-embed-origins';

/** Serialises an origin list for {@link EMBED_ORIGINS_HEADER}. */
export function encodeOriginsHeader(origins: readonly string[]): string {
  return normalizeOrigins(origins).join(' ');
}

/** Reads {@link EMBED_ORIGINS_HEADER} back. An absent or empty header is an empty list. */
export function decodeOriginsHeader(value: string | null | undefined): readonly string[] {
  if (typeof value !== 'string' || value.trim().length === 0) {
    return Object.freeze([]);
  }
  return normalizeOrigins(value.trim().split(/\s+/));
}
