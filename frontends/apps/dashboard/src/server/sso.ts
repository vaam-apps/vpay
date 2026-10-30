/**
 * The browser-facing half of the federated (OIDC) staff sign-in (VPA-5b).
 *
 * vpay stays the OpenID provider for `/dash/v1`; only the human step goes to
 * an external identity provider. This app is the broker's front door:
 *
 * ```
 * POST /api/auth/sso/start      -> vpay /staff/sso/start    -> 303 to the IdP
 * GET  /api/auth/sso/callback   -> vpay /staff/sso/callback -> 303 to /
 * ```
 *
 * The routes are two of the three `STAFF_ROUTES` entries VPA-4 serves
 * (`docs/flows/dashboard-auth/sso.md`); the third, `GET /staff/sso`, is read
 * by `/login` to decide whether to draw the button at all.
 *
 * # Two cookies, and neither is the other
 *
 * `start` stores the **pending-flow token** vpay returns in
 * {@link SSO_PENDING_COOKIE}, for ten minutes. It is not a session — only the
 * callback accepts it — and it never touches {@link SESSION_COOKIE}: a person
 * who already holds a working session and presses the button by mistake keeps
 * it. The callback then does one of three things with a **new** token vpay
 * answers with, never the one it was called with:
 *
 * * `signed_in` — the new token *replaces* the session cookie (whatever it
 *   held, so a session planted in this browser before the flow cannot survive
 *   it: no session fixation) and the pending cookie is cleared;
 * * `reauthenticate` — the new token replaces the pending cookie, for the
 *   single `prompt=login` retry vpay allows;
 * * anything else — the pending cookie is cleared and the browser lands on
 *   `/login?sso=failed`.
 *
 * `signed_in` does **not** run the authorization-code leg here. The session is
 * authenticated and the next request is `/`, whose gate (`session.ts`,
 * `gateFor`) already owns every outcome of a mint — the printed password still
 * in use, an outage, a refusal — so a vpay that is briefly unreachable costs
 * the landing page a retry and not the person a second trip to the provider.
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
 * - **No `return_to`.** Nothing in either route takes a destination from the
 *   request: a success lands on {@link HOME_PATH}, a failure on
 *   {@link SSO_FAILED_PATH}, and the only absolute `Location` is the identity
 *   provider's, which {@link safeAuthorizationUrl} admits only if it names the
 *   configured issuer's origin.
 * - Every response carries `Cache-Control: no-store` (the URL of a callback
 *   holds a one-time code) and `Referrer-Policy: no-referrer` (the page the
 *   browser lands on must not receive that URL in a `Referer`).
 *
 * # Relative `Location` for in-app redirects
 *
 * `signed-out.ts` states the reason at length: an absolute URL built from
 * `request.url` names the container, not the public host. In-app redirects
 * here are therefore relative.
 *
 * # The callback path is a contract with vpay's configuration
 *
 * The path of `staff_auth.sso.redirect_uri` must equal {@link SSO_CALLBACK_PATH}
 * (`sso-paths.ts`). The identity provider redirects there and nothing else
 * answers.
 */
import { NextResponse, type NextRequest } from "next/server";

import { dashboardConfig } from "../config/runtime";
import { ssoCallback, ssoStart } from "./api";
import {
  COOKIE_ATTRIBUTES,
  SESSION_COOKIE,
  SESSION_MAX_AGE_SECONDS,
  SSO_PENDING_COOKIE,
  SSO_PENDING_MAX_AGE_SECONDS,
} from "./cookies";
import { originRefusal } from "./csrf";
import { HOME_PATH, LOGIN_PATH } from "./session";

/** The one place every refusal goes; the query value is the whole message. */
export const SSO_FAILED_PATH = `${LOGIN_PATH}?sso=failed`;

/** Hosts a development identity provider may be reached on over plain `http`. */
const LOOPBACK_HOSTS: ReadonlySet<string> = new Set([
  "localhost",
  "127.0.0.1",
  "[::1]",
]);

/**
 * `url` normalised, if it is somewhere this deployment's staff may be sent;
 * otherwise `null`.
 *
 * What the browser is about to be sent to is decided by an upstream value, so
 * it is checked rather than trusted. All of these must hold:
 *
 * * absolute and **`https`** — `http` only for a loopback host
 *   (`localhost`, `127.0.0.1`, `[::1]`), because a development identity
 *   provider in a compose stack is `http://localhost`, and the cookie on this
 *   side is `Secure` either way. A non-loopback `http` provider is refused
 *   here even though vpay accepts it outside livemode;
 * * the **origin** is `issuerOrigin` (`VPAY_DASHBOARD_SSO_ISSUER` reduced to
 *   its origin), byte for byte. A provider whose authorization endpoint lives
 *   on a different origin than its issuer is refused until that is a
 *   decision somebody takes;
 * * no `user:pass@` (a phishing shape) and no `#fragment` (no part of an
 *   authorization request is a fragment, and one in a `Location` is where a
 *   token would be returned in the implicit flow this never uses).
 *
 * A relative reference, `javascript:` or `data:` fails the first rule; the
 * second also rejects a look-alike host and a `//evil.example` scheme-relative
 * URL, which `new URL` rejects without a base.
 *
 * Returns the parsed `href`, so what is sent is the canonical form.
 */
