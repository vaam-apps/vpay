/**
 * What a protected page must do about the session it just read.
 *
 * A pure function over `GET /dash/v1/staff/session`'s answer, separated from
 * the Next-bound half in `session.ts` so the decisions are a unit test rather
 * than something only a browser can exercise. Every branch below is one a
 * page takes *before* rendering anything.
 */
import type { ApiFailure, SessionResponse, SessionStageResponse } from './api';

/** What `gateFor` decided. */
export type Gate =
  /**
   * The one-time password the operator printed is still in use. ADR-0017
   * decision 1: every authenticated route refuses a session carrying this
   * flag, `/authorize` included — so this is not a nag screen, it is the
   * only page such a session can reach.
   */
  | { readonly kind: 'must-change-password'; readonly session: SessionResponse }
  /**
   * Signed in, but no code exchange has happened yet — the token that reads
   * `/dash/v1` has not been minted. The page runs the authorization-code leg
   * and asks again.
   */
  | { readonly kind: 'needs-token'; readonly session: SessionResponse }
  /**
   * Signed in, holding a token that is **near the end of its life**. Mint a
   * replacement before reading — and, unlike `needs-token`, there is
   * something to fall back on if the mint cannot be reached: see
   * {@link staleTokenFor}.
   */
  | {
      readonly kind: 'stale-token';
      readonly session: SessionResponse;
      readonly accessToken: string;
    }
  /** Signed in, with a token to read `/dash/v1` with. */
  | {
      readonly kind: 'ready';
      readonly session: SessionResponse;
      readonly accessToken: string;
    };

/**
 * How much of a token's life must be **gone** before a render replaces it.
 *
 * 80 %, so a token is re-minted with a fifth of its TTL still in hand: 180
 * seconds at the shipping 900, four at the few seconds an end-to-end run
 * configures. A fraction and not a fixed number of seconds because the TTL is
 * `staff_auth.access_token_ttl_seconds` now and a deployment may set it —
 * sixty seconds of margin would be a fifteenth of one TTL and three times
 * another, which is either too late to matter or a re-mint on every render.
 *
 * The margin is not an optimisation. Without it the *first* read after the
 * expiry fails and `dash-read.ts` retries it, so every fifteen minutes a staff
 * member's page costs a refused request before it costs a good one — and that
 * reactive path stays, because a clock that disagrees with vpay's, or a token
 * revoked mid-render, is not something a margin can see.
 */
export const REMINT_AFTER_FRACTION = 0.8;

/**
 * The decision, in the order the checks have to happen.
 *
 * The password check is **first**, and that ordering is load-bearing: a
 * session with `password_change_required` cannot obtain a token at all —
 * `vpay_api::staff::oauth::authorize`'s third refusal — so asking for one
 * first would spend a round trip to be told the same thing in a shape that
 * reads like a failure rather than like a step.
 *
 * @param session what `GET /dash/v1/staff/session` answered
 * @param now the render's own instant, in milliseconds since the epoch.
 *   A parameter and not `Date.now()` inside, so the expiry arithmetic is
 *   something a unit test can stand at either side of.
 */
export function gateFor(session: SessionResponse, now: number): Gate {
  if (session.password_change_required) {
    return { kind: 'must-change-password', session };
  }
  const token = session.access_token;
  if (typeof token !== 'string' || token.length === 0) {
    return { kind: 'needs-token', session };
  }
  return staleTokenFor(session, now)
    ? { kind: 'stale-token', session, accessToken: token }
    : { kind: 'ready', session, accessToken: token };
}

/**
 * Whether the token on this session is close enough to its expiry to replace.
 *
 * # Every unreadable answer is "replace it"
 *
 * A missing `access_token_expires_at`, one that does not parse, and a
 * non-positive TTL all read as stale. That is the fail-closed direction and it
 * is cheap: re-minting is *more* checking than carrying a token — the
 * authorization leg re-reads the staff row, the active status, the merchant
 * binding and `password_change_required` on every mint — where trusting an
 * unknown expiry is the fifteen minutes the exp28 review measured.
 *
 * vpay pairs the two columns (migration 0040's
 * `staff_sessions_token_expiry_is_paired`), so a token with no expiry is a
 * shape this app should never see; this is what it does if it ever does.
 *
 * The decisive mutation is returning `false` for an absent or unparseable
 * expiry, which is the shape "only re-mint when we are sure" would take.
 */
function staleTokenFor(session: SessionResponse, now: number): boolean {
  const ttlSeconds = session.access_token_ttl_seconds;
  if (typeof ttlSeconds !== 'number' || !Number.isFinite(ttlSeconds) || ttlSeconds <= 0) {
    return true;
  }
  const expiresAt = Date.parse(session.access_token_expires_at ?? '');
  if (Number.isNaN(expiresAt)) {
    return true;
  }
  // `>=` and not `>`: at exactly the margin the fifth is spent, and a
  // boundary that renders with the old token is a boundary an end-to-end run
  // cannot stand on.
  const marginMs = ttlSeconds * (1 - REMINT_AFTER_FRACTION) * 1000;
  return now >= expiresAt - marginMs;
}

