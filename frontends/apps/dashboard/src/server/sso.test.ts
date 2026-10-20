/**
 * The two single-sign-on route handlers, driven against a stubbed `fetch`.
 *
 * Mocked: `next/headers` (the origin check reads `headers()`, which only
 * exists inside a request), the config (so `publicOrigin` is set), and the
 * authorization-code leg (`oauth.test.ts` is its suite). Everything else —
 * the cookie attributes, the redirect headers, the vpay requests — is the real
 * code.
 *
 * The decisive cases are the ones about tokens and codes:
 * `replaces the pending token with the NEW session` fails if the callback
 * reuses the token it was called with, and `never lets the code or state out`
 * fails if either reaches a `Location`, a body or a header of a failure.
 */
import { NextRequest } from "next/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DashboardConfig } from "../config/settings";
import {
  COOKIE_ATTRIBUTES,
  SESSION_COOKIE,
  SESSION_MAX_AGE_SECONDS,
  SSO_PENDING_MAX_AGE_SECONDS,
} from "./cookies";

const CONFIG: DashboardConfig = {
  apiBaseUrl: "http://vpay-server:8080",
  clientId: "vpay-dashboard",
  redirectUri: "https://dash.example/dash/v1/callback",
  scope: "dashboard:read",
  publicOrigin: "https://dash.example",
};

const mocks = vi.hoisted(() => ({
  origin: { value: null as string | null },
  completeAuthorizationCode: vi.fn(),
}));

vi.mock("next/headers", () => ({
  headers: () =>
    Promise.resolve({
      get: (name: string) =>
        name.toLowerCase() === "origin" ? mocks.origin.value : null,
    }),
}));
vi.mock("../config/runtime", () => ({
  dashboardConfig: () => ({ config: CONFIG, problems: [] }),
}));
vi.mock("./oauth", () => ({
  completeAuthorizationCode: mocks.completeAuthorizationCode,
}));

import {
  handleSsoCallback,
  handleSsoStart,
  safeAuthorizationUrl,
  SSO_FAILED_PATH,
} from "./sso";

const CODE = "IDP-CODE-do-not-leak-7f3a";
const STATE = "IDP-STATE-do-not-leak-91bc";
const PENDING = "pending-flow-token-AAAA";
const IDP_URL = "https://idp.example/realms/staff/auth?client_id=vpay";

interface Recorded {
  url: string;
  init: RequestInit | undefined;
}

