/**
 * D6 on the page's own URL: a secret is read from the fragment and never
 * from the query string.
 */
import { describe, expect, it } from 'vitest';

import {
  PUBLISHABLE_KEY_STORAGE_PREFIX,
  parsePageCredentials,
  parseReturnToken,
  recallPublishableKey,
  rememberPublishableKey,
} from './link';

const SECRET = 'cs_test_1_secret_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

describe('parsePageCredentials', () => {
  it("reads the plan's shape: key in the query, bare secret in the fragment", () => {
    expect(parsePageCredentials('?key=pk_test_1', `#${SECRET}`)).toEqual({
      key: 'pk_test_1',
      clientSecret: SECRET,
    });
  });

  it('also reads a key=value fragment, so both values can live out of the logged half', () => {
    expect(
      parsePageCredentials('', `#client_secret=${encodeURIComponent(SECRET)}&key=pk_test_1`),
    ).toEqual({ key: 'pk_test_1', clientSecret: SECRET });
  });

  it('IGNORES a client_secret in the query string', () => {
    // The whole of D6. A page that read it here would make the unsafe URL
    // shape work exactly as well as the safe one.
    expect(parsePageCredentials(`?key=pk_test_1&client_secret=${SECRET}`, '')).toEqual({
      key: 'pk_test_1',
      clientSecret: null,
    });
  });

  it('refuses a fragment that is not a vpay credential', () => {
    expect(parsePageCredentials('?key=pk_test_1', '#not-a-secret').clientSecret).toBeNull();
    expect(parsePageCredentials('?key=pk_test_1', '#').clientSecret).toBeNull();
  });

  it('is null for a missing key rather than inventing one', () => {
    expect(parsePageCredentials('', `#${SECRET}`).key).toBeNull();
    expect(parsePageCredentials('?key=', `#${SECRET}`).key).toBeNull();
  });

  it('prefers the query key over a fragment key, since that is where the plan puts it', () => {
    expect(parsePageCredentials('?key=pk_query', `#client_secret=${SECRET}&key=pk_frag`).key).toBe(
      'pk_query',
    );
  });
});

describe('parseReturnToken', () => {
  it('reads ?t=', () => {
    expect(parseReturnToken('?t=abc')).toBe('abc');
  });

  it('is null when absent or blank', () => {
    expect(parseReturnToken('')).toBeNull();
    expect(parseReturnToken('?t=')).toBeNull();
  });
});

describe('the remembered publishable key', () => {
  function memoryStorage(): Storage {
    const map = new Map<string, string>();
    return {
      get length() {
        return map.size;
      },
      clear: () => map.clear(),
      getItem: (k: string) => map.get(k) ?? null,
      key: (i: number) => Array.from(map.keys())[i] ?? null,
      removeItem: (k: string) => map.delete(k),
      setItem: (k: string, v: string) => void map.set(k, v),
    };
  }

  it('round-trips per session id', () => {
    const storage = memoryStorage();
    rememberPublishableKey(storage, 'cs_1', 'pk_test_1');
    expect(recallPublishableKey(storage, 'cs_1')).toBe('pk_test_1');
    expect(recallPublishableKey(storage, 'cs_2')).toBeNull();
    expect(storage.getItem(`${PUBLISHABLE_KEY_STORAGE_PREFIX}cs_1`)).toBe('pk_test_1');
  });

  it('survives a storage that throws, rather than taking the page down with it', () => {
    const hostile = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    } as unknown as Storage;
    expect(() => rememberPublishableKey(hostile, 'cs_1', 'pk')).not.toThrow();
    expect(recallPublishableKey(hostile, 'cs_1')).toBeNull();
  });

  it('is given no storage at all without complaint', () => {
    expect(() => rememberPublishableKey(null, 'cs_1', 'pk')).not.toThrow();
    expect(recallPublishableKey(undefined, 'cs_1')).toBeNull();
  });
});
