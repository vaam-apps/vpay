/**
 * Reading this page's own URL.
 *
 * D6: a secret rides in the **fragment**, never the query string. A
 * fragment is not sent to a server, is not written to an access log, and
 * does not reach a `Referer` — which is why the hosted URL vpay mints is
 * `{base}/c/{cs_id}#{client_secret}` and the embedded iframe's `src` is
 * `{base}/e/{cs_id}?key={pk}#{client_secret}`.
 *
 * Two consequences this module enforces:
 *
 * 1. **A `client_secret` in the query string is ignored, not used.** Reading
 *    one would make the safe shape and the unsafe shape work equally well,
 *    and the unsafe one is what a copy-pasted URL turns into.
 * 2. **The publishable key is read from the query, and the fragment is
 *    accepted as a fallback.** It is not a secret (browser-checkout D1: it
 *    is rendered into a merchant's public page by construction), so the
 *    query is the right place for it.
 *
 * The fragment is accepted in two shapes: the bare secret the plan writes
 * (`#cs_…_secret_…`) and a `key=value&key=value` form, so a deployment that
 * wants both values out of the log-free half of the URL can have that
 * without a second parser.
 */

export interface PageCredentials {
  /** The publishable key, or `null` when the link carries none. */
  key: string | null;
  /** The session's `client_secret`, or `null`. Only ever read from the fragment. */
  clientSecret: string | null;
}

/** The separator `vpay_core::ids::client_secret` joins an id and its suffix with. */
const SECRET_SEPARATOR = '_secret_';

function stripLeadingHash(hash: string): string {
  return hash.startsWith('#') ? hash.slice(1) : hash;
}

/**
 * Parses `location.search` and `location.hash`.
 *
 * Takes the two strings rather than reading `location` itself so that it is
 * a pure function: every case below is a test, and none of them needs a
 * browser.
 */
export function parsePageCredentials(search: string, hash: string): PageCredentials {
  const query = new URLSearchParams(search.startsWith('?') ? search.slice(1) : search);
  const fragment = stripLeadingHash(hash);

  let clientSecret: string | null = null;
  let fragmentKey: string | null = null;

  if (fragment.length > 0) {
    if (fragment.includes('=')) {
      const parameters = new URLSearchParams(fragment);
      clientSecret = parameters.get('client_secret');
      fragmentKey = parameters.get('key');
    } else {
      // The plan's shape: the fragment *is* the secret.
      clientSecret = decodeURIComponent(fragment);
    }
  }

  if (clientSecret !== null && !clientSecret.includes(SECRET_SEPARATOR)) {
    // Not a vpay credential. Refused rather than sent to the API, and not
    // echoed anywhere — it is whatever was in the address bar.
    clientSecret = null;
  }

  const key = query.get('key') ?? fragmentKey;
  return {
    key: key !== null && key.length > 0 ? key : null,
    clientSecret: clientSecret !== null && clientSecret.length > 0 ? clientSecret : null,
  };
}

/** The return page's `?t=` token. */
export function parseReturnToken(search: string): string | null {
  const query = new URLSearchParams(search.startsWith('?') ? search.slice(1) : search);
  const token = query.get('t');
  return token !== null && token.length > 0 ? token : null;
}

/**
 * Where the return page remembers the publishable key.
 *
 * The plan's return URL is `{base}/c/{cs_id}/return?t={return_token}` and
 * carries no `key`, but `GET /v1/browser/checkout/sessions/{id}/return`
 * needs one. Three sources, in order: the query string (`?key=`, the shape
 * this lane asks lane 1 to mint — see `docs/plans/step9-notes/lane-3.md`),
 * then this `sessionStorage` entry, written by the checkout page in the same
 * tab before it sends the payer to the rail, then nothing — in which case
 * the page says which parameter is missing rather than rendering an outcome
 * it did not read.
 *
 * Only the publishable key is stored, never a secret: `sessionStorage`
 * outlives the payment, and D6's whole point is that the credential lives in
 * a fragment that does not.
 */
export const PUBLISHABLE_KEY_STORAGE_PREFIX = 'vpay.checkout.key.';

export function rememberPublishableKey(
  storage: Storage | null | undefined,
  sessionId: string,
  key: string,
): void {
  try {
    storage?.setItem(`${PUBLISHABLE_KEY_STORAGE_PREFIX}${sessionId}`, key);
  } catch {
    // A browser with storage disabled, or a partitioned third-party
    // context. The return page falls back to `?key=`; there is nothing to
    // report to the payer here.
  }
}

export function recallPublishableKey(
  storage: Storage | null | undefined,
  sessionId: string,
): string | null {
  try {
    const value = storage?.getItem(`${PUBLISHABLE_KEY_STORAGE_PREFIX}${sessionId}`) ?? null;
    return value !== null && value.length > 0 ? value : null;
  } catch {
    return null;
  }
}
