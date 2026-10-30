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
import { describe, expect, it } from "vitest";

import {
  COOKIE_ATTRIBUTES,
  ENROLMENT_COOKIE,
  ENROLMENT_MAX_AGE_SECONDS,
  SESSION_COOKIE,
  SESSION_MAX_AGE_SECONDS,
  SSO_PENDING_COOKIE,
  SSO_PENDING_MAX_AGE_SECONDS,
} from "./cookies";

describe("the session cookie", () => {
  it("is httpOnly — no script on this origin may read a payments credential", () => {
    expect(COOKIE_ATTRIBUTES.httpOnly).toBe(true);
  });

  it("is Secure, unconditionally and in every deployment", () => {
    // No environment branch: AGENTS.md forbids one, and a cookie that drops
    // Secure outside production is a cookie in clear text the day a staging
    // hostname points at this image. Browsers treat http://localhost as a
    // secure context, so the compose stack is covered too.
    expect(COOKIE_ATTRIBUTES.secure).toBe(true);
  });

  it("is SameSite=Lax — not None, and not Strict", () => {
    // `none` would let any site POST with the session attached. `strict`
    // would sign a staff member out of a link to /payments/pi_… followed
    // from a chat window, which is a top-level GET.
    expect(COOKIE_ATTRIBUTES.sameSite).toBe("lax");
  });

  it("is scoped to the whole app", () => {
    expect(COOKIE_ATTRIBUTES.path).toBe("/");
  });

  it("lives as long as the absolute session bound, never the idle one", () => {
    // 12 h is `staff_sessions`' absolute expiry (ADR-0017 decision 2). The
    // idle bound is 30 minutes and moves on every accepted request; a cookie
    // cannot move with it, and one that expired first would sign out a
    // working session.
    expect(SESSION_MAX_AGE_SECONDS).toBe(12 * 60 * 60);
  });

  it("is not the enrolment cookie, and the enrolment cookie is short-lived", () => {
    expect(SESSION_COOKIE).not.toBe(ENROLMENT_COOKIE);
    expect(ENROLMENT_MAX_AGE_SECONDS).toBeLessThan(SESSION_MAX_AGE_SECONDS);
  });
});

describe("the cookies are __Host- cookies", () => {
  // `__Host-` makes the BROWSER refuse the cookie unless it is Secure, set
  // from a secure origin, has `Path=/` and has no `Domain` — so a sibling
  // subdomain, or a downgraded connection, cannot plant or overwrite the
  // session cookie. The names are the whole mechanism; the attributes below
  // are what the prefix demands and are asserted so a change to one cannot
  // quietly turn the prefix into a cookie the browser drops.
  it("names both cookies with the prefix", () => {
    expect(SESSION_COOKIE).toBe("__Host-vpay_dash_session");
    expect(ENROLMENT_COOKIE).toBe("__Host-vpay_dash_enrolment");
    expect(SESSION_COOKIE.startsWith("__Host-")).toBe(true);
    expect(ENROLMENT_COOKIE.startsWith("__Host-")).toBe(true);
  });

  it("names the single sign-on pending cookie with the prefix too, and apart from the session's", () => {
    expect(SSO_PENDING_COOKIE).toBe("__Host-vpay_dash_sso_pending");
    expect(SSO_PENDING_COOKIE.startsWith("__Host-")).toBe(true);
    // Held apart so a pending token can never be read as a session, and a
    // refused flow can forget itself without signing anybody out.
    expect(SSO_PENDING_COOKIE).not.toBe(SESSION_COOKIE);
    expect(SSO_PENDING_COOKIE).not.toBe(ENROLMENT_COOKIE);
  });

  it("lets the pending cookie live ten minutes — vpay's own bound on a flow", () => {
    expect(SSO_PENDING_MAX_AGE_SECONDS).toBe(10 * 60);
    expect(SSO_PENDING_MAX_AGE_SECONDS).toBeLessThan(SESSION_MAX_AGE_SECONDS);
  });

  it("carries what the prefix requires: Secure, Path=/, and no Domain", () => {
    expect(COOKIE_ATTRIBUTES.secure).toBe(true);
    expect(COOKIE_ATTRIBUTES.path).toBe("/");
    expect(COOKIE_ATTRIBUTES).not.toHaveProperty("domain");
  });
});
