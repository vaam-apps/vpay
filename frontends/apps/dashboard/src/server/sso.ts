/**
 * The browser-facing half of the federated (OIDC) staff sign-in.
 *
 * vpay stays the OpenID provider for `/dash/v1`; only the human step goes to
 * an external identity provider. This app is the broker's front door:
 *
 * ```
 * POST /api/auth/sso/start      -> vpay /staff/sso/start    -> 303 to the IdP
 * GET  /api/auth/sso/callback   -> vpay /staff/sso/callback -> 303 to /payments
 * ```
 *
 * # One cookie slot, two kinds of token
 *
 * `start` stores the **pending-flow token** vpay returns in the ordinary
 * session cookie, for ten minutes. It is not a session — only the callback
 * accepts it — and the callback **replaces** it with the token vpay answers
 * with, never reuses it: after `signed_in` the new token is the authenticated
 * session (twelve hours), after `reauthenticate` it is the next pending token
 * (ten minutes) for the single `prompt=login` retry vpay allows.
 *
 * # What a browser, a log and a URL never see
 *
 * - **No token in a URL, ever.** The pending token travels in the cookie and
 *   in the `X-Vpay-Staff-Session` header of the one server-side call.
 * - **The IdP's `code` and `state` go to exactly one place**: the body of the
 *   one vpay call. They are not rendered, logged, put in a `Location`, or
 *   echoed into a failure. Every failure answers the same redirect to
 *   `/login?sso=failed`, which carries no reason — vpay's own log has it.
 * - **`error` / `error_description` from the IdP are not read for display**,
 *   and an `error` parameter stops the callback before vpay is called.
 * - Every redirect carries `Cache-Control: no-store` (the URL of a callback
 *   holds a one-time code) and `Referrer-Policy: no-referrer` (the page the
 *   browser lands on must not receive that URL in a `Referer`).
 *
 * # Relative `Location` for in-app redirects
 *
 * `signed-out.ts` states the reason at length: an absolute URL built from
 * `request.url` names the container, not the public host. In-app redirects
 * here are therefore relative. The one absolute `Location` is the identity
 * provider's, validated by {@link safeAuthorizationUrl} first.
 */
import { NextResponse, type NextRequest } from "next/server";

import { dashboardConfig } from "../config/runtime";
import { ssoCallback, ssoStart } from "./api";
import {
  COOKIE_ATTRIBUTES,
  SESSION_COOKIE,
  SESSION_MAX_AGE_SECONDS,
  SSO_PENDING_MAX_AGE_SECONDS,
} from "./cookies";
import { originRefusal } from "./csrf";
import { completeAuthorizationCode } from "./oauth";
import { HOME_PATH, LOGIN_PATH } from "./session";

/** The one place every refusal goes; the query value is the whole message. */
export const SSO_FAILED_PATH = `${LOGIN_PATH}?sso=failed`;

/**
 * `url` as an absolute `http(s)` URL with no embedded credentials, or `null`.
 *
 * What the browser is about to be sent to is decided by an upstream value, so
 * it is checked rather than trusted: a `javascript:` or `data:` URL in a
 * `Location` is not followed by browsers, but a relative reference would
 * silently land back on this origin and a `user:pass@` URL is a phishing
 * shape. `http` is allowed because a local identity provider in a development
 * stack is `http://localhost`; the cookie on this side is `Secure` either way.
 */
export function safeAuthorizationUrl(url: string): string | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
    return null;
  }
  if (parsed.username !== "" || parsed.password !== "") {
    return null;
  }
  return parsed.href;
}

/** A `303` to `location` with the two headers every response here carries. */
function redirectTo(location: string): NextResponse {
  return new NextResponse(null, {
    status: 303,
    headers: {
      location,
      "cache-control": "no-store",
      "referrer-policy": "no-referrer",
    },
  });
}

/** The generic refusal: back to the form, with no reason attached. */
function failed(): NextResponse {
  return redirectTo(SSO_FAILED_PATH);
}

/** The refusal that also forgets whatever token the cookie held. */
function failedAndForgotten(): NextResponse {
  const response = failed();
  // `maxAge: 0` with the SAME attributes the cookie was written with, for the
  // reason `signed-out.ts` gives: a deletion whose attributes differ leaves
  // the original in place.
  response.cookies.set(SESSION_COOKIE, "", { ...COOKIE_ATTRIBUTES, maxAge: 0 });
  return response;
}