export function safeAuthorizationUrl(
  url: string,
  issuerOrigin: string | null,
): string | null {
  if (issuerOrigin === null) {
    return null;
  }
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  const secure =
    parsed.protocol === "https:" ||
    (parsed.protocol === "http:" && LOOPBACK_HOSTS.has(parsed.hostname));
  if (!secure) {
    return null;
  }
  if (parsed.origin !== issuerOrigin) {
    return null;
  }
  if (parsed.username !== "" || parsed.password !== "") {
    return null;
  }
  // `hash` is empty for a bare trailing `#` too, which the href would keep.
  if (parsed.hash !== "" || parsed.href.includes("#")) {
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

/** The generic refusal: back to the sign-in, with no reason attached. */
function failed(): NextResponse {
  return redirectTo(SSO_FAILED_PATH);
}

/**
 * The refusal that also forgets the pending token.
 *
 * `maxAge: 0` with the SAME attributes the cookie was written with, for the
 * reason `signed-out.ts` gives: a deletion whose attributes differ leaves the
 * original in place. The **session** cookie is left alone — a callback that
 * fails has replaced nothing, and a stray cross-site `GET` to this route must
 * not be able to sign a staff member out.
 */
function failedAndForgotten(): NextResponse {
  const response = failed();
  forgetPending(response);
  return response;
}

function forgetPending(response: NextResponse): void {
  response.cookies.set(SSO_PENDING_COOKIE, "", {
    ...COOKIE_ATTRIBUTES,
    maxAge: 0,
  });
}

/** `response` with the pending cookie set to `token` for ten minutes. */
function withPendingCookie(
  response: NextResponse,
  token: string,
): NextResponse {
  response.cookies.set(SSO_PENDING_COOKIE, token, {
    ...COOKIE_ATTRIBUTES,
    maxAge: SSO_PENDING_MAX_AGE_SECONDS,
  });
  return response;
}

/**
 * `POST /api/auth/sso/start`.
 *
 * The origin check is the one the server actions run, first, before vpay is
 * asked for anything: a cross-site form post must not be able to spend a
 * rate-limit unit or overwrite this browser's pending cookie. A refusal
 * answers the same generic redirect as every other failure.
 *
 * A failure here sets no cookie and clears none: nothing has been replaced.
 */
export async function handleSsoStart(): Promise<NextResponse> {
  if ((await originRefusal()) !== null) {
    return failed();
  }
  const { config, ssoIssuerOrigin } = dashboardConfig();
  // No `config` is no sign-in at all, and no issuer origin is a redirect that
  // cannot be checked — neither asks vpay for a flow nobody could finish.
  if (config === null || ssoIssuerOrigin === null) {
    return failed();
  }

  const started = await ssoStart(config.apiBaseUrl);
  if (!started.ok) {
    return failed();
  }
  const target = safeAuthorizationUrl(
    started.value.authorization_url,
    ssoIssuerOrigin,
  );
  if (target === null) {
    return failed();
  }

  return withPendingCookie(redirectTo(target), started.value.session);
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
 * Every branch that is not a success clears the pending cookie and answers
 * `/login?sso=failed`.
 */
export async function handleSsoCallback(
  request: NextRequest,
): Promise<NextResponse> {
  const { config, ssoIssuerOrigin } = dashboardConfig();
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
  const pending = request.cookies.get(SSO_PENDING_COOKIE)?.value;
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
    const target = safeAuthorizationUrl(
      answered.value.authorization_url,
      ssoIssuerOrigin,
    );
    if (target === null) {
      return failedAndForgotten();
    }
    return withPendingCookie(redirectTo(target), answered.value.session);
  }

  // `signed_in`: the session vpay minted is authenticated and the pending
  // token is spent. Replacing the session cookie outright is what makes a
  // session planted before the flow worthless; the gate on `/` mints.
  const response = redirectTo(HOME_PATH);
  response.cookies.set(SESSION_COOKIE, answered.value.session, {
    ...COOKIE_ATTRIBUTES,
    maxAge: SESSION_MAX_AGE_SECONDS,
  });
  forgetPending(response);
  return response;
}
