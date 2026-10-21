/**
 * The decisive check for Lane 3: **403 is an outage, not a sign-out.**
 *
 * The plan names widening `onError` to `>= 400` as the mutation that must
 * make this file red, because that widening is issue #88 item 2 — a rolling
 * restart of vpay answers `503`, and reading any `4xx`/`5xx` as "your
 * session is over" signed every staff member out with no way to tell it from
 * a deliberate sign-out.
 */
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it, vi } from "vitest";

import { dashAuthProvider, isSignOut } from "./auth-provider";

const noopActions = {
  signOut: () => Promise.resolve(),
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

describe("check is not a second gate, and costs nothing", () => {
  it("answers without touching the network or the session", async () => {
    // Refine calls this on every pathname change and throws the answer
    // away. When it called the `checkSession` Server Action, every
    // navigation paid a POST and an extra `GET /dash/v1/staff/session`
    // (measured 2026-10-21). The server gates every page; this must not.
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    const signOut = vi.fn(() => Promise.resolve());
    const provider = dashAuthProvider({ signOut });
    const result = await provider.check({});
    expect(result.authenticated).toBe(true);
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(signOut).not.toHaveBeenCalled();
    fetchSpy.mockRestore();
  });

  it("is never acted on: nothing renders <Authenticated> or asks useIsAuthenticated", () => {
    // `check` answering `true` without asking is only honest while no
    // component makes a decision from it. Refine's two consumers are those
    // two; the day one appears, this goes red and `check` has to ask again.
    const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
    const offenders: string[] = [];
    for (const dir of ["app", "src"]) {
      for (const entry of readdirSync(join(root, dir), { recursive: true })) {
        const rel = entry.toString();
        if (!/\.tsx?$/.test(rel) || rel.includes(".test.")) continue;
        const code = readFileSync(join(root, dir, rel), "utf8")
          .replace(/\/\*[\s\S]*?\*\//g, "")
          .replace(/\/\/[^\n]*/g, "");
        if (/<Authenticated[\s>]|useIsAuthenticated\(/.test(code)) {
          offenders.push(`${dir}/${rel}`);
        }
      }
    }
    expect(offenders).toEqual([]);
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
