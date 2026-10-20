/**
 * The two route paths, alone in a file with no imports.
 *
 * `sso.ts` imports `next/server` and the session stack; a client-reachable
 * component that only needs a form `action` must not pull those in.
 */

/** Where the sign-in button posts to. */
export const SSO_START_PATH = "/api/auth/sso/start";
/** The `redirect_uri` path the identity provider sends the browser back to. */
export const SSO_CALLBACK_PATH = "/api/auth/sso/callback";
