/**
 * The claim in `env.ts` that bracket access is a runtime read, asserted.
 *
 * If Next's build-time substitution ever started replacing
 * `process.env['NEXT_PUBLIC_…']` too, the value would be frozen into lane
 * 4's image and every deployment of it would call the same API. This is the
 * cheapest possible tripwire for that: set the variable *after* the module
 * was imported and read it back.
 */
import { afterEach, describe, expect, it } from 'vitest';

import { browserApiBaseUrl, serverApiBaseUrl } from './env';

afterEach(() => {
  delete process.env['NEXT_PUBLIC_VPAY_API_URL'];
  delete process.env['VPAY_API_URL'];
});

describe('browserApiBaseUrl', () => {
  it('reads the variable at call time, not at import time', () => {
    process.env['NEXT_PUBLIC_VPAY_API_URL'] = 'https://api.one.test';
    expect(browserApiBaseUrl()).toBe('https://api.one.test');
    process.env['NEXT_PUBLIC_VPAY_API_URL'] = 'https://api.two.test';
    expect(browserApiBaseUrl()).toBe('https://api.two.test');
  });

  it('trims, because a trailing newline in a Kubernetes secret is a common way to lose an hour', () => {
    process.env['NEXT_PUBLIC_VPAY_API_URL'] = '  https://api.test\n';
    expect(browserApiBaseUrl()).toBe('https://api.test');
  });

  it('throws when unset, rather than serving a payment page pointed nowhere', () => {
    expect(() => browserApiBaseUrl()).toThrow(/NEXT_PUBLIC_VPAY_API_URL/);
  });

  it('throws for a blank value', () => {
    process.env['NEXT_PUBLIC_VPAY_API_URL'] = '   ';
    expect(() => browserApiBaseUrl()).toThrow();
  });
});

describe('serverApiBaseUrl', () => {
  it('is null when unset, which middleware treats as "no origins"', () => {
    expect(serverApiBaseUrl()).toBeNull();
  });

  it('reads the variable at call time', () => {
    process.env['VPAY_API_URL'] = 'http://vpay-server:8080';
    expect(serverApiBaseUrl()).toBe('http://vpay-server:8080');
  });
});