/** What a refused read of `/dash/v1/staff/session` means for this browser. */
export type Refusal =
  /** The session is over: forget the cookie and show the form. */
  | 'sign-out'
  /** vpay could not answer. Keep the cookie and render the failure. */
  | 'outage';

/**
 * Whether a refusal ends the session, or is a vpay this app could not reach.
 *
 * # `401` and nothing else ends a session
 *
 * `requireStaff` used to send a browser to `/signed-out` for **every**
 * failure of the session read — and `server/api.ts` turns a `fetch` that
 * rejected into an `ApiFailure` with `status: 0` rather than throwing, so
 * "vpay is restarting", "the connection was reset" and "vpay answered `503`"
 * were all indistinguishable from "your session is over". A rolling restart
 * of vpay therefore signed every staff member out of the dashboard, and they
 * could not tell that from having been signed out on purpose (issue #88
 * item 2).
 *
 * `401` is the only status vpay answers for a session it has refused, and it
 * answers it for **every** such refusal by design — absent, expired, idle,
 * forged, disabled, at the wrong stage
 * (`docs/flows/dashboard-auth.md`, "Every refusal is one answer"). So the
 * mapping is exact rather than conservative: there is no other status that
 * could mean the session is over, and every other status is something the
 * deployment has to fix.
 *
 * **`403` is an outage here, not a sign-out**, and that is deliberate: on the
 * session route it would mean vpay is behind something that refused this app,
 * which is a deployment problem and not a fact about the person.
 *
 * The decisive mutation is widening this to `>= 400`, or to `!== 200`: the
 * `503` case in `gate.test.ts` then reads `'sign-out'`.
 */
export function refusalFor(failure: ApiFailure): Refusal {
  return failure.status === 401 ? 'sign-out' : 'outage';
}

/** What `/login/totp` must do about the session read it just took. */
export type TotpGate =
  /** Live, and still owed a code: render the form (and the enrolment panel). */
  | { readonly kind: 'enter-code' }
  /**
   * Live, and both factors are already in. A back button, a second tab, or a
   * reload after the action redirected. Send them on: `requireStaff` decides
   * from there whether the printed password still has to be replaced.
   */
  | { readonly kind: 'signed-in' }
  /** vpay refused the session. Back to the form — this is the only branch that does. */
  | { readonly kind: 'dead' }
  /** vpay could not answer. Keep the cookie and render the failure. */
  | { readonly kind: 'outage'; readonly failure: ApiFailure };

/**
 * The decision `/login/totp` takes before rendering anything.
 *
 * # A mistyped code is not a dead session, and telling them apart needs a read
 *
 * `POST /dash/v1/staff/totp` answers `401` for a **wrong six-digit code** and
 * for every session it will not accept — one answer, deliberately
 * (`docs/flows/dashboard-auth.md`, "Every refusal is one answer"). Until
 * 2026-09-10 `submitTotp` read any `401` from it as "the session is over" and
 * cleared the cookie, so the commonest of those cases — a typo — sent a staff
 * member back to the email-and-password form with no explanation. That is the
 * exp36 review's F6, and F1 was the identical shape one route over.
 *
 * The two lines could not simply be dropped, and that is why this function
 * exists: the page read **no** session, only the cookie's presence, so with
 * the cookie kept a session that really was over would leave somebody typing
 * codes at a form that could never accept one. The page now asks
 * `GET /dash/v1/staff/session/stage` on every render — `/staff/session` is
 * refused at this stage, which is what made this a new route rather than a
 * reuse — and the answer, not the action's status, is what ends a session.
 *
 * The decisive mutation is mapping any failure to `'dead'`: the outage case
 * below then reads `'dead'`, which is a vpay restart signing everybody out
 * mid-sign-in (issue #88 item 2, on this route).
 *
 * @param stage the stage read's answer, or `null` if it failed
 * @param failure why it failed, or `null` if it did not
 */
export function totpGateFor(
  stage: SessionStageResponse | null,
  failure: ApiFailure | null,
): TotpGate {
  if (stage === null) {
    // Fail closed on a read that answered neither: a page that rendered a
    // code form for a session it knows nothing about is what this replaces.
    if (failure !== null && refusalFor(failure) === 'outage') {
      return { kind: 'outage', failure };
    }
    return { kind: 'dead' };
  }
  return stage.stage === 'authenticated' ? { kind: 'signed-in' } : { kind: 'enter-code' };
}
