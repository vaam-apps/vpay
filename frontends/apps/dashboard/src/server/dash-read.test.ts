/**
 * Reading `/dash/v1` across the fifteen-minute boundary.
 *
 * Stubbed at the network boundary and nowhere else, like `oauth.test.ts`, so
 * every assertion is about the requests this app actually put on the wire —
 * which bearer each carried, and whether a code exchange happened at all.
 *
 * The decisive case is `mints a fresh token once when the one it holds has
 * expired`: without the retry, `/payments` renders "The bearer token is
 * invalid, expired, or was not issued for this endpoint" for the rest of every
 * sign-in past its first quarter of an hour, and `dashboard.cy.ts` — which
 * runs in under thirty seconds — cannot see it.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import type { DashboardConfig } from "../config/settings";
import { readDash } from "./dash-read";

const CONFIG: DashboardConfig = {
  apiBaseUrl: "http://vpay-server:8080",
  clientId: "vpay-dashboard",
  redirectUri: "http://localhost:3000/dash/v1/callback",
  scope: "dashboard:read",
  // Not what this suite is about; `csrf.test.ts` is where the origin check
  // is exercised. `null` is the shipping default and means the `Host`
  // fallback rather than an allow-everything.
  publicOrigin: null,
};

const EXPIRED = "the-token-on-the-row";
const FRESH = "the-token-the-exchange-minted";
const SESSION = "the-staff-session-token";

/** One request the stub saw. */
interface Seen {
  url: string;
  authorization: string | null;
}

/**
 * A `fetch` that answers the two OAuth legs and, for `/payment_intents`,
 * whatever `answers` says for the bearer it was given.
 */
function stub(seen: Seen[], answers: (bearer: string | null) => Response) {
  return vi.fn((input: string | URL | Request, init?: RequestInit) => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.href
          : input.url;
    const headers = (init?.headers ?? {}) as Record<string, string>;
    const authorization = headers["authorization"] ?? null;
    seen.push({ url, authorization });

    if (url.includes("/oauth/authorize")) {
      const query = new URL(url).searchParams;
      return Promise.resolve(
        new Response(null, {
          status: 302,
          headers: {
            location: `${CONFIG.redirectUri}?code=a-code&state=${query.get("state") ?? ""}`,
          },
        }),
      );
    }
    if (url.includes("/oauth/token")) {
      return Promise.resolve(
        new Response(
          JSON.stringify({
            access_token: FRESH,
            token_type: "Bearer",
            expires_in: 900,
            scope: CONFIG.scope,
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        ),
      );
    }
    return Promise.resolve(answers(authorization));
  });
}

/** vpay's own refusal envelope, at a status. */
function refusal(status: number, message: string): Response {
  return new Response(
    JSON.stringify({
      error: { code: "invalid_token", message, type: "authentication_error" },
    }),
    {
      status,
      headers: { "content-type": "application/json", "request-id": "req_01J8" },
    },
  );
}

/** A page of payment intents. */
function page(): Response {
  return new Response(
    JSON.stringify({
      object: "list",
      data: [{ id: "pi_one" }],
      has_more: false,
    }),
    { status: 200, headers: { "content-type": "application/json" } },
  );
}

/** Only the `/payment_intents` calls, in order. */
function reads(seen: Seen[]): Seen[] {
  return seen.filter((call) => call.url.includes("/payment_intents"));
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("reading /dash/v1", () => {
  it("uses the token from the session row and exchanges nothing when it works", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, () => page()),
    );

    const result = await readDash(
      CONFIG,
      "/dash/v1/payment_intents",
      EXPIRED,
      SESSION,
    );

    expect(result.ok).toBe(true);
    expect(reads(seen)).toHaveLength(1);
    expect(reads(seen)[0]?.authorization).toBe(`Bearer ${EXPIRED}`);
    // The happy path must not spend a code exchange on every render.
    expect(seen.some((call) => call.url.includes("/oauth/"))).toBe(false);
  });

  it("mints a fresh token once when the one it holds has expired — THE decisive case", async () => {
    // `/dash/v1` answers 401 for an expired bearer, and the token's TTL is 900
    // seconds against a session that may live twelve hours. Without this, the
    // dashboard stops working fifteen minutes into every sign-in.
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, (bearer) =>
        bearer === `Bearer ${FRESH}`
          ? page()
          : refusal(
              401,
              "The bearer token is invalid, expired, or was not issued for this endpoint.",
            ),
      ),
    );

    const result = await readDash(
      CONFIG,
      "/dash/v1/payment_intents",
      EXPIRED,
      SESSION,
    );

    expect(result.ok, "the retry must succeed").toBe(true);
    expect(reads(seen).map((call) => call.authorization)).toEqual([
      `Bearer ${EXPIRED}`,
      `Bearer ${FRESH}`,
    ]);
    // And the replacement came from the real grant, not from anywhere else.
    expect(seen.some((call) => call.url.includes("/oauth/authorize"))).toBe(
      true,
    );
    expect(seen.some((call) => call.url.includes("/oauth/token"))).toBe(true);
  });

  it("gives up after one mint rather than looping on a credential operation", async () => {
    // A second 401 after a fresh token is not an expiry, and retrying it again
    // would be a loop with a code exchange in it.
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, () => refusal(401, "still refused")),
    );

    const result = await readDash(
      CONFIG,
      "/dash/v1/payment_intents",
      EXPIRED,
      SESSION,
    );

    expect(result.ok).toBe(false);
    expect(reads(seen)).toHaveLength(2);
    expect(result.ok ? "" : result.failure.requestId).toBe("req_01J8");
  });

  it("never retries a 403 — a new token from the same registration says the same thing", async () => {
    // `/dash/v1` answers 403 for a token that is valid and not allowed: the
    // wrong scope, or a merchant claim that is not the binding.
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, () => refusal(403, "not allowed here")),
    );

    const result = await readDash(
      CONFIG,
      "/dash/v1/payment_intents",
      EXPIRED,
      SESSION,
    );

    expect(result.ok).toBe(false);
    expect(reads(seen)).toHaveLength(1);
    expect(seen.some((call) => call.url.includes("/oauth/"))).toBe(false);
  });

  it("reports the read refusal, not the exchange, when no token can be minted", async () => {
    // The session was signed out or went idle. The read's own sentence is the
    // truer one to render, and the next render takes requireStaff's refusal
    // path properly.
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((input: string | URL | Request, init?: RequestInit) => {
        const url =
          typeof input === "string"
            ? input
            : input instanceof URL
              ? input.href
              : input.url;
        const headers = (init?.headers ?? {}) as Record<string, string>;
        seen.push({ url, authorization: headers["authorization"] ?? null });
        if (url.includes("/oauth/authorize")) {
          return Promise.resolve(refusal(401, "this session cannot authorize"));
        }
        return Promise.resolve(
          refusal(401, "the read refusal a page should render"),
        );
      }),
    );

    const result = await readDash(
      CONFIG,
      "/dash/v1/payment_intents",
      EXPIRED,
      SESSION,
    );

    expect(result.ok).toBe(false);
    expect(result.ok ? "" : result.failure.message).toBe(
      "the read refusal a page should render",
    );
    expect(reads(seen)).toHaveLength(1);
  });
});
