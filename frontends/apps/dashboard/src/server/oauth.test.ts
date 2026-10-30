/**
 * The authorization-code leg, driven against a stubbed `fetch`.
 *
 * Stubbed **at the network boundary** and nowhere else: every check below is
 * a check on the request this app actually put on the wire — the PKCE
 * challenge it sent, the verifier it exchanged, the redirect URI it repeated
 * — because those are the four values the grant rests on and they are
 * invisible from anywhere else in this app.
 *
 * The decisive mutation is `the exchange carries the verifier the challenge
 * was derived from`: delete `code_verifier` from `completeAuthorizationCode`'s
 * token request, or send a fresh one, and it fails.
 */
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { afterEach, describe, expect, it, vi } from "vitest";

import type { DashboardConfig } from "../config/settings";
import { challengeFor } from "./pkce";
import { completeAuthorizationCode, readAuthorizationResponse } from "./oauth";

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

/** What one call to the stub recorded. */
interface Recorded {
  url: string;
  headers: Record<string, string>;
  body: URLSearchParams | null;
}

/**
 * A `fetch` that answers `/authorize` with a `302` echoing the challenge back
 * as the code, then answers `/oauth/token` with a token.
 *
 * Echoing the challenge into the code is what lets the token half assert the
 * two legs belong to one exchange without this test reaching inside the
 * module for the verifier it deliberately never stores.
 */
function stubFetch(
  calls: Recorded[],
  overrides: { authorizeStatus?: number; location?: string } = {},
) {
  return vi.fn((input: string | URL | Request, init?: RequestInit) => {
    const url = urlOf(input);
    const headers = Object.fromEntries(
      Object.entries((init?.headers ?? {}) as Record<string, string>),
    );
    const body =
      typeof init?.body === "string" ? new URLSearchParams(init.body) : null;
    calls.push({ url, headers, body });

    if (url.includes("/oauth/authorize")) {
      const challenge = new URL(url).searchParams.get("code_challenge") ?? "";
      const state = new URL(url).searchParams.get("state") ?? "";
      const location =
        overrides.location ??
        `${CONFIG.redirectUri}?code=code-for-${challenge}&state=${state}`;
      return Promise.resolve(
        new Response(null, {
          status: overrides.authorizeStatus ?? 302,
          headers: { location },
        }),
      );
    }
    return Promise.resolve(
      new Response(
        JSON.stringify({
          access_token: "header.payload.signature",
          token_type: "Bearer",
          expires_in: 900,
          scope: CONFIG.scope,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    );
  });
}

/**
 * A `fetch` input as the URL it names.
 *
 * `String(request)` is `[object Request]`, which would make every `url`
 * assertion in this file pass vacuously.
 */
function urlOf(input: string | URL | Request): string {
  if (typeof input === "string") {
    return input;
  }
  return input instanceof URL ? input.href : input.url;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the authorization request", () => {
  it("sends the session token, the registered client, scope and an S256 challenge", async () => {
    const calls: Recorded[] = [];
    vi.stubGlobal("fetch", stubFetch(calls));

    const result = await completeAuthorizationCode(CONFIG, "session-token");
    expect(result.ok).toBe(true);

    const authorize = calls.find((call) =>
      call.url.includes("/oauth/authorize"),
    );
    expect(authorize).toBeDefined();
    const query = new URL(authorize?.url ?? "").searchParams;
    expect(query.get("client_id")).toBe(CONFIG.clientId);
    expect(query.get("redirect_uri")).toBe(CONFIG.redirectUri);
    expect(query.get("response_type")).toBe("code");
    expect(query.get("scope")).toBe(CONFIG.scope);
    expect(query.get("code_challenge_method")).toBe("S256");
    expect(query.get("code_challenge")).toBeTruthy();
    // The session, in the header vpay reads it from — never a cookie: vpay
    // sets none and reads none.
    expect(authorize?.headers["x-vpay-staff-session"]).toBe("session-token");
  });

  it("does not follow the redirect itself", async () => {
    // ADR-0017 decision 4 in one option. `fetch`'s default would follow the
    // Location to a URL nothing serves, and the code would be spent on it.
    const calls: Recorded[] = [];
    const fetchStub = stubFetch(calls);
    vi.stubGlobal("fetch", fetchStub);

    await completeAuthorizationCode(CONFIG, "session-token");

    const authorizeCall = fetchStub.mock.calls.find(([input]) =>
      urlOf(input).includes("/oauth/authorize"),
    );
    expect(authorizeCall?.[1]?.redirect).toBe("manual");
  });
});

describe("the token exchange", () => {
  it("carries the verifier the challenge was derived from — THE decisive case", async () => {
    const calls: Recorded[] = [];
    vi.stubGlobal("fetch", stubFetch(calls));

    await completeAuthorizationCode(CONFIG, "session-token");

    const token = calls.find((call) => call.url.includes("/oauth/token"));
    const verifier = token?.body?.get("code_verifier") ?? "";
    const code = token?.body?.get("code") ?? "";

    // The stub echoed the challenge into the code, so this asserts the
    // exchange presents the verifier for the challenge THIS call sent —
    // not merely that some verifier was present. Drop `code_verifier` from
    // the request, or generate a second pair for the exchange, and this
    // fails.
    expect(verifier).not.toBe("");
    expect(code).toBe(`code-for-${challengeFor(verifier)}`);
  });

  it("repeats the same redirect_uri and client_id the authorization named", async () => {
    const calls: Recorded[] = [];
    vi.stubGlobal("fetch", stubFetch(calls));
    await completeAuthorizationCode(CONFIG, "session-token");

    const token = calls.find((call) => call.url.includes("/oauth/token"));
    expect(token?.body?.get("grant_type")).toBe("authorization_code");
    expect(token?.body?.get("redirect_uri")).toBe(CONFIG.redirectUri);
    expect(token?.body?.get("client_id")).toBe(CONFIG.clientId);
  });

  it("is not attempted at all when /authorize refused", async () => {
    const calls: Recorded[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((input: string | URL | Request, init?: RequestInit) => {
        // Recorded, so the assertion below is about requests that were made
        // and not about an array nothing ever wrote to.
        calls.push({ url: urlOf(input), headers: {}, body: null });
        void init;
        return Promise.resolve(
          new Response(JSON.stringify({ error: { message: "Refused." } }), {
            status: 401,
            headers: {
              "content-type": "application/json",
              "request-id": "req_01J8",
            },
          }),
        );
      }),
    );

    const result = await completeAuthorizationCode(CONFIG, "session-token");
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.failure.status).toBe(401);
      expect(result.failure.requestId).toBe("req_01J8");
    }
    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toContain("/oauth/authorize");
    expect(
      calls.filter((call) => call.url.includes("/oauth/token")),
    ).toHaveLength(0);
  });

  it("asks for an authorization_code grant and for nothing else — there is no refresh grant", async () => {
    const calls: Recorded[] = [];
    vi.stubGlobal("fetch", stubFetch(calls));
    const result = await completeAuthorizationCode(CONFIG, "session-token");
    expect(result.ok).toBe(true);

    for (const call of calls) {
      expect(call.body?.get("grant_type") ?? "authorization_code").toBe(
        "authorization_code",
      );
      expect(call.body?.has("refresh_token") ?? false).toBe(false);
      expect(call.url).not.toContain("refresh");
    }
    // The wire half of "no refresh token". The source half is the scan below.
    expect(calls.filter((call) => call.body !== null)).toHaveLength(1);
  });
});

