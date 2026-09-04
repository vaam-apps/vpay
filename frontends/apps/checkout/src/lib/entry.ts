/**
 * What a page must decide before it may read anything: does it have a
 * credential, and is it allowed to be where it is?
 *
 * A pure function so that every refusal is a test rather than a branch
 * inside a `useEffect` nobody can reach twice. The order matters and is the
 * point: the **embedding check runs first**, before the credential is even
 * looked at, so a page framed by an origin the merchant never registered
 * refuses without touching the API — and therefore without a hostile framer
 * learning whether the session id in the URL exists.
 */
import { resolveParentOrigin } from './origins';
import { parsePageCredentials } from './link';
import type { CheckoutErrorCode } from './types';

export type EntryDecision =
  | { kind: 'ready'; key: string; clientSecret: string; parentOrigin: string | null }
  | { kind: 'refused' }
  | { kind: 'error'; code: CheckoutErrorCode };

export interface EntryInput {
  /** `hosted` for `/c/{id}`, `embedded` for `/e/{id}`. */
  mode: 'hosted' | 'embedded';
  /** `location.search`. */
  search: string;
  /** `location.hash`. */
  hash: string;
  /** `document.referrer`. Only consulted for an embedded page. */
  referrer: string | null;
  /** The origins the server resolved for this publishable key. Empty means "none". */
  allowedOrigins: readonly string[];
  /** `window.parent !== window`. */
  framed: boolean;
}

export function decideEntry(input: EntryInput): EntryDecision {
  let parentOrigin: string | null = null;

  if (input.mode === 'embedded') {
    if (!input.framed) {
      // `/e/{id}` opened top-level. Nothing here is unsafe, but the page
      // has no parent to report to and the merchant's integration is
      // broken; saying so beats rendering a payment form whose completion
      // nobody will hear about.
      return { kind: 'refused' };
    }
    parentOrigin = resolveParentOrigin(input.referrer, input.allowedOrigins);
    if (parentOrigin === null) {
      return { kind: 'refused' };
    }
  } else if (input.framed) {
    // `/c/{id}` inside a frame. The CSP already said `frame-ancestors
    // 'none'`, so a browser that honoured it never got here; this is the
    // second lock for one that did not.
    return { kind: 'refused' };
  }

  const credentials = parsePageCredentials(input.search, input.hash);
  if (credentials.key === null) {
    return { kind: 'error', code: 'error.missing_key' };
  }
  if (credentials.clientSecret === null) {
    return { kind: 'error', code: 'error.missing_secret' };
  }
  return {
    kind: 'ready',
    key: credentials.key,
    clientSecret: credentials.clientSecret,
    parentOrigin,
  };
}

/** The return page's version: a token in the query, and a key from wherever one is. */
export type ReturnEntryDecision =
  | { kind: 'ready'; key: string; returnToken: string }
  | { kind: 'error'; code: CheckoutErrorCode };

export function decideReturnEntry(input: {
  search: string;
  /** `recallPublishableKey`'s answer, or `null`. */
  rememberedKey: string | null;
}): ReturnEntryDecision {
  const query = new URLSearchParams(
    input.search.startsWith('?') ? input.search.slice(1) : input.search,
  );
  const token = query.get('t');
  if (token === null || token.length === 0) {
    return { kind: 'error', code: 'error.missing_return_token' };
  }
  const key = query.get('key') ?? input.rememberedKey;
  if (key === null || key.length === 0) {
    return { kind: 'error', code: 'error.missing_key' };
  }
  return { kind: 'ready', key, returnToken: token };
}
