/**
 * The cookie, the session read, and the redirect a page takes instead of
 * rendering.
 *
 * The Next-bound half of the gate: `gate.ts` decides, this file acts. Split
 * that way because `cookies()` and `redirect()` only work inside a request,
 * and a decision that can only be exercised through a browser is a decision
 * nothing tests.
 *
 * # 401 goes back to the login form, once
 *
 * Every refusal on this path is the same `401` by design
 * (`docs/flows/dashboard-auth.md`, "Every refusal is one answer"): absent,
 * expired, idle, forged, disabled, at the wrong stage. The answer to all of
 * them is the same — forget the cookie and show the form. What stops the loop
 * is that `/login` itself never calls into this file, so a browser holding a
 * dead cookie lands on a form rather than bouncing between two redirects.
 *
 * # A page may not clear a cookie, and finding that out costs a 500
 *
 * `cookies().set(…)` throws outside a Server Action or a Route Handler, and a
 * Server Component that calls it hands the browser Next's **500** rather than
 * the redirect it was about to perform. Until the exp28 review this file did
 * exactly that on every refusal {@link requireStaff} handles: measured against
 * the real stack, a forged cookie, a session signed out elsewhere and a
 * disabled account each answered `500` on `/payments`, and — the cookie never
 * having been cleared — so did every request after it.
 *
 * So the clearing moved to `app/signed-out/route.ts`, which is allowed to do
 * it, and this file redirects there. {@link clearSessionCookie} stays for the
 * server actions, where it is legal and where it is called.
 */
import { cookies } from "next/headers";
import { redirect } from "next/navigation";

import { dashboardConfig } from "../config/runtime";
import type { DashboardConfig } from "../config/settings";
import {
  getJson,
  type ApiFailure,
  type SessionResponse,
  type SessionStageResponse,
} from "./api";
import { COOKIE_ATTRIBUTES, SESSION_COOKIE } from "./cookies";
import { gateFor, refusalFor } from "./gate";
import { completeAuthorizationCode } from "./oauth";

/** Where an unauthenticated visitor is sent. */
export const LOGIN_PATH = "/login";
/** Where a password that was accepted goes next: the second factor. */
export const TOTP_PATH = "/login/totp";
/** Where a session still carrying the printed password is sent. */
export const PASSWORD_PATH = "/login/password";
/** The first page a signed-in staff member sees. */
export const HOME_PATH = "/payments";
/**
 * Where a page sends a browser whose session vpay refused.
 *
 * A Route Handler, because that is the only place in a Next app besides a
 * server action where a cookie may be written — see this module's header. It
 * deletes the session cookie and redirects to {@link LOGIN_PATH}.
 */
export const SIGNED_OUT_PATH = "/signed-out";

/**
 * What {@link requireStaff} answers when it did not redirect.
 *
 * A union rather than a `StaffContext` because a page has **three** possible
 * next moves and only two of them are a redirect: render, go somewhere, or
 * say that vpay could not be reached. The third had no representation until
 * 2026-09-10 and was spelled as the second — see {@link refusalFor}.
 */
export type StaffGate =
  /** Signed in, with a token to read `/dash/v1` with. */
  | { readonly kind: "ready"; readonly staff: StaffContext }
  /**
   * vpay could not answer. **The cookie is untouched**: this browser may
   * still hold a perfectly good session, and the page renders the failure and
   * its request id instead of the data.
   */
  | { readonly kind: "outage"; readonly failure: ApiFailure };

/** Everything a protected page needs, once the gate has let it through. */
export interface StaffContext {
  /** Who is signed in, and which tenant they may read. */
  readonly session: SessionResponse;
  /** The `/dash/v1` bearer token, read back from the session row. */
  readonly accessToken: string;
  /**
   * The staff session token this browser presented.
   *
   * Carried so a page can mint a **replacement** access token when the one
   * above has expired — `server/dash-read.ts` and the fifteen minutes it
   * describes. Server-side only, like everything else here: it is the cookie
   * value, and nothing renders it.
   */
  readonly sessionToken: string;
  /** The settings this container booted with. */
  readonly config: DashboardConfig;
}

