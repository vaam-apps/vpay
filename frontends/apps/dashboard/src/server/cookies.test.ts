/**
 * The cookie attributes, pinned.
 *
 * These are the decisive mutations `docs/plans/exp28-dashboard-pages-notes/opus.md`
 * names: drop `httpOnly` and this file fails; drop `secure` and it fails.
 * They are asserted on the exported constant rather than on a rendered
 * `Set-Cookie`, because the constant is what every call site spreads — three
 * call sites spelling the attributes out separately is exactly how one of
 * them loses `httpOnly`, and a test that only exercised one of the three
 * would not see it.
 */
import { describe, expect, it } from 'vitest';

import {
  COOKIE_ATTRIBUTES,
  ENROLMENT_COOKIE,
  ENROLMENT_MAX_AGE_SECONDS,
  SESSION_COOKIE,
  SESSION_MAX_AGE_SECONDS,
} from './cookies';

describe('the session cookie', () => {
  it('is httpOnly — no script on this origin may read a payments credential', () => {
    expect(COOKIE_ATTRIBUTES.httpOnly).toBe(true);
  });

  it('is Secure, unconditionally and in every deployment', () => {
    // No environment branch: AGENTS.md forbids one, and a cookie that drops
    // Secure outside production is a cookie in clear text the day a staging
    // hostname points at this image. Browsers treat http://localhost as a
    // secure context, so the compose stack is covered too.
    expect(COOKIE_ATTRIBUTES.secure).toBe(true);
  });

  it('is SameSite=Lax — not None, and not Strict', () => {
    // `none` would let any site POST with the session attached. `strict`
    // would sign a staff member out of a link to /payments/pi_… followed
    // from a chat window, which is a top-level GET.
    expect(COOKIE_ATTRIBUTES.sameSite).toBe('lax');
  });

  it('is scoped to the whole app', () => {
    expect(COOKIE_ATTRIBUTES.path).toBe('/');
  });

  it('lives as long as the absolute session bound, never the idle one', () => {
    // 12 h is `staff_sessions`' absolute expiry (ADR-0017 decision 2). The
    // idle bound is 30 minutes and moves on every accepted request; a cookie
    // cannot move with it, and one that expired first would sign out a
    // working session.
    expect(SESSION_MAX_AGE_SECONDS).toBe(12 * 60 * 60);
  });

  it('is not the enrolment cookie, and the enrolment cookie is short-lived', () => {
    expect(SESSION_COOKIE).not.toBe(ENROLMENT_COOKIE);
    expect(ENROLMENT_MAX_AGE_SECONDS).toBeLessThan(SESSION_MAX_AGE_SECONDS);
  });
});
