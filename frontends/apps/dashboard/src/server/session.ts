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
 * them is the same — clear the cookie and show the form — and clearing it is
 * what stops the loop: `/login` itself never calls into this file, so a
 * browser holding a dead cookie lands on a form rather than bouncing between
 * two redirects.
 */
import { cookies } from 'next/headers';
import { redirect } from 'next/navigation';

import { dashboardConfig } from '../config/runtime';
import type { DashboardConfig } from '../config/settings';
import { getJson, type ApiFailure, type SessionResponse } from './api';
import { COOKIE_ATTRIBUTES, SESSION_COOKIE } from './cookies';
import { gateFor } from './gate';
import { completeAuthorizationCode } from './oauth';

/** Where an unauthenticated visitor is sent. */
export const LOGIN_PATH = '/login';
/** Where a password that was accepted goes next: the second factor. */
export const TOTP_PATH = '/login/totp';
/** Where a session still carrying the printed password is sent. */
export const PASSWORD_PATH = '/login/password';
/** The first page a signed-in staff member sees. */
export const HOME_PATH = '/payments';

/** Everything a protected page needs, once the gate has let it through. */
export interface StaffContext {
  /** Who is signed in, and which tenant they may read. */
  readonly session: SessionResponse;
  /** The `/dash/v1` bearer token, read back from the session row. */
  readonly accessToken: string;
  /** The settings this container booted with. */
  readonly config: DashboardConfig;
}

/** The session token this browser is holding, if any. */
export async function sessionToken(): Promise<string | null> {
  const store = await cookies();
  const value = store.get(SESSION_COOKIE)?.value;
  return typeof value === 'string' && value.length > 0 ? value : null;
}

/** Sets the session cookie with the attributes ADR-0017 decision 2 requires. */
export async function setSessionCookie(token: string, maxAge: number): Promise<void> {
  const store = await cookies();
  store.set(SESSION_COOKIE, token, { ...COOKIE_ATTRIBUTES, maxAge });
}

/** Removes the session cookie. */
export async function clearSessionCookie(): Promise<void> {
  const store = await cookies();
  store.set(SESSION_COOKIE, '', { ...COOKIE_ATTRIBUTES, maxAge: 0 });
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
  const result = await getJson<SessionResponse>(config.apiBaseUrl, '/dash/v1/staff/session', {
    sessionToken: token,
  });
  return result.ok
    ? { session: result.value, failure: null }
    : { session: null, failure: result.failure };
}

/**
 * The gate every protected page opens with. Redirects rather than returning
 * when the answer is "not here".
 *
 * The `needs-token` branch runs the authorization-code leg and then reads the
 * session **again** rather than using the token the exchange returned. Both
 * carry the same string, and reading it back is what proves the row was
 * written — the one place `/dash/v1`'s token comes from for the rest of this
 * session is `staff_sessions.access_token`, and a token this app kept in a
 * local variable would still work for a render after the row was deleted.
 */
export async function requireStaff(): Promise<StaffContext> {
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
    await clearSessionCookie();
    redirect(LOGIN_PATH);
  }

  const gate = gateFor(first.session);
  if (gate.kind === 'must-change-password') {
    redirect(PASSWORD_PATH);
  }
  if (gate.kind === 'ready') {
    return { session: gate.session, accessToken: gate.accessToken, config };
  }

  const exchanged = await completeAuthorizationCode(config, token);
  if (!exchanged.ok) {
    await clearSessionCookie();
    redirect(LOGIN_PATH);
  }

  const second = await readSession(config, token);
  const settled = second.session === null ? null : gateFor(second.session);
  if (settled === null || settled.kind !== 'ready') {
    await clearSessionCookie();
    redirect(LOGIN_PATH);
  }
  return { session: settled.session, accessToken: settled.accessToken, config };
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
 * three pages read during their own render does not, and giving it one widens
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
