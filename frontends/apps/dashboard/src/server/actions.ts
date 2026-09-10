'use server';

/**
 * The four things a staff member can actually do: sign in, present a code,
 * replace the printed password, sign out.
 *
 * Server actions rather than route handlers so that every form works as a
 * plain `POST` — no JavaScript required to sign in — while the session token,
 * the sealed enrolment secret and the `/dash/v1` access token stay in this
 * process. A `<form action={…}>` submits to the server either way; the
 * client component around it only adds the `pending` state and the error
 * rendering.
 *
 * # One shape for every answer
 *
 * Each action returns {@link FormState}: a message and a request id, or
 * `null` and a redirect. Never a thrown error for a refusal — a staff member
 * who mistyped a password must read a sentence, not Next's error page.
 *
 * # Every one of them opens with an origin check
 *
 * An export of a `'use server'` file is a `POST` endpoint anything on the
 * internet can reach, and the session cookie is `SameSite=Lax` rather than
 * `Strict`, so a top-level form submission from another site carries it.
 * `server/csrf.ts` is the check and it compares the **configured** public
 * origin — never `X-Forwarded-Host`, which is what Next's own check uses and
 * which the caller sets (issue #88 item 4). Adding a fifth action means
 * adding the two lines; there is no wrapper, because a wrapper around a
 * `'use server'` export changes what Next registers.
 */
import { redirect } from 'next/navigation';
import { cookies } from 'next/headers';

import { dashboardConfig } from '../config/runtime';
import type { FormState } from '../form-state';
import { postForm, type ApiFailure, type LoginResponse, type TotpResponse } from './api';
import {
  COOKIE_ATTRIBUTES,
  ENROLMENT_COOKIE,
  ENROLMENT_MAX_AGE_SECONDS,
  SESSION_MAX_AGE_SECONDS,
} from './cookies';
import { originRefusal } from './csrf';
import { decodePendingEnrolment, encodePendingEnrolment } from './enrolment';
import { completeAuthorizationCode } from './oauth';
import {
  clearSessionCookie,
  HOME_PATH,
  LOGIN_PATH,
  PASSWORD_PATH,
  sessionToken,
  setSessionCookie,
  TOTP_PATH,
} from './session';

/** A refusal, in the shape a form renders. */
function shown(failure: ApiFailure): FormState {
  return { error: failure.message, requestId: failure.requestId };
}

/** The sentence a deployment with no registration shows instead of a form. */
const UNCONFIGURED =
  'This deployment has no dashboard client registered, so it can sign nobody in. See the container log.';

/**
 * A single string field out of a `FormData`, trimmed.
 *
 * `FormData.get` answers `File | string | null`; anything but a string is
 * treated as absent rather than stringified, because `[object File]` reaching
 * an argon2 verification is a refusal whose message would be about a
 * password.
 */
function field(form: FormData, name: string): string {
  const value = form.get(name);
  return typeof value === 'string' ? value.trim() : '';
}

/**
 * Step 1 — the password.
 *
 * On success the session cookie is set **before** the redirect, and the
 * sealed enrolment secret with it when this is a first sign-in. Neither
 * value is returned to the caller: `FormState` is what a browser reads.
 */
export async function signIn(_previous: FormState, form: FormData): Promise<FormState> {
  // Issue #88 item 4. First, before anything is read out of the form and
  // before any cookie is touched: a cross-origin caller must not be able to
  // spend a rate-limit unit, let alone a credential.
  const crossOrigin = await originRefusal();
  if (crossOrigin !== null) {
    return crossOrigin;
  }

  const { config } = dashboardConfig();
  if (config === null) {
    return { error: UNCONFIGURED, requestId: null };
  }

  const email = field(form, 'email');
  const password = form.get('password');
  if (email.length === 0 || typeof password !== 'string' || password.length === 0) {
    // Refused here rather than at vpay, which would spend an argon2id
    // verification on an empty string. The message says nothing about which
    // of the two is missing for the same reason vpay's does not.
    return { error: 'Enter your work email and your password.', requestId: null };
  }

  const result = await postForm<LoginResponse>(config.apiBaseUrl, '/dash/v1/staff/login', {
    email,
    // NOT trimmed: a password's leading or trailing space is part of it, and
    // trimming one here would refuse a password this deployment accepted when
    // `staff add` hashed it.
    password,
  });
  if (!result.ok) {
    return shown(result.failure);
  }

  await setSessionCookie(result.value.session, SESSION_MAX_AGE_SECONDS);

  const store = await cookies();
  const sealed = result.value.enrolment;
  const otpauth = result.value.otpauth_uri;
  if (
    result.value.next === 'totp_enrolment' &&
    typeof sealed === 'string' &&
    typeof otpauth === 'string'
  ) {
    store.set(ENROLMENT_COOKIE, encodePendingEnrolment({ sealed, otpauth }), {
      ...COOKIE_ATTRIBUTES,
      maxAge: ENROLMENT_MAX_AGE_SECONDS,
    });
  } else {
    // An account that is already enrolled must not carry a stale blob into
    // `/staff/totp`. It would be ignored there — the stored secret is the
    // only one that may ever authenticate an enrolled account — but a
    // credential blob living in a browser for no reason is one this app can
    // simply not leave behind.
    store.set(ENROLMENT_COOKIE, '', { ...COOKIE_ATTRIBUTES, maxAge: 0 });
  }

  redirect(TOTP_PATH);
}