/** `response` with the session cookie set to `token` for `maxAge` seconds. */
function withSessionCookie(
  response: NextResponse,
  token: string,
  maxAge: number,
): NextResponse {
  response.cookies.set(SESSION_COOKIE, token, { ...COOKIE_ATTRIBUTES, maxAge });
  return response;
}

/**
 * `POST /api/auth/sso/start`.
 *
 * The origin check is the one the server actions run, first, before vpay is
 * asked for anything: a cross-site form post must not be able to spend a
 * rate-limit unit or overwrite this browser's session cookie. A refusal
 * answers the same generic redirect as every other failure.
 *
 * A failure here does **not** clear the cookie: nothing has been replaced
 * yet, and a person who already holds a working session and pressed the
 * button by mistake should keep it.
 */
export async function handleSsoStart(): Promise<NextResponse> {
  if ((await originRefusal()) !== null) {
    return failed();
  }
  const { config } = dashboardConfig();
  if (config === null) {
    return failed();
  }

  const started = await ssoStart(config.apiBaseUrl);
  if (!started.ok) {
    return failed();
  }
  const target = safeAuthorizationUrl(started.value.authorization_url);
  if (target === null) {
    return failed();
  }

  return withSessionCookie(
    redirectTo(target),
    started.value.session,
    SSO_PENDING_MAX_AGE_SECONDS,
  );
}

/** One query parameter that must appear exactly once and not be empty. */
function single(params: URLSearchParams, name: string): string | null {
  const values = params.getAll(name);
  const [only] = values;
  return values.length === 1 && only !== undefined && only.length > 0
    ? only
    : null;
}

/**
 * `GET /api/auth/sso/callback?code=…&state=…`.
 *
 * Called by the browser after the identity provider redirects to the
 * configured `redirect_uri`. No origin check is possible or wanted — it is a
 * cross-site top-level navigation by construction. What this app relies on is
 * vpay's: the callback is refused unless the pending-flow token and the
 * `state` belong together (the backend's contract; not something this file
 * can check), and the cookie carrying that token is `SameSite=Lax`, which a
 * top-level `GET` sends.
 *
 * Every branch that is not a success clears the cookie and answers
 * `/login?sso=failed`.
 */
export async function handleSsoCallback(
  request: NextRequest,
): Promise<NextResponse> {
  const { config } = dashboardConfig();
  if (config === null) {
    return failedAndForgotten();
  }

  const params = request.nextUrl.searchParams;
  // An `error` from the IdP ends it here: vpay is not called, and the
  // description is not read.
  if (params.has("error")) {
    return failedAndForgotten();
  }
  const code = single(params, "code");
  const state = single(params, "state");
  const pending = request.cookies.get(SESSION_COOKIE)?.value;
  if (
    code === null ||
    state === null ||
    pending === undefined ||
    pending.length === 0
  ) {
    return failedAndForgotten();
  }

  const answered = await ssoCallback(config.apiBaseUrl, pending, code, state);
  if (!answered.ok) {
    return failedAndForgotten();
  }

  if (answered.value.outcome === "reauthenticate") {
    const target = safeAuthorizationUrl(answered.value.authorization_url);
    if (target === null) {
      return failedAndForgotten();
    }
    return withSessionCookie(
      redirectTo(target),
      answered.value.session,
      SSO_PENDING_MAX_AGE_SECONDS,
    );
  }

  // `signed_in`: the session is authenticated and the pending token is gone.
  const session = answered.value.session;
  // The same function the TOTP success path runs. Its result is not needed
  // here: vpay writes the access token onto the session row, and every render
  // reads it back.
  //
  // A failure of the code leg is **not** a failed sign-in. The session is
  // authenticated, and `requireStaff` — which the landing page runs first —
  // already owns every outcome of a mint that did not happen: it sends a
  // session still carrying the printed password to the password form (which
  // this response cannot know about; `signed_in` carries no flag), retries
  // the mint, and shows the outage page or signs out on a refusal. Clearing
  // the cookie here would turn "vpay was briefly unreachable" into a sign-in
  // that has to be started again at the identity provider.
  await completeAuthorizationCode(config, session);
  return withSessionCookie(
    redirectTo(HOME_PATH),
    session,
    SESSION_MAX_AGE_SECONDS,
  );
}
