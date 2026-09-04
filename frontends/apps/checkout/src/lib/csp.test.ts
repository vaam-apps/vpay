import { describe, expect, it } from 'vitest';

import {
  CSP_FRAME_ANCESTORS_NONE,
  SECURITY_HEADERS,
  contentSecurityPolicy,
  decodeOriginsHeader,
  encodeOriginsHeader,
  frameAncestors,
} from './csp';

describe('the constant security headers', () => {
  it('are exactly the three D6 requires', () => {
    expect(SECURITY_HEADERS).toEqual({
      'Referrer-Policy': 'no-referrer',
      'Cache-Control': 'no-store',
      'X-Content-Type-Options': 'nosniff',
    });
  });
});

describe('frameAncestors', () => {
  it("is 'none' for an empty list — the fail-closed default", () => {
    expect(frameAncestors([])).toBe("'none'");
    expect(contentSecurityPolicy([])).toBe(CSP_FRAME_ANCESTORS_NONE);
  });

  it('lists the origins, space-separated, in order', () => {
    expect(contentSecurityPolicy(['https://a.example', 'https://b.example'])).toBe(
      'frame-ancestors https://a.example https://b.example',
    );
  });

  it("is 'none' when every candidate was malformed, rather than emitting the malformed value", () => {
    expect(contentSecurityPolicy(['*', 'https://shop.example/pay'])).toBe(
      CSP_FRAME_ANCESTORS_NONE,
    );
  });
});

describe('the origins request header', () => {
  it('round-trips a list', () => {
    const origins = ['https://a.example', 'https://b.example'];
    expect(decodeOriginsHeader(encodeOriginsHeader(origins))).toEqual(origins);
  });

  it('reads an absent or blank header as no origins', () => {
    expect(decodeOriginsHeader(null)).toEqual([]);
    expect(decodeOriginsHeader('')).toEqual([]);
    expect(decodeOriginsHeader('   ')).toEqual([]);
  });

  it('re-validates on the way in, so a forged header cannot smuggle a wildcard', () => {
    expect(decodeOriginsHeader('* https://shop.example')).toEqual(['https://shop.example']);
  });
});