/** A `fetch` that answers every vpay call with `answer`, recording each. */
function stubVpay(answer: () => Response | Error): {
  calls: Recorded[];
  fetchStub: ReturnType<typeof vi.fn>;
} {
  const calls: Recorded[] = [];
  const fetchStub = vi.fn(
    (input: string | URL | Request, init?: RequestInit) => {
      calls.push({
        url:
          typeof input === "string"
            ? input
            : input instanceof URL
              ? input.href
              : input.url,
        init,
      });
      const out = answer();
      return out instanceof Error ? Promise.reject(out) : Promise.resolve(out);
    },
  );
  vi.stubGlobal("fetch", fetchStub);
  return { calls, fetchStub };
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function callbackRequest(
  query: string,
  cookie: string | null = PENDING,
): NextRequest {
  return new NextRequest(`https://dash.example/api/auth/sso/callback${query}`, {
    headers: cookie === null ? {} : { cookie: `${SESSION_COOKIE}=${cookie}` },
  });
}

const GOOD_QUERY = `?code=${CODE}&state=${STATE}`;

/** Everything a response says, as one string, for "never appears" checks. */
async function everything(response: Response): Promise<string> {
  const headers = [...response.headers]
    .map(([k, v]) => `${k}: ${v}`)
    .join("\n");
  return `${response.status}\n${headers}\n${await response.text()}`;
}

beforeEach(() => {
  mocks.origin.value = "https://dash.example";
  mocks.completeAuthorizationCode.mockReset();
  mocks.completeAuthorizationCode.mockResolvedValue({
    ok: true,
    value: {
      access_token: "t",
      token_type: "Bearer",
      expires_in: 900,
      scope: "",
    },
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("safeAuthorizationUrl", () => {
  it("admits an absolute https and http URL, normalised", () => {
    expect(safeAuthorizationUrl(IDP_URL)).toBe(IDP_URL);
    expect(safeAuthorizationUrl("http://localhost:8081/auth")).toBe(
      "http://localhost:8081/auth",
    );
  });

  it.each([
    "javascript:alert(1)",
    "data:text/html,<script>1</script>",
    "ftp://idp.example/",
    "/login",
    "//evil.example/",
    "idp.example/auth",
    "",
    "https://user:pw@idp.example/",
    "https://user@idp.example/",
  ])("refuses %j", (url) => {
    expect(safeAuthorizationUrl(url)).toBeNull();
  });
});

describe("POST /api/auth/sso/start", () => {
  it("sets the pending token as a short-lived session cookie and redirects 303 to the IdP", async () => {
    const { calls } = stubVpay(() =>
      json({ session: PENDING, authorization_url: IDP_URL }),
    );

    const response = await handleSsoStart();

    expect(response.status).toBe(303);
    expect(response.headers.get("location")).toBe(IDP_URL);
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(response.headers.get("referrer-policy")).toBe("no-referrer");

    const cookie = response.cookies.get(SESSION_COOKIE);
    expect(cookie?.value).toBe(PENDING);
    expect(cookie?.maxAge).toBe(SSO_PENDING_MAX_AGE_SECONDS);
    expect(SSO_PENDING_MAX_AGE_SECONDS).toBe(600);
    expect(cookie?.httpOnly).toBe(COOKIE_ATTRIBUTES.httpOnly);
    expect(cookie?.secure).toBe(COOKIE_ATTRIBUTES.secure);
    expect(cookie?.sameSite).toBe(COOKIE_ATTRIBUTES.sameSite);
    expect(cookie?.path).toBe(COOKIE_ATTRIBUTES.path);

    // One call, to the start route, with no session header: nobody is signed
    // in yet and the pending token does not exist before this answer.
    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toBe(`${CONFIG.apiBaseUrl}/dash/v1/staff/sso/start`);
    expect(
      (calls[0]?.init?.headers as Record<string, string>)[
        "x-vpay-staff-session"
      ],
    ).toBeUndefined();
    // The token is in the cookie and nowhere a URL could carry it.
    expect(response.headers.get("location")).not.toContain(PENDING);
  });

  it.each([
    ["a foreign origin", "https://evil.example"],
    ["no origin at all", null],
    ["the literal null origin", "null"],
  ])("refuses %s before vpay is asked for anything", async (_n, origin) => {
    mocks.origin.value = origin;
    const { fetchStub } = stubVpay(() =>
      json({ session: PENDING, authorization_url: IDP_URL }),
    );

    const response = await handleSsoStart();

    expect(fetchStub).not.toHaveBeenCalled();
    expect(response.status).toBe(303);
    expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
    expect(response.cookies.get(SESSION_COOKIE)).toBeUndefined();
  });

  it.each([
    ["401", () => json({ error: { message: "x" } }, 401)],
    ["429", () => json({ error: { message: "x" } }, 429)],
    ["404", () => json({ error: { message: "x" } }, 404)],
    ["a network failure", () => new Error("ECONNREFUSED")],
    ["a body that is not JSON", () => new Response("<html>", { status: 200 })],
    ["a body with no session", () => json({ authorization_url: IDP_URL })],
  ])(
    "redirects to the generic failure on %s, setting no cookie",
    async (_n, answer) => {
      stubVpay(answer);
      const response = await handleSsoStart();
      expect(response.status).toBe(303);
      expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
      expect(response.headers.get("cache-control")).toBe("no-store");
      expect(response.cookies.get(SESSION_COOKIE)).toBeUndefined();
    },
  );

  it.each([
    "javascript:alert(1)",
    "ftp://idp.example/",
    "/relative/path",
    "https://user:pw@idp.example/",
  ])(
    "refuses an authorization_url of %j and keeps the pending token out of the cookie",
    async (url) => {
      stubVpay(() => json({ session: PENDING, authorization_url: url }));
      const response = await handleSsoStart();
      expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
      expect(response.cookies.get(SESSION_COOKIE)).toBeUndefined();
    },
  );
});

describe("GET /api/auth/sso/callback", () => {
  it("replaces the pending token with the NEW session, runs the code leg, and lands on /payments", async () => {
    const { calls } = stubVpay(() =>
      json({ outcome: "signed_in", session: "NEW-SESSION-TOKEN" }),
    );

    const response = await handleSsoCallback(callbackRequest(GOOD_QUERY));

    expect(response.status).toBe(303);
    expect(response.headers.get("location")).toBe("/payments");
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(response.headers.get("referrer-policy")).toBe("no-referrer");

    const cookie = response.cookies.get(SESSION_COOKIE);
    expect(cookie?.value).toBe("NEW-SESSION-TOKEN");
    expect(cookie?.value).not.toBe(PENDING);
    expect(cookie?.maxAge).toBe(SESSION_MAX_AGE_SECONDS);
    expect(cookie?.httpOnly).toBe(true);
    expect(cookie?.secure).toBe(true);
    expect(cookie?.sameSite).toBe("lax");
    expect(cookie?.path).toBe("/");

    // The code leg runs with the NEW token, not the pending one.
    expect(mocks.completeAuthorizationCode).toHaveBeenCalledTimes(1);
    expect(mocks.completeAuthorizationCode).toHaveBeenCalledWith(
      CONFIG,
      "NEW-SESSION-TOKEN",
    );

    // The one vpay call: pending token in the header, code and state in the
    // form body, nowhere else.
    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toBe(
      `${CONFIG.apiBaseUrl}/dash/v1/staff/sso/callback`,
    );
    const init = calls[0]?.init;
    expect(
      (init?.headers as Record<string, string>)["x-vpay-staff-session"],
    ).toBe(PENDING);
    const body = new URLSearchParams(init?.body as string);
    expect(body.get("code")).toBe(CODE);
    expect(body.get("state")).toBe(STATE);
    expect(calls[0]?.url).not.toContain(CODE);
    expect(await everything(response)).not.toContain(CODE);
    expect(await everything(response)).not.toContain(STATE);
  });

  it("still lands on /payments when the code leg fails, because the session is authenticated and the gate owns the rest", async () => {
    stubVpay(() =>
      json({ outcome: "signed_in", session: "NEW-SESSION-TOKEN" }),
    );
    mocks.completeAuthorizationCode.mockResolvedValue({
      ok: false,
      failure: { status: 401, message: "x", requestId: null },
    });

    const response = await handleSsoCallback(callbackRequest(GOOD_QUERY));

    expect(response.headers.get("location")).toBe("/payments");
    expect(response.cookies.get(SESSION_COOKIE)?.value).toBe(
      "NEW-SESSION-TOKEN",
    );
  });

  it("on reauthenticate, replaces the cookie with the new pending token and redirects to the IdP without running the code leg", async () => {
    const retry = "https://idp.example/realms/staff/auth?prompt=login";
    stubVpay(() =>
      json({
        outcome: "reauthenticate",
        session: "PENDING-2",
        authorization_url: retry,
      }),
    );

    const response = await handleSsoCallback(callbackRequest(GOOD_QUERY));

    expect(response.status).toBe(303);
    expect(response.headers.get("location")).toBe(retry);
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(response.headers.get("referrer-policy")).toBe("no-referrer");
    const cookie = response.cookies.get(SESSION_COOKIE);
    expect(cookie?.value).toBe("PENDING-2");
    expect(cookie?.value).not.toBe(PENDING);
    expect(cookie?.maxAge).toBe(SSO_PENDING_MAX_AGE_SECONDS);
    expect(mocks.completeAuthorizationCode).not.toHaveBeenCalled();
    expect(await everything(response)).not.toContain(CODE);
  });

  it("refuses a reauthenticate whose url is not an absolute http(s) URL, clearing the cookie", async () => {
    stubVpay(() =>
      json({
        outcome: "reauthenticate",
        session: "PENDING-2",
        authorization_url: "javascript:alert(1)",
      }),
    );
    const response = await handleSsoCallback(callbackRequest(GOOD_QUERY));
    expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
    expect(response.cookies.get(SESSION_COOKIE)?.value).toBe("");
    expect(response.cookies.get(SESSION_COOKIE)?.maxAge).toBe(0);
  });

  it.each([
    ["no code", `?state=${STATE}`],
    ["no state", `?code=${CODE}`],
    ["an empty code", `?code=&state=${STATE}`],
    ["an empty state", `?code=${CODE}&state=`],
    ["a duplicated code", `?code=${CODE}&code=other&state=${STATE}`],
    ["a duplicated state", `?code=${CODE}&state=${STATE}&state=other`],
    ["an error parameter", `?error=access_denied&error_description=No+way`],
    [
      "an error parameter beside a code and state",
      `?code=${CODE}&state=${STATE}&error=access_denied`,
    ],
    ["no parameters at all", ""],
  ])(
    "with %s: does not call vpay, clears the cookie, answers the generic failure",
    async (_n, query) => {
      const { fetchStub } = stubVpay(() =>
        json({ outcome: "signed_in", session: "NEW" }),
      );

      const response = await handleSsoCallback(callbackRequest(query));

      expect(fetchStub).not.toHaveBeenCalled();
      expect(mocks.completeAuthorizationCode).not.toHaveBeenCalled();
      expect(response.status).toBe(303);
      expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
      expect(response.headers.get("cache-control")).toBe("no-store");
      expect(response.headers.get("referrer-policy")).toBe("no-referrer");
      const cookie = response.cookies.get(SESSION_COOKIE);
      expect(cookie?.value).toBe("");
      expect(cookie?.maxAge).toBe(0);
      expect(cookie?.httpOnly).toBe(true);
      expect(cookie?.secure).toBe(true);
      expect(cookie?.path).toBe("/");
      const text = await everything(response);
      expect(text).not.toContain(CODE);
      expect(text).not.toContain(STATE);
      expect(text).not.toContain("access_denied");
      expect(text).not.toContain("No+way");
    },
  );

  it("with no pending cookie: does not call vpay and answers the generic failure", async () => {
    const { fetchStub } = stubVpay(() =>
      json({ outcome: "signed_in", session: "NEW" }),
    );
    const response = await handleSsoCallback(callbackRequest(GOOD_QUERY, null));
    expect(fetchStub).not.toHaveBeenCalled();
    expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
    expect(response.cookies.get(SESSION_COOKIE)?.maxAge).toBe(0);
  });

  it.each([
    ["a 401", () => json({ error: { message: "vpay says: bad state" } }, 401)],
    ["a 429", () => json({ error: { message: "slow down" } }, 429)],
    ["a 404", () => json({ error: { message: "not configured" } }, 404)],
    ["a 500", () => json({ error: { message: "oops" } }, 500)],
    ["a network failure", () => new Error(`ECONNREFUSED ${CODE}`)],
    ["a body that is not JSON", () => new Response("<html>", { status: 200 })],
    ["an unknown outcome", () => json({ outcome: "welcome", session: "x" })],
  ])(
    "on %s: clears the cookie and never lets the code or state out",
    async (_n, answer) => {
      stubVpay(answer);

      const response = await handleSsoCallback(callbackRequest(GOOD_QUERY));

      expect(response.status).toBe(303);
      expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
      expect(response.headers.get("cache-control")).toBe("no-store");
      expect(response.headers.get("referrer-policy")).toBe("no-referrer");
      const cookie = response.cookies.get(SESSION_COOKIE);
      expect(cookie?.value).toBe("");
      expect(cookie?.maxAge).toBe(0);
      expect(mocks.completeAuthorizationCode).not.toHaveBeenCalled();

      const text = await everything(response);
      expect(text).not.toContain(CODE);
      expect(text).not.toContain(STATE);
      expect(text).not.toContain(PENDING);
      // vpay's own sentence is not forwarded either: the redirect has no reason.
      expect(text).not.toContain("bad state");
      expect(text).not.toContain("slow down");
    },
  );

  it("logs nothing on any branch", async () => {
    const spies = (["log", "info", "warn", "error"] as const).map((m) =>
      vi.spyOn(console, m).mockImplementation(() => {}),
    );
    stubVpay(() => json({ error: { message: "x" } }, 401));
    await handleSsoCallback(callbackRequest(GOOD_QUERY));
    stubVpay(() => json({ outcome: "signed_in", session: "NEW" }));
    await handleSsoCallback(callbackRequest(GOOD_QUERY));
    await handleSsoCallback(callbackRequest("?error=x"));
    for (const spy of spies) {
      expect(spy).not.toHaveBeenCalled();
    }
  });
});