/** The session token this browser is holding, if any. */
export async function sessionToken(): Promise<string | null> {
  const store = await cookies();
  const value = store.get(SESSION_COOKIE)?.value;
  return typeof value === "string" && value.length > 0 ? value : null;
}

/** Sets the session cookie with the attributes ADR-0017 decision 2 requires. */
export async function setSessionCookie(
  token: string,
  maxAge: number,
): Promise<void> {
  const store = await cookies();
  store.set(SESSION_COOKIE, token, { ...COOKIE_ATTRIBUTES, maxAge });
}

/**
 * Removes the session cookie.
 *
 * **Callable only from a server action.** `cookies().set` throws during a
 * page render (this module's header), so a page that has decided a session is
 * dead redirects to {@link SIGNED_OUT_PATH} instead of calling this.
 */
export async function clearSessionCookie(): Promise<void> {
  const store = await cookies();
  store.set(SESSION_COOKIE, "", { ...COOKIE_ATTRIBUTES, maxAge: 0 });
}

/**
 * Reads the session, or `null` with the refusal.
 *
 * Does not redirect: the login pages call this to find out whether somebody
 * is already signed in, and a redirect there would be the loop this module's
 * header is about.
 */
export async function readSession(
  config: DashboardConfig,
  token: string,
): Promise<{ session: SessionResponse | null; failure: ApiFailure | null }> {
  const result = await getJson<SessionResponse>(
    config.apiBaseUrl,
    "/dash/v1/staff/session",
    {
      sessionToken: token,
    },
  );
  return result.ok
    ? { session: result.value, failure: null }
    : { session: null, failure: result.failure };
}

/**
 * Reads how far a session has got, or `null` with the refusal.
 *
 * The read `/login/totp` takes on every render. {@link readSession} cannot
 * serve it: `GET /dash/v1/staff/session` is refused for a session that has not
 * presented a code, with the same `401` it answers for a session that is over,
 * so the page could not tell a typo from a sign-out — see
 * {@link import('./gate').totpGateFor}.
 *
 * Like {@link readSession} it does not redirect: deciding is
 * `totpGateFor`'s job and acting is the page's.
 */
export async function readSessionStage(
  config: DashboardConfig,
  token: string,
): Promise<{ stage: SessionStageResponse | null; failure: ApiFailure | null }> {
  const result = await getJson<SessionStageResponse>(
    config.apiBaseUrl,
    "/dash/v1/staff/session/stage",
    { sessionToken: token },
  );
  return result.ok
    ? { stage: result.value, failure: null }
    : { stage: null, failure: result.failure };
}

/**
 * The gate every protected page opens with. Redirects rather than returning
 * when the answer is "not here".
 *
 * # The `needs-token` branch uses the token the exchange returned
 *
 * It used to read the session a **second** time instead, on the argument that
 * reading it back proves the row was written. That second read never reached
 * vpay: React memoises `fetch` for identical `GET`s within one render, so it
 * answered with the *first* read's body — the one taken before the exchange,
 * with no token on it. The branch therefore decided the session was unusable,
 * tried to clear the cookie mid-render, and answered `500`.
 *
 * Measured on the real stack (exp28 review, finding F2): three consecutive
 * first visits to `/payments` by a freshly authenticated session answered
 * `500`, the server log showed the code and the token both issued, and
 * `staff_sessions.last_seen_at` afterwards was *earlier* than the token — no
 * `/staff/session` request was served after the mint at all.
 *
 * Using the exchanged token costs nothing the re-read bought. Sign-out is
 * still a revocation: this token is used for the remainder of the one request
 * that minted it, and **every later render reads the row** — which is the
 * property ADR-0017 decision 2 rests on. A token cached across requests would
 * break it; a token used by the request that created it cannot.
 */
