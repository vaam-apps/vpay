/**
 * What a protected page must do about the session it just read.
 *
 * A pure function over `GET /dash/v1/staff/session`'s answer, separated from
 * the Next-bound half in `session.ts` so the decisions are a unit test rather
 * than something only a browser can exercise. Every branch below is one a
 * page takes *before* rendering anything.
 */
import type { ApiFailure, SessionResponse } from './api';

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
  /** Signed in, with a token to read `/dash/v1` with. */
  | {
      readonly kind: 'ready';
      readonly session: SessionResponse;
      readonly accessToken: string;
    };

/**
 * The decision, in the order the checks have to happen.
 *
 * The password check is **first**, and that ordering is load-bearing: a
 * session with `password_change_required` cannot obtain a token at all —
 * `vpay_api::staff::oauth::authorize`'s third refusal — so asking for one
 * first would spend a round trip to be told the same thing in a shape that
 * reads like a failure rather than like a step.
 */
export function gateFor(session: SessionResponse): Gate {
  if (session.password_change_required) {
    return { kind: 'must-change-password', session };
  }
  const token = session.access_token;
  if (typeof token !== 'string' || token.length === 0) {
    return { kind: 'needs-token', session };
  }
  return { kind: 'ready', session, accessToken: token };
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
