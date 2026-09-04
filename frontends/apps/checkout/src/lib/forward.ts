/**
 * Sending the payer back to the merchant.
 *
 * D5: vpay appends nothing to a merchant's URL. The one substitution it
 * makes is the literal `{CHECKOUT_SESSION_ID}`, which a merchant opts into
 * by writing it in `success_url`, `cancel_url` or `return_url`. A merchant
 * who does not write it gets a return with no correlation value, which is
 * documented on the field rather than fixed by a silent parameter.
 */
import type { CheckoutSession } from './types';

/** The one template placeholder vpay substitutes. Stripe's own spelling. */
export const SESSION_ID_PLACEHOLDER = '{CHECKOUT_SESSION_ID}';

/**
 * Replaces **every** occurrence of {@link SESSION_ID_PLACEHOLDER}.
 *
 * Every, not the first: a merchant who writes it in both a path segment and
 * a query parameter meant both. The id is percent-encoded — a no-op for the
 * `cs_[A-Za-z0-9]+` ids vpay mints, and the thing that keeps this correct if
 * that alphabet ever widens, whichever part of the URL the placeholder sits
 * in.
 */
export function substituteSessionId(template: string, sessionId: string): string {
  return template.split(SESSION_ID_PLACEHOLDER).join(encodeURIComponent(sessionId));
}

/** Which of the session's URLs an outcome sends the payer to. */
export type ForwardKind = 'success' | 'cancel' | 'return';

/**
 * The absolute URL to forward to, or `null`.
 *
 * `null` when the session carries no URL for that outcome, and — the case
 * that matters — when the URL is not an absolute `http:`/`https:` one. That
 * value came out of the database, where a merchant put it; the server
 * validates it on create, and this is the second check, on the side that
 * would actually perform the navigation. `javascript:` in an
 * `location.assign` is script execution on vpay's own origin.
 */
export function forwardTarget(session: CheckoutSession, kind: ForwardKind): string | null {
  const template =
    kind === 'success'
      ? session.success_url
      : kind === 'cancel'
        ? session.cancel_url
        : session.return_url;
  if (typeof template !== 'string' || template.length === 0) {
    return null;
  }
  const substituted = substituteSessionId(template, session.id);
  let parsed: URL;
  try {
    parsed = new URL(substituted);
  } catch {
    return null;
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    return null;
  }
  return substituted;
}

/**
 * The outcome-to-URL rule, in one place.
 *
 * Hosted sessions have `success_url` and `cancel_url`; embedded ones have a
 * single `return_url` for every outcome (the wire contract refuses the other
 * pair for that `ui_mode`). A failed payment goes to `cancel_url` on a
 * hosted session: the payer did not pay, and `success_url` is the merchant's
 * "this worked" page.
 */
export function forwardKindFor(
  session: CheckoutSession,
  paid: boolean,
): ForwardKind {
  if (session.ui_mode === 'embedded') {
    return 'return';
  }
  return paid ? 'success' : 'cancel';
}
