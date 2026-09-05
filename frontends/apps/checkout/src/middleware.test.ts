/**
 * The headers, per route, asserted as exact strings.
 *
 * This calls the shipping `middleware` function with a real `NextRequest`
 * and reads the real `NextResponse` it returns — not a re-implementation of
 * what it is supposed to do. `frame-ancestors` is the one that would be
 * silently wrong: a policy that lists an origin nobody registered, or
 * `'none'` where a merchant expected its own site, is invisible until a
 * payer's browser refuses to paint.
 */
import { NextRequest } from 'next/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { middleware } from '../middleware';

const API = 'https://api.vpay.test';

function request(path: string): NextRequest {
  return new NextRequest(new URL(path, 'https://checkout.example'));
}

/** A `fetch` that answers the origins route and records what it was asked. */
function originsFetch(origins: string[] | null, status = 200) {
  const calls: string[] = [];
  // a `fetch` stand-in must return a promise; this one answers from a literal.
  // eslint-disable-next-line @typescript-eslint/require-await
  const impl = vi.fn(async (input: RequestInfo | URL) => {
    // A `Request` stringifies to "[object Request]"; these calls are
    // asserted against by URL.
    calls.push(
      typeof input === 'string'
        ? input
        : input instanceof URL
          ? input.href
          : input.url,
    );
    if (origins === null) {
      throw new TypeError('network');
    }
    return new Response(JSON.stringify({ origins }), {
      status,
      headers: { 'Content-Type': 'application/json' },
    });
  });
  return { impl, calls };
}

beforeEach(() => {
  process.env['VPAY_API_URL'] = API;
});

afterEach(() => {
  vi.unstubAllGlobals();
  delete process.env['VPAY_API_URL'];
});

describe('every response', () => {
  it('carries the three constant headers on every route', async () => {
    const { impl } = originsFetch([]);
    vi.stubGlobal('fetch', impl);
    for (const path of ['/c/cs_1', '/c/cs_1/return?t=tok', '/e/cs_1?key=pk_1', '/_next/static/x.js']) {
      const response = await middleware(request(path));
      expect(response.headers.get('referrer-policy'), path).toBe('no-referrer');
      expect(response.headers.get('cache-control'), path).toBe('no-store');
      expect(response.headers.get('x-content-type-options'), path).toBe('nosniff');
    }
  });
});

describe('the hosted page', () => {
  it("is frame-ancestors 'none' and asks the API nothing", async () => {
    const { impl, calls } = originsFetch(['https://shop.example']);
    vi.stubGlobal('fetch', impl);
    const response = await middleware(request('/c/cs_1'));
    expect(response.headers.get('content-security-policy')).toBe("frame-ancestors 'none'");
    expect(calls).toEqual([]);
  });
});

describe('the return page', () => {
  it("is frame-ancestors 'none' — it is top-level in both modes", async () => {
    const { impl } = originsFetch(['https://shop.example']);
    vi.stubGlobal('fetch', impl);
    const response = await middleware(request('/c/cs_1/return?t=tok'));
    expect(response.headers.get('content-security-policy')).toBe("frame-ancestors 'none'");
  });
});

describe('the embedded page', () => {
  it('lists exactly the origins the API returned for that key', async () => {
    const { impl, calls } = originsFetch(['https://shop.example', 'https://www.shop.example']);
    vi.stubGlobal('fetch', impl);
    const response = await middleware(request('/e/cs_1?key=pk_test_1'));
    expect(response.headers.get('content-security-policy')).toBe(
      'frame-ancestors https://shop.example https://www.shop.example',
    );
    expect(calls).toEqual([`${API}/v1/browser/checkout/origins?key=pk_test_1`]);
  });

  it("is 'none' when the merchant has registered no origin", async () => {
    const { impl } = originsFetch([]);
    vi.stubGlobal('fetch', impl);
    const response = await middleware(request('/e/cs_1?key=pk_test_1'));
    expect(response.headers.get('content-security-policy')).toBe("frame-ancestors 'none'");
  });

  it("is 'none' when the lookup fails — fail-closed, not fail-open", async () => {
    const { impl } = originsFetch(null);
    vi.stubGlobal('fetch', impl);
    const response = await middleware(request('/e/cs_1?key=pk_test_1'));
    expect(response.headers.get('content-security-policy')).toBe("frame-ancestors 'none'");
  });

  it("is 'none' when the API refuses the key", async () => {
    const { impl } = originsFetch(['https://shop.example'], 404);
    vi.stubGlobal('fetch', impl);
    const response = await middleware(request('/e/cs_1?key=pk_unknown'));
    expect(response.headers.get('content-security-policy')).toBe("frame-ancestors 'none'");
  });

  it("is 'none' when the URL carries no key at all", async () => {
    const { impl, calls } = originsFetch(['https://shop.example']);
    vi.stubGlobal('fetch', impl);
    const response = await middleware(request('/e/cs_1'));
    expect(response.headers.get('content-security-policy')).toBe("frame-ancestors 'none'");
    expect(calls).toEqual([]);
  });

  it("is 'none' when VPAY_API_URL is not configured", async () => {
    delete process.env['VPAY_API_URL'];
    const { impl, calls } = originsFetch(['https://shop.example']);
    vi.stubGlobal('fetch', impl);
    const response = await middleware(request('/e/cs_1?key=pk_test_1'));
    expect(response.headers.get('content-security-policy')).toBe("frame-ancestors 'none'");
    expect(calls).toEqual([]);
  });

  it('drops a malformed origin the API returned rather than putting it in the policy', async () => {
    const { impl } = originsFetch(['*', 'https://shop.example/pay', 'https://ok.example']);
    vi.stubGlobal('fetch', impl);
    const response = await middleware(request('/e/cs_1?key=pk_test_1'));
    expect(response.headers.get('content-security-policy')).toBe(
      'frame-ancestors https://ok.example',
    );
  });

  it('does not treat a nested path as the embedded route', async () => {
    const { impl, calls } = originsFetch(['https://shop.example']);
    vi.stubGlobal('fetch', impl);
    const response = await middleware(request('/e/cs_1/extra?key=pk_test_1'));
    expect(response.headers.get('content-security-policy')).toBe("frame-ancestors 'none'");
    expect(calls).toEqual([]);
  });

  it('forwards the resolved origins to the route, and overwrites any header a caller sent', async () => {
    const { impl } = originsFetch(['https://shop.example']);
    vi.stubGlobal('fetch', impl);
    const forged = new NextRequest(new URL('/e/cs_1?key=pk_test_1', 'https://checkout.example'), {
      headers: { 'x-vpay-embed-origins': 'https://evil.example' },
    });
    const response = await middleware(forged);
    // `NextResponse.next({ request: { headers } })` carries the rewritten
    // request headers on this response header, which is how Next hands them
    // to the route handler.
    const forwarded = response.headers.get('x-middleware-override-headers');
    expect(forwarded).toContain('x-vpay-embed-origins');
    expect(response.headers.get('x-middleware-request-x-vpay-embed-origins')).toBe(
      'https://shop.example',
    );
  });
});