/**
 * ADR-0037: the dashboard grant is `authorization_code` alone, and a token
 * near the end of its life is replaced by running the authorization request
 * again from the session. The vocabulary of the grant that is not served —
 * the refresh grant, the `refresh_token` parameter and the `offline_access`
 * scope — appears in no source file of this app, so a refresh cannot be wired
 * in by a change that no test reads.
 *
 * The files that may *name* the words are this one and the doc comment in
 * `api.ts` that says there is no such field; both are listed with a reason,
 * and an allowance that matches nothing fails the scan.
 */
describe("no refresh grant anywhere in this app", () => {
  const SERVER_DIR = dirname(fileURLToPath(import.meta.url));
  const APP_ROOT = join(SERVER_DIR, "..", "..");
  const FORBIDDEN = /refresh_token|offline_access|"refresh"/i;
  const ALLOWED: Readonly<Record<string, string>> = {
    "src/server/oauth.test.ts": "this scan, and the assertions above it",
    "src/server/api.ts":
      "a doc comment that says the token response has no refresh_token",
  };

  function sources(dir: string): string[] {
    return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
      if (entry.name === "node_modules" || entry.name === ".next") {
        return [];
      }
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        return sources(path);
      }
      return /\.(ts|tsx)$/.test(entry.name) ? [path] : [];
    });
  }

  it("finds the vocabulary only where it is allowed", () => {
    const found = [
      ...sources(join(APP_ROOT, "src")),
      ...sources(join(APP_ROOT, "app")),
    ].filter((path) => FORBIDDEN.test(readFileSync(path, "utf8")));
    const relative = found
      .map((path) => path.slice(APP_ROOT.length + 1))
      .sort();
    expect(relative).toEqual(Object.keys(ALLOWED).sort());
  });

  it("is not vacuous: it reads the source tree and would match the word", () => {
    expect(sources(join(APP_ROOT, "src")).length).toBeGreaterThan(20);
    expect(FORBIDDEN.test("grant_type=refresh_token")).toBe(true);
  });
});

describe("reading the authorization response", () => {
  it("accepts the registered URI carrying the expected state", () => {
    const result = readAuthorizationResponse(
      `${CONFIG.redirectUri}?code=abc&state=st`,
      CONFIG.redirectUri,
      "st",
    );
    expect(result).toEqual({ ok: true, value: "abc" });
  });

  it("refuses a redirect to anywhere else", () => {
    const result = readAuthorizationResponse(
      "https://evil.test/callback?code=abc&state=st",
      CONFIG.redirectUri,
      "st",
    );
    expect(result.ok).toBe(false);
  });

  it("refuses the wrong state", () => {
    const result = readAuthorizationResponse(
      `${CONFIG.redirectUri}?code=abc&state=other`,
      CONFIG.redirectUri,
      "st",
    );
    expect(result.ok).toBe(false);
  });

  it("reports an error response as the error it is, not as a missing code", () => {
    const result = readAuthorizationResponse(
      `${CONFIG.redirectUri}?error=invalid_scope&error_description=nope&state=st`,
      CONFIG.redirectUri,
      "st",
    );
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.failure.message).toContain("invalid_scope");
    }
  });
});
