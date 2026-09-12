/**
 * The decisive check for Lane 3: **403 is an outage, not a sign-out.**
 *
 * The plan names widening `onError` to `>= 400` as the mutation that must
 * make this file red, because that widening is issue #88 item 2 — a rolling
 * restart of vpay answers `503`, and reading any `4xx`/`5xx` as "your
 * session is over" signed every staff member out with no way to tell it from
 * a deliberate sign-out.
 */
import { describe, expect, it, vi } from "vitest";

import { dashAuthProvider, isSignOut } from "./auth-provider";

const noopActions = {
  signOut: () => Promise.resolve(),
  check: () => Promise.resolve(true),
};

describe("onError — the one status that means the session is over", () => {
  it("signs out on 401, and only on 401", async () => {
    const provider = dashAuthProvider(noopActions);
    const result = await provider.onError({ statusCode: 401 });
    expect(result.logout, "401 is the session refusal vpay answers").toBe(true);
    expect(result.redirectTo).toBe("/login");
  });

  /**
   * Each of these is a deployment problem, not a fact about the person. `403`
   * is the one the plan calls out by name: on this surface it means vpay is
   * behind something that refused this app.
   */
  for (const status of [400, 403, 404, 409, 429, 500, 502, 503]) {
    it(`treats ${status} as an outage and leaves the session alone`, async () => {
      const provider = dashAuthProvider(noopActions);
      const result = await provider.onError({ statusCode: status });
      expect(
        result.logout,
        `${status} must not sign anyone out — widening onError to >= 400 is the named mutation`,
      ).toBeFalsy();
      expect(result.redirectTo).toBeUndefined();
    });
  }

  it("treats an error carrying no status as an outage", async () => {
    // A network failure has no status. The conservative reading — "something
    // went wrong, sign them out" — is the same defect in another costume.
    const provider = dashAuthProvider(noopActions);
    const result = await provider.onError(new Error("fetch failed"));
    expect(result.logout).toBeFalsy();
  });

  it("reads the status structurally, so a serialised error still maps", () => {
    // Refine may carry an error across a boundary that loses the prototype.
    expect(isSignOut({ statusCode: 401 })).toBe(true);
    expect(isSignOut({ statusCode: 403 })).toBe(false);
    expect(isSignOut({ status: 401 }), "`status` is not the property").toBe(
      false,
    );
    expect(isSignOut(null)).toBe(false);
    expect(isSignOut("401")).toBe(false);
  });
});

describe("logout revokes before it clears", () => {
  it("calls the existing signOut Server Action", async () => {
    // Not a reimplementation: the existing action posts the revocation BEFORE
    // clearing the cookie, and that ordering is the whole of why this wraps
    // rather than replaces.
    const signOut = vi.fn(() => Promise.resolve());
    const provider = dashAuthProvider({ ...noopActions, signOut });
    const result = await provider.logout({});
    expect(signOut).toHaveBeenCalledOnce();
    expect(result.success).toBe(true);
    expect(result.redirectTo).toBe("/login");
  });
});

describe("check defers to the existing gate", () => {
  it("sends an unauthenticated visitor to /login", async () => {
    const provider = dashAuthProvider({
      ...noopActions,
      check: () => Promise.resolve(false),
    });
    const result = await provider.check({});
    expect(result.authenticated).toBe(false);
    expect(result.redirectTo).toBe("/login");
  });

  it("lets a live session through with no redirect", async () => {
    const provider = dashAuthProvider(noopActions);
    const result = await provider.check({});
    expect(result.authenticated).toBe(true);
    expect(result.redirectTo).toBeUndefined();
  });
});

describe("login is not handled here, and says so", () => {
  it("refuses rather than pretending to sign anyone in", async () => {
    // Two factors and a forced-password-change stage. Expressing that through
    // a single `login()` would mean reimplementing it, and a provider that
    // silently succeeded would be worse than one that refuses.
    const provider = dashAuthProvider(noopActions);
    const result = await provider.login({});
    expect(result.success).toBe(false);
    expect(result.redirectTo).toBe("/login");
  });
});