/**
 * Step 2 — the code, and on a first sign-in the enrolment it completes.
 *
 * The sealed secret is read from the cookie and sent back; vpay opens it,
 * verifies the code against it, and commits the enrolment with a
 * compare-and-swap. A caller that sends one for an already-enrolled account
 * has it ignored, which is why this is safe to send whenever it is present.
 */
export async function submitTotp(_previous: FormState, form: FormData): Promise<FormState> {
  // Issue #88 item 4. First, before anything is read out of the form and
  // before any cookie is touched: a cross-origin caller must not be able to
  // spend a rate-limit unit, let alone a credential.
  const crossOrigin = await originRefusal();
  if (crossOrigin !== null) {
    return crossOrigin;
  }

  const { config } = dashboardConfig();
  if (config === null) {
    return { error: UNCONFIGURED, requestId: null };
  }

  const token = await sessionToken();
  if (token === null) {
    redirect(LOGIN_PATH);
  }

  const code = field(form, 'code');
  if (code.length === 0) {
    return { error: 'Enter the six-digit code from your authenticator app.', requestId: null };
  }

  const store = await cookies();
  const pending = decodePendingEnrolment(store.get(ENROLMENT_COOKIE)?.value);

  const body: Record<string, string> =
    pending === null ? { code } : { code, enrolment: pending.sealed };

  const result = await postForm<TotpResponse>(
    config.apiBaseUrl,
    '/dash/v1/staff/totp',
    body,
    { sessionToken: token },
  );
  if (!result.ok) {
    if (result.failure.status === 401) {
      // The session is gone, expired, idle or at the wrong stage — all one
      // answer. Back to the form with the cookie cleared, which is what stops
      // a loop.
      await clearSessionCookie();
      store.set(ENROLMENT_COOKIE, '', { ...COOKIE_ATTRIBUTES, maxAge: 0 });
    }
    return shown(result.failure);
  }

  // Enrolment is committed; the blob has no further use and outliving the
  // request is the only thing it could do wrong.
  store.set(ENROLMENT_COOKIE, '', { ...COOKIE_ATTRIBUTES, maxAge: 0 });

  if (result.value.password_change_required) {
    // No token is obtained yet, and none can be: `/authorize` refuses a
    // session whose staff row still carries the flag.
    redirect(PASSWORD_PATH);
  }

  const exchanged = await completeAuthorizationCode(config, token);
  if (!exchanged.ok) {
    return shown(exchanged.failure);
  }
  redirect(HOME_PATH);
}

/**
 * Replaces this staff member's password, having been shown the current one.
 *
 * Reachable only by a session that has presented both factors — vpay checks
 * that, not this app. Since 2026-09-10 (issue #79 item 3) vpay also requires
 * the password **in force**, and this action carries it: the two factors were
 * presented once, up to twelve hours earlier, so without it the credential
 * protecting an irreversible account takeover is the session cookie alone.
 *
 * vpay deletes every *other* session of this staff member on success. Nothing
 * is needed here for that — the sessions being ended are other browsers' —
 * and this one survives on purpose, which is why the redirect below still
 * works.
 */
