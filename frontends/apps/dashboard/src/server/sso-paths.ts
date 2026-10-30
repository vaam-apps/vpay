/**
 * The two route paths, alone in a file with no imports.
 *
 * `sso.ts` imports `next/server` and the session stack; a client-reachable
 * component that only needs a form `action` must not pull those in.
 */

/** Where the sign-in button posts to. */
export const SSO_START_PATH = "/api/auth/sso/start";

/**
 * The path of the route the identity provider sends the browser back to.
 *
 * **It must equal the path of `staff_auth.sso.redirect_uri`** — the provider
 * redirects to that URL, vpay sends it in the token request, and this is the
 * route that answers. The origin is the deployment's to choose; the path is
 * this app's (`app/api/auth/sso/callback/route.ts`), and
 * `sso-paths.test.ts` fails if the route file and this constant disagree. A
 * `redirect_uri` with any other path is a callback that 404s after a
 * successful sign-in at the provider.
 *
 * This is **not** `VPAY_DASHBOARD_REDIRECT_URI`, which is the dashboard
 * *client's* (`dashboard_client.redirect_uris`) and is never visited.
 */
export const SSO_CALLBACK_PATH = "/api/auth/sso/callback";
