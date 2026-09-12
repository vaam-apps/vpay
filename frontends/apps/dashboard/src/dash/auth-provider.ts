/**
 * Refine's `AuthProvider`, as a **wrapper over the auth stack that already
 * exists** — never a replacement for it.
 *
 * Lane 3 of `docs/plans/exp55-refine-seam-bff-notes/refine-plan.md`, §2.5.
 *
 * # What must never move in here
 *
 * PKCE, the code exchange, the 80 %-of-TTL re-mint, the
 * `X-Vpay-Staff-Session` header and the single-`401` retry all run
 * server-side, because the token must not reach a browser. An
 * `authProvider` that "handles the token" is the failure this shape exists
 * to prevent, and none of those files is touched by this one.
 *
 * # `onError` is the decisive rule, and it is not Refine's default
 *
 * A CRUD framework's instinct is that any `4xx` means the session is over.
 * That instinct is **issue #88 item 2**: a rolling restart of vpay answers
 * `503`, and reading that as "signed out" logged every staff member out and
 * gave them no way to tell it from being signed out on purpose.
 *
 * `refusalFor` in `src/server/gate.ts` is the one rule, and this mirrors it
 * exactly rather than approximating it: **`401` and only `401` is a
 * sign-out.** `403` is an outage — on this surface it means vpay is behind
 * something that refused this app, which is a deployment problem and not a
 * fact about the person. Widening this to `>= 400`, or to `!== 200`, is the
 * named decisive mutation, and `auth-provider.test.ts` fails on it.
 */
import type { AuthProvider } from "@refinedev/core";

/**
 * The status a failed read carries. Structural rather than a class check, so
 * an error crossing Refine's own boundaries — which may serialise it — is
 * still read correctly.
 */
function statusOf(error: unknown): number | null {
  if (typeof error !== "object" || error === null) {
    return null;
  }
  const code: unknown = (error as { statusCode?: unknown }).statusCode;
  return typeof code === "number" ? code : null;
}

/**
 * `401` is the only status vpay answers for a session it has refused, and it
 * answers it for **every** such refusal by design — absent, expired, idle,
 * forged, disabled, at the wrong stage. So the mapping is exact rather than
 * conservative: no other status could mean the session is over, and every
 * other status is something the deployment has to fix.
 */
export function isSignOut(error: unknown): boolean {
  return statusOf(error) === 401;
}

export interface DashAuthActions {
  /** The existing `signOut` Server Action — it revokes BEFORE clearing the cookie. */
  readonly signOut: () => Promise<void>;
  /** The existing gate, as a client-callable check. Returns whether the session is live. */
  readonly check: () => Promise<boolean>;
}

export function dashAuthProvider(actions: DashAuthActions): AuthProvider {
  return {
    /**
     * Sign-in is the existing `signIn` / `submitTotp` Server Actions and the
     * `/login` routes around them, unchanged. Refine never drives it — it
     * is a two-factor flow with a forced-password-change stage, and
     * expressing that through a single `login()` would mean reimplementing
     * it. `check` sending an unauthenticated visitor to `/login` is the
     * whole of this provider's involvement.
     */
    // Not `async`: Refine types every AuthProvider method as returning a
    // promise, and this one has nothing to await. Returning the promise
    // directly says that, where an `async` with no `await` in it only looks
    // like an oversight.
    login: () =>
      Promise.resolve({
        success: false,
        redirectTo: "/login",
        error: {
          name: "NotHandledHere",
          message: "sign-in is the /login routes and their Server Actions",
        },
      }),

    logout: async () => {
      await actions.signOut();
      return { success: true, redirectTo: "/login" };
    },

    check: async () => {
      const live = await actions.check();
      return live
        ? { authenticated: true }
        : { authenticated: false, redirectTo: "/login" };
    },

    // Pure function of the status, returned as a promise for the same
    // reason `login` is.
    onError: (error) =>
      Promise.resolve(
        // The one rule. See this module's header, and `refusalFor`.
        isSignOut(error)
          ? { logout: true, redirectTo: "/login", error: error as Error }
          : // Everything else — 403, 500, 503, a network failure — is an
            // outage. The screen renders `ReadFailure`; the session is
            // untouched.
            { error: error as Error },
      ),
  };
}
