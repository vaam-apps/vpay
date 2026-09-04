/**
 * The origin logic: what may frame this page, and who is framing it.
 */
import { describe, expect, it } from 'vitest';

import { normalizeOrigins, originOf, resolveParentOrigin } from './origins';

describe('originOf', () => {
  it('returns the origin of an absolute http(s) URL', () => {
    expect(originOf('https://shop.example/pay?x=1#y')).toBe('https://shop.example');
    expect(originOf('http://localhost:3000/')).toBe('http://localhost:3000');
  });

  it('is null for a scheme that is not http(s)', () => {
    expect(originOf('javascript:alert(1)')).toBeNull();
    expect(originOf('data:text/html,x')).toBeNull();
    expect(originOf('file:///etc/passwd')).toBeNull();
  });

  it('is null for a relative URL', () => {
    expect(originOf('/pay')).toBeNull();
    expect(originOf('shop.example')).toBeNull();
  });

  it('is null for a URL carrying credentials', () => {
    expect(originOf('https://user:pass@shop.example')).toBeNull();
  });
});

describe('normalizeOrigins', () => {
  it('keeps well-formed origins in order and drops duplicates', () => {
    expect(
      normalizeOrigins(['https://a.example', 'https://b.example', 'https://a.example']),
    ).toEqual(['https://a.example', 'https://b.example']);
  });

  it('drops a value carrying a path, which would widen the policy beyond what the merchant wrote', () => {
    expect(normalizeOrigins(['https://shop.example/pay'])).toEqual([]);
  });

  it('drops a wildcard rather than passing it through to a CSP', () => {
    expect(normalizeOrigins(['*'])).toEqual([]);
    expect(normalizeOrigins(['https://*.example'])).toEqual([]);
  });

  it('drops a trailing slash form, since it is not an origin', () => {
    expect(normalizeOrigins(['https://shop.example/'])).toEqual([]);
  });

  it('is empty for an empty input, which is the fail-closed shape', () => {
    expect(normalizeOrigins([])).toEqual([]);
  });
});

describe('resolveParentOrigin', () => {
  const allowed = ['https://shop.example', 'https://other.example'];

  it('resolves the framer when it is on the list', () => {
    expect(resolveParentOrigin('https://shop.example/checkout', allowed)).toBe(
      'https://shop.example',
    );
  });

  it('refuses a framer that is not on the list', () => {
    expect(resolveParentOrigin('https://evil.example/checkout', allowed)).toBeNull();
  });

  it('refuses when the list is empty, whatever the referrer says', () => {
    expect(resolveParentOrigin('https://shop.example/checkout', [])).toBeNull();
  });

  it('refuses an absent or empty referrer rather than guessing a parent', () => {
    expect(resolveParentOrigin('', allowed)).toBeNull();
    expect(resolveParentOrigin(null, allowed)).toBeNull();
    expect(resolveParentOrigin(undefined, allowed)).toBeNull();
  });

  it('refuses a look-alike origin', () => {
    expect(resolveParentOrigin('https://shop.example.evil.test/', allowed)).toBeNull();
    expect(resolveParentOrigin('http://shop.example/', allowed)).toBeNull();
    expect(resolveParentOrigin('https://shop.example:8443/', allowed)).toBeNull();
  });
});