export async function requireStaff(): Promise<StaffGate> {
  const { config } = dashboardConfig();
  if (config === null) {
    // Nothing can be read without a registration; `/login` is where the
    // reason is rendered.
    redirect(LOGIN_PATH);
  }

  const token = await sessionToken();
  if (token === null) {
    redirect(LOGIN_PATH);
  }

  const first = await readSession(config, token);
  if (first.session === null) {
    // `refusalFor` and not `if (session === null)`. This branch used to send
    // a browser to `/signed-out` for every failure, and `api.ts` turns a
    // rejected `fetch` into a failure rather than throwing — so a vpay that
    // was restarting signed every staff member out and told them nothing
    // (issue #88 item 2).
    if (first.failure !== null && refusalFor(first.failure) === "outage") {
      return { kind: "outage", failure: first.failure };
    }
    redirect(SIGNED_OUT_PATH);
  }

  const gate = gateFor(first.session, Date.now());
  if (gate.kind === "must-change-password") {
    redirect(PASSWORD_PATH);
  }
  if (gate.kind === "ready") {
    return {
      kind: "ready",
      staff: {
        session: gate.session,
        accessToken: gate.accessToken,
        sessionToken: token,
        config,
      },
    };
  }

  const exchanged = await completeAuthorizationCode(config, token);
  if (!exchanged.ok) {
    // The same rule on the mint. `/oauth/authorize` refuses a session it will
    // not issue for with a `401`, so anything else here — a `502` from a
    // proxy, a connection that was reset — is vpay being unreachable while
    // this browser's session is very likely still good.
    if (refusalFor(exchanged.failure) === "outage") {
      // A **stale** token is not a missing one, and this is the whole reason
      // `gateFor` tells them apart. The one in hand is still inside its TTL —
      // that is what the margin bought — so a vpay that could not be reached
      // for the re-mint costs nothing at all here, where it would otherwise
      // replace a page that could have rendered with an error box. If it does
      // expire before vpay comes back, `dash-read.ts` answers the `401` with
      // its own attempt and then renders the refusal.
      if (gate.kind === "stale-token") {
        return {
          kind: "ready",
          staff: {
            session: gate.session,
            accessToken: gate.accessToken,
            sessionToken: token,
            config,
          },
        };
      }
      return { kind: "outage", failure: exchanged.failure };
    }
    // A `401` is NOT fallen back on, deliberately, stale token or not: it
    // means `/oauth/authorize` refused this session on this request — signed
    // out elsewhere, the account disabled, the staff member moved to another
    // merchant — and reading on with the token that refusal has just
    // invalidated is the fifteen-minute hole the exp24 review's finding F1
    // closed one layer down.
    redirect(SIGNED_OUT_PATH);
  }
  return {
    kind: "ready",
    staff: {
      session: gate.session,
      accessToken: exchanged.value.access_token,
      sessionToken: token,
      config,
    },
  };
}

/**
 * Whether this browser already holds a usable session, for the login pages.
 *
 * Answers a plain boolean rather than redirecting, because `/login` is where a
 * dead cookie has to be *survivable* — see this module's header.
 *
 * **Here rather than in `actions.ts`, and that is a boundary rather than
 * tidiness.** Every export of a `'use server'` file is a callable endpoint:
 * Next registers an action id for it and a `POST` from anywhere reaches it.
 * The four things a staff member can *do* have to be endpoints; a predicate
 * two pages read during their own render does not, and giving it one widens
 * the app's reachable surface for nothing.
 */
export async function alreadySignedIn(): Promise<boolean> {
  const { config } = dashboardConfig();
  if (config === null) {
    return false;
  }
  const token = await sessionToken();
  if (token === null) {
    return false;
  }
  const { session } = await readSession(config, token);
  return session !== null && !session.password_change_required;
}
