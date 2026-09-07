/**
 * What a protected page must do about the session it just read.
 *
 * A pure function over `GET /dash/v1/staff/session`'s answer, separated from
 * the Next-bound half in `session.ts` so the decisions are a unit test rather
 * than something only a browser can exercise. Every branch below is one a
 * page takes *before* rendering anything.
 */
import type { SessionResponse } from './api';

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