export async function changePassword(_previous: FormState, form: FormData): Promise<FormState> {
  // Issue #88 item 4. First, before anything is read out of the form and
  // before any cookie is touched: a cross-origin caller must not be able to
  // spend a rate-limit unit, let alone a credential.
  const crossOrigin = await originRefusal();
  if (crossOrigin !== null) {
    return crossOrigin;
  }

  const { config } = dashboardConfig();
  if (config === null) {
    return { error: UNCONFIGURED, requestId: null };
  }

  const token = await sessionToken();
  if (token === null) {
    redirect(LOGIN_PATH);
  }

  const current = form.get('current_password');
  const next = form.get('new_password');
  const confirm = form.get('confirm_password');
  if (typeof current !== 'string' || current.length === 0) {
    // Refused here rather than at vpay for `signIn`'s reason — an empty
    // string must not cost an argon2id verification — and, unlike the pair
    // check below, this is NOT a rule this app owns: vpay refuses an absent
    // current password with the same `401` it answers a wrong one. What this
    // buys is a sentence that says which field is empty, which vpay
    // deliberately will not.
    return { error: 'Enter your current password.', requestId: null };
  }
  if (typeof next !== 'string' || next.length === 0) {
    return { error: 'Choose a new password.', requestId: null };
  }
  if (next !== confirm) {
    // Checked here because it is the one rule vpay has no way to check: it
    // sees one password, and a mistyped pair would be a password nobody can
    // reproduce.
    return { error: 'The two passwords do not match.', requestId: null };
  }

  const result = await postForm<{ password_change_required: boolean; other_sessions_revoked: number }>(
    config.apiBaseUrl,
    '/dash/v1/staff/password',
    // NOT trimmed, either of them, for the reason `signIn` states: a
    // password's leading or trailing space is part of it.
    { current_password: current, new_password: next },
    { sessionToken: token },
  );
  if (!result.ok) {
    // NOT `if (status === 401) clearSessionCookie()`, which is what stood
    // here until the exp36 review, and which was correct only for as long as
    // this endpoint had no credential to refuse.
    //
    // Since 2026-09-10 vpay answers `401` here for a wrong or absent CURRENT
    // PASSWORD as well as for a session it will not accept — one answer for
    // both, deliberately, because this module has exactly one refusal. So the
    // old reading turned a typo into a sign-out: the cookie went, the next
    // render of this page found no token and redirected, and the person never
    // saw the sentence telling them what was wrong. Measured in a browser —
    // `dashboard.cy.ts` leg 4 typed a wrong current password, expected the
    // alert, and got `(new url) /login` instead.
    //
    // Nothing is lost by not clearing it. A session that really IS over is
    // caught one render later by `PasswordPage`, which reads the session on
    // every render and redirects to `/login` when vpay refuses it — the page
    // whose job that is, deciding it from a fresh answer, rather than an
    // action inferring it from a status that now means two things.
    return shown(result.failure);
  }

  const exchanged = await completeAuthorizationCode(config, token);
  if (!exchanged.ok) {
    return shown(exchanged.failure);
  }
  redirect(HOME_PATH);
}

/**
 * Signs out: deletes the vpay session row, then clears the cookie.
 *
 * **That order matters.** The row is what carries the access token, so
 * deleting it is the revocation (ADR-0017 decision 2); clearing the cookie
 * first and failing to reach vpay would leave a live row with a live token
 * on it and nothing pointing at it — a session nobody can sign out of.
 *
 * `/dash/v1/staff/logout` is idempotent and answers `200` for a token that
 * was never there, so a second sign-out is not an error. The cookie is
 * cleared whatever vpay answered: a staff member who pressed the button must
 * end up signed out of this browser.
 */
export async function signOut(): Promise<void> {
  // Issue #88 item 4, and this one is the reason the check is per action
  // rather than per form: a sign-out is the action an `<img src>` or a link
  // scanner would fire, which `signed-in-bar.tsx` already made a POST to
  // avoid. A refused call does nothing at all — it does not clear the cookie
  // and it does not delete the row — and answers the redirect a browser that
  // reached it honestly would have got, so a forged one is indistinguishable
  // from a completed one to whoever forged it.
  if ((await originRefusal()) !== null) {
    redirect(LOGIN_PATH);
  }

  const { config } = dashboardConfig();
  const token = await sessionToken();
  if (config !== null && token !== null) {
    await postForm<{ signed_out: boolean }>(
      config.apiBaseUrl,
      '/dash/v1/staff/logout',
      {},
      { sessionToken: token },
    );
  }
  await clearSessionCookie();
  redirect(LOGIN_PATH);
}
