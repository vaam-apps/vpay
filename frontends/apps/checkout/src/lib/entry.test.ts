/**
 * The decision every page makes before it reads anything.
 */
import { describe, expect, it } from 'vitest';

import { decideEntry, decideReturnEntry } from './entry';

const SECRET = 'cs_test_1_secret_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const ALLOWED = ['https://shop.example'];

function embedded(overrides: Partial<Parameters<typeof decideEntry>[0]> = {}) {
  return decideEntry({
    mode: 'embedded',
    search: '?key=pk_test_1',
    hash: `#${SECRET}`,
    referrer: 'https://shop.example/cart',
    allowedOrigins: ALLOWED,
    framed: true,
    ...overrides,
  });
}

describe('an embedded page', () => {
  it('is ready when the framer is on the merchant’s list', () => {
    expect(embedded()).toEqual({
      kind: 'ready',
      key: 'pk_test_1',
      clientSecret: SECRET,
      parentOrigin: 'https://shop.example',
    });
  });

  it('refuses a framer that is not on the list', () => {
    expect(embedded({ referrer: 'https://evil.example/' })).toEqual({ kind: 'refused' });
  });

  it('refuses when the origins lookup produced nothing — fail-closed', () => {
    expect(embedded({ allowedOrigins: [] })).toEqual({ kind: 'refused' });
  });

  it('refuses when no referrer reached it, rather than guessing a parent', () => {
    expect(embedded({ referrer: '' })).toEqual({ kind: 'refused' });
  });

  it('refuses before it looks at the credential, so a hostile framer learns nothing', () => {
    // No key and no secret in the URL, and the answer is still `refused`
    // rather than `missing_key` — the refusal cannot be used to probe which
    // half of a link is wrong.
    expect(embedded({ referrer: 'https://evil.example/', search: '', hash: '' })).toEqual({
      kind: 'refused',
    });
  });

  it('refuses when opened top-level, where there is no parent to report to', () => {
    expect(embedded({ framed: false })).toEqual({ kind: 'refused' });
  });
});

describe('a hosted page', () => {
  const hosted = (overrides: Partial<Parameters<typeof decideEntry>[0]> = {}) =>
    decideEntry({
      mode: 'hosted',
      search: '?key=pk_test_1',
      hash: `#${SECRET}`,
      referrer: null,
      allowedOrigins: [],
      framed: false,
      ...overrides,
    });

  it('is ready with no parent origin', () => {
    expect(hosted()).toEqual({
      kind: 'ready',
      key: 'pk_test_1',
      clientSecret: SECRET,
      parentOrigin: null,
    });
  });

  it('refuses to render inside a frame, as the second lock behind frame-ancestors none', () => {
    expect(hosted({ framed: true })).toEqual({ kind: 'refused' });
  });

  it('names the missing half of a broken link', () => {
    expect(hosted({ search: '' })).toEqual({ kind: 'error', code: 'error.missing_key' });
    expect(hosted({ hash: '' })).toEqual({ kind: 'error', code: 'error.missing_secret' });
  });
});

describe('the return page', () => {
  it('takes the key from the query when the link carries one', () => {
    expect(decideReturnEntry({ search: '?t=tok&key=pk_query', rememberedKey: 'pk_stored' })).toEqual(
      { kind: 'ready', key: 'pk_query', returnToken: 'tok' },
    );
  });

  it('falls back to the key this tab remembered, since the plan’s return URL carries none', () => {
    expect(decideReturnEntry({ search: '?t=tok', rememberedKey: 'pk_stored' })).toEqual({
      kind: 'ready',
      key: 'pk_stored',
      returnToken: 'tok',
    });
  });

  it('names what is missing rather than rendering an outcome it did not read', () => {
    expect(decideReturnEntry({ search: '?t=tok', rememberedKey: null })).toEqual({
      kind: 'error',
      code: 'error.missing_key',
    });
    expect(decideReturnEntry({ search: '?key=pk', rememberedKey: null })).toEqual({
      kind: 'error',
      code: 'error.missing_return_token',
    });
  });
});
