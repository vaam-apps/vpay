/**
 * The two environment variables this app reads, and why they are read the
 * way they are.
 *
 * - `VPAY_API_URL` — server-side only, used by `middleware.ts` for the
 *   origins lookup. May be an internal address a browser could not reach.
 * - `NEXT_PUBLIC_VPAY_API_URL` — the origin the **browser** calls
 *   `/v1/browser/...` on. Passed to the client component as a prop rather
 *   than read in client code.
 *
 * **Both are read with bracket notation, deliberately.** Next replaces
 * `process.env.NEXT_PUBLIC_FOO` (dot access) with a literal at build time,
 * which would bake one deployment's API URL into the image lane 4 builds
 * and ships to every environment. `process.env['NEXT_PUBLIC_FOO']` is left
 * alone and read at runtime. That is a behaviour of a bundler, so it is
 * asserted rather than assumed: `env.test.ts` sets the variable after
 * import and reads it back.
 *
 * A missing value **throws** rather than falling back to a default. A
 * checkout page pointed at the wrong API is a deployment fault an operator
 * must see in a log and a 500, not a payer-facing "something went wrong".
 */

function required(name: string): string {
  const value = process.env[name];
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new Error(
      `${name} is not set. The vpay checkout app cannot serve a payment page without it.`,
    );
  }
  return value.trim();
}

/** The base URL the payer's browser calls. */
export function browserApiBaseUrl(): string {
  return required('NEXT_PUBLIC_VPAY_API_URL');
}

/** The base URL this server calls for the origins lookup. */
export function serverApiBaseUrl(): string | null {
  const value = process.env['VPAY_API_URL'];
  return typeof value === 'string' && value.trim().length > 0 ? value.trim() : null;
}
