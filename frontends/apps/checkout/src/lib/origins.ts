/**
 * Origins: what may frame this page, and which single origin is framing it.
 *
 * Two separate questions, answered by two functions, because they have two
 * different enforcers. {@link frameAncestors} produces the header the
 * *browser* enforces before a pixel is painted; {@link resolveParentOrigin}
 * produces the value *this page* enforces on every `postMessage` it sends
 * and every one it receives. Either alone is not enough: a CSP cannot tell
 * the page who its parent is, and a JavaScript check cannot stop the frame
 * from loading.
 */

/**
 * The origin of an absolute `http:`/`https:` URL with no userinfo, or
 * `null`.
 *
 * `null` for anything else — a relative URL, a `javascript:` URL, an origin
 * with a path or credentials. Every caller here treats `null` as "refuse",
 * so a value this function cannot vouch for can never become a
 * `postMessage` target or a CSP source.
 */
export function originOf(value: string): string | null {
  let parsed: URL;
  try {
    parsed = new URL(value);
  } catch {
    return null;
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    return null;
  }
  if (parsed.username.length > 0 || parsed.password.length > 0) {
    return null;
  }
  // `new URL` happily parses `https://*.example` — `*` is a legal character
  // in a URL host — and its `.origin` round-trips, so the path-comparison
  // check below would let a wildcard through into a CSP source list. A host
  // is letters, digits, dots and hyphens (IDN arrives already punycoded), or
  // a bracketed IPv6 literal.
  const host = parsed.hostname;
  const bracketedIpv6 = host.startsWith('[') && host.endsWith(']');
  if (host.length === 0 || (!bracketedIpv6 && !/^[a-z0-9.-]+$/.test(host))) {
    return null;
  }
  return parsed.origin;
}

/**
 * Normalises the list `GET /v1/browser/checkout/origins` returned into
 * origins this page will act on.
 *
 * Anything that is not an absolute `http`/`https` **origin** is dropped
 * rather than passed through: a value with a path (`https://shop/pay`)
 * would widen a CSP source in a way the merchant did not write, and a `*`
 * would widen it to everything. Duplicates collapse, order is preserved so
 * the header is stable across requests, and the result is frozen because
 * the same array is handed to a CSP builder and to a `postMessage` check
 * and neither may mutate it.
 */
export function normalizeOrigins(raw: readonly string[]): readonly string[] {
  const seen: string[] = [];
  for (const candidate of raw) {
    if (typeof candidate !== 'string') {
      continue;
    }
    const origin = originOf(candidate.trim());
    // `new URL('https://shop.example/pay').origin` is `https://shop.example`,
    // so comparing the parsed origin back against the input is what refuses
    // a value carrying a path, a query or a fragment.
    if (origin === null || origin !== candidate.trim()) {
      continue;
    }
    if (!seen.includes(origin)) {
      seen.push(origin);
    }
  }
  return Object.freeze(seen);
}

/**
 * The single origin that framed this page, or `null` when there is not
 * exactly one that is allowed.
 *
 * `document.referrer` is the only thing a framed page can learn about its
 * embedder without asking it — and asking it is not an option, because a
 * hostile embedder would answer whatever it liked. The referrer is not
 * trusted either: it is only ever *matched against* the allow-list the
 * server produced, never used to extend it. So the worst a lying embedder
 * can do is name an origin the merchant already registered, which is an
 * origin it would have been allowed to frame from anyway.
 *
 * An empty referrer (a `Referrer-Policy` on the embedder that strips it, a
 * direct navigation) yields `null`: this page refuses rather than guessing,
 * because the alternative is a `postMessage` with no target it can name.
 */
export function resolveParentOrigin(
  referrer: string | null | undefined,
  allowed: readonly string[],
): string | null {
  if (typeof referrer !== 'string' || referrer.length === 0) {
    return null;
  }
  const origin = originOf(referrer);
  if (origin === null) {
    return null;
  }
  return allowed.includes(origin) ? origin : null;
}
