/**
 * The two single-sign-on route handlers, driven against a stubbed `fetch`.
 *
 * Mocked: `next/headers` (the origin check reads `headers()`, which only
 * exists inside a request) and the config (so the public origin and the
 * issuer origin are set). Everything else — the cookie attributes, the
 * redirect headers, the vpay requests, `dash-fetch`'s credential — is the real
 * code.
 *
 * The decisive cases, each with the sabotage that proves it can fail
 * (`docs/status/verification/2026-10-30-vpa5b-dashboard-sso.md`):
 * `refuses a foreign origin before vpay is asked` (origin check removed),
 * `does not call vpay when the provider sent error=` (error stopped late),
 * `carries no-store and no-referrer on every response` (header dropped),
 * `refuses an authorization_url …` (provider URL not validated), the cookie
 * attribute cases (pending cookie not `__Host-`/httpOnly), and
 * `replaces whatever session the browser held with the NEW token` (session
 * not replaced on `signed_in`).
 */
import { NextRequest } from "next/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DashboardConfig } from "../config/settings";
import {
  COOKIE_ATTRIBUTES,
  SESSION_COOKIE,
  SESSION_MAX_AGE_SECONDS,
  SSO_PENDING_COOKIE,
  SSO_PENDING_MAX_AGE_SECONDS,
} from "./cookies";

const CONFIG: DashboardConfig = {
  apiBaseUrl: "http://vpay-server:8080",
  clientId: "vpay-dashboard",
  redirectUri: "https://dash.example/dash/v1/callback",
  scope: "dashboard:read",
};

const mocks = vi.hoisted(() => ({
  origin: { value: null as string | null },
  assembled: { value: {} },
}));

vi.mock("next/headers", () => ({
  headers: () =>
    Promise.resolve({
      get: (name: string) =>
        name.toLowerCase() === "origin" ? mocks.origin.value : null,
    }),
}));
vi.mock("../config/runtime", () => ({
  dashboardConfig: () => mocks.assembled.value,
}));

import {
  handleSsoCallback,
  handleSsoStart,
  safeAuthorizationUrl,
  SSO_FAILED_PATH,
} from "./sso";

const ISSUER_ORIGIN = "https://idp.example";
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
  cookies: Record<string, string> = { [SSO_PENDING_COOKIE]: PENDING },
): NextRequest {
  const cookie = Object.entries(cookies)
    .map(([name, value]) => `${name}=${value}`)
    .join("; ");
  return new NextRequest(`https://dash.example/api/auth/sso/callback${query}`, {
    headers: cookie === "" ? {} : { cookie },
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

/** The attributes a `__Host-` cookie needs, read off a response's cookie. */
function expectHostCookie(
  cookie: ReturnType<NextRequest["cookies"]["get"]> & object,
): void {
  expect(cookie.name.startsWith("__Host-")).toBe(true);
  expect(cookie).toMatchObject({
    httpOnly: COOKIE_ATTRIBUTES.httpOnly,
    secure: COOKIE_ATTRIBUTES.secure,
    sameSite: COOKIE_ATTRIBUTES.sameSite,
    path: COOKIE_ATTRIBUTES.path,
  });
  // `__Host-` forbids a Domain.
  expect(cookie).not.toHaveProperty("domain");
}

beforeEach(() => {
  mocks.origin.value = "https://dash.example";
  mocks.assembled.value = {
    config: CONFIG,
    problems: [],
    apiBaseUrl: CONFIG.apiBaseUrl,
    publicOrigin: "https://dash.example",
    ssoIssuerOrigin: ISSUER_ORIGIN,
  };
  vi.spyOn(console, "warn").mockImplementation(() => {});
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("safeAuthorizationUrl", () => {
  it("admits an https URL on the issuer's origin, normalised", () => {
    expect(safeAuthorizationUrl(IDP_URL, ISSUER_ORIGIN)).toBe(IDP_URL);
  });

  it("admits plain http only for a loopback issuer (a development stack)", () => {
    expect(
      safeAuthorizationUrl(
        "http://localhost:8180/realms/staff/auth",
        "http://localhost:8180",
      ),
    ).toBe("http://localhost:8180/realms/staff/auth");
    expect(
      safeAuthorizationUrl("http://[::1]:8180/auth", "http://[::1]:8180"),
    ).toBe("http://[::1]:8180/auth");
    // A non-loopback http provider is refused even if the issuer says so.
    expect(
      safeAuthorizationUrl("http://idp.example/auth", "http://idp.example"),
    ).toBeNull();
  });

  it.each([
    ["javascript:", "javascript:alert(1)"],
    ["data:", "data:text/html,<script>1</script>"],
    ["ftp:", "ftp://idp.example/"],
    ["a relative path", "/login"],
    ["a scheme-relative URL", "//evil.example/"],
    ["a bare host", "idp.example/auth"],
    ["an empty string", ""],
    ["userinfo with a password", "https://user:pw@idp.example/auth"],
    ["userinfo without one", "https://user@idp.example/auth"],
    ["a fragment", "https://idp.example/auth#access_token=x"],
    ["an empty fragment", "https://idp.example/auth#"],
    ["another host", "https://evil.example/auth"],
    ["a look-alike host", "https://idp.example.evil.example/auth"],
    ["a suffix-trick host", "https://evil.example/https://idp.example/auth"],
    ["another port", "https://idp.example:8443/auth"],
    ["another scheme on the issuer's host", "http://idp.example/auth"],
  ])("refuses %s", (_name, url) => {
    expect(safeAuthorizationUrl(url, ISSUER_ORIGIN)).toBeNull();
  });

  it("refuses everything when the issuer's origin is not known", () => {
    expect(safeAuthorizationUrl(IDP_URL, null)).toBeNull();
  });
});

describe("POST /api/auth/sso/start", () => {
  it("sets the pending token as a __Host- httpOnly cookie for ten minutes and redirects 303 to the IdP", async () => {
    const { calls } = stubVpay(() =>
      json({ session: PENDING, authorization_url: IDP_URL }),
    );

    const response = await handleSsoStart();

    expect(response.status).toBe(303);
    expect(response.headers.get("location")).toBe(IDP_URL);

    const cookie = response.cookies.get(SSO_PENDING_COOKIE);
    expect(cookie?.value).toBe(PENDING);
    expect(cookie?.maxAge).toBe(SSO_PENDING_MAX_AGE_SECONDS);
    expect(SSO_PENDING_MAX_AGE_SECONDS).toBe(600);
    expect(cookie).toBeDefined();
    if (cookie !== undefined) {
      expectHostCookie(cookie);
    }
    // It never touches the session's slot.
    expect(response.cookies.get(SESSION_COOKIE)).toBeUndefined();

    // One call, to the start route, carrying no credential at all: nobody is
    // signed in yet and the pending token does not exist before this answer.
    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toBe(`${CONFIG.apiBaseUrl}/dash/v1/staff/sso/start`);
    const headers = calls[0]?.init?.headers as Record<string, string>;
    expect(headers["x-vpay-staff-session"]).toBeUndefined();
    expect(headers["authorization"]).toBeUndefined();
    // The token is in the cookie and nowhere a URL could carry it.
    expect(response.headers.get("location")).not.toContain(PENDING);
  });

  it("carries no-store and no-referrer on every response", async () => {
    stubVpay(() => json({ session: PENDING, authorization_url: IDP_URL }));
    const ok = await handleSsoStart();

    stubVpay(() => json({ error: { message: "x" } }, 401));
    const refused = await handleSsoStart();

    mocks.origin.value = "https://evil.example";
    const crossOrigin = await handleSsoStart();

    for (const response of [ok, refused, crossOrigin]) {
      expect(response.headers.get("cache-control")).toBe("no-store");
      expect(response.headers.get("referrer-policy")).toBe("no-referrer");
    }
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
    expect(response.cookies.get(SSO_PENDING_COOKIE)).toBeUndefined();
    expect(response.cookies.get(SESSION_COOKIE)).toBeUndefined();
  });

  it("refuses, without asking vpay, when this container has no client registration", async () => {
    mocks.assembled.value = {
      config: null,
      problems: [],
      apiBaseUrl: CONFIG.apiBaseUrl,
      publicOrigin: "https://dash.example",
      ssoIssuerOrigin: ISSUER_ORIGIN,
    };
    const { fetchStub } = stubVpay(() => json({}));
    const response = await handleSsoStart();
    expect(fetchStub).not.toHaveBeenCalled();
    expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
  });

  it("refuses, without asking vpay, when it does not know the provider's origin", async () => {
    mocks.assembled.value = {
      config: CONFIG,
      problems: [],
      apiBaseUrl: CONFIG.apiBaseUrl,
      publicOrigin: "https://dash.example",
      ssoIssuerOrigin: null,
    };
    const { fetchStub } = stubVpay(() => json({}));
    const response = await handleSsoStart();
    expect(fetchStub).not.toHaveBeenCalled();
    expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
    expect(response.cookies.get(SSO_PENDING_COOKIE)).toBeUndefined();
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
      expect(response.cookies.get(SSO_PENDING_COOKIE)).toBeUndefined();
      expect(response.cookies.get(SESSION_COOKIE)).toBeUndefined();
    },
  );

  it.each([
    "javascript:alert(1)",
    "ftp://idp.example/",
    "/relative/path",
    "https://user:pw@idp.example/",
    "https://evil.example/auth",
    "https://idp.example/auth#frag",
    "http://idp.example/auth",
  ])(
    "refuses an authorization_url of %j and keeps the pending token out of the cookie",
    async (url) => {
      stubVpay(() => json({ session: PENDING, authorization_url: url }));
      const response = await handleSsoStart();
      expect(response.status).toBe(303);
      expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
      expect(response.cookies.get(SSO_PENDING_COOKIE)).toBeUndefined();
    },
  );

  it("takes no destination from the request — there is no return_to", async () => {
    stubVpay(() => json({ session: PENDING, authorization_url: IDP_URL }));
    const response = await handleSsoStart();
    // The only absolute Location is the provider's; nothing the request said
    // could change it, because the handler reads no request at all.
    expect(handleSsoStart.length).toBe(0);
    expect(response.headers.get("location")).toBe(IDP_URL);
  });
});

describe("GET /api/auth/sso/callback", () => {
  it("replaces whatever session the browser held with the NEW token, clears the pending cookie, and lands on /", async () => {
    const { calls } = stubVpay(() =>
      json({ outcome: "signed_in", session: "NEW-SESSION-TOKEN" }),
    );

    // The browser already holds a session — one planted before the flow.
    const response = await handleSsoCallback(
      callbackRequest(GOOD_QUERY, {
        [SSO_PENDING_COOKIE]: PENDING,
        [SESSION_COOKIE]: "PLANTED-SESSION",
      }),
    );

    expect(response.status).toBe(303);
    expect(response.headers.get("location")).toBe("/");
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(response.headers.get("referrer-policy")).toBe("no-referrer");

    const session = response.cookies.get(SESSION_COOKIE);
    expect(session?.value).toBe("NEW-SESSION-TOKEN");
    expect(session?.value).not.toBe(PENDING);
    expect(session?.value).not.toBe("PLANTED-SESSION");
    expect(session?.maxAge).toBe(SESSION_MAX_AGE_SECONDS);
    expect(session).toBeDefined();
    if (session !== undefined) {
      expectHostCookie(session);
    }

    const pending = response.cookies.get(SSO_PENDING_COOKIE);
    expect(pending?.value).toBe("");
    expect(pending?.maxAge).toBe(0);
    expect(pending).toBeDefined();
    if (pending !== undefined) {
      expectHostCookie(pending);
    }

    // The one vpay call: the pending token as the ONE credential, a session
    // header and no `Authorization`; code and state in the form body only.
    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toBe(
      `${CONFIG.apiBaseUrl}/dash/v1/staff/sso/callback`,
    );
    const init = calls[0]?.init;
    const headers = init?.headers as Record<string, string>;
    expect(headers["x-vpay-staff-session"]).toBe(PENDING);
    expect(headers["authorization"]).toBeUndefined();
    const body = new URLSearchParams(init?.body as string);
    expect(body.get("code")).toBe(CODE);
    expect(body.get("state")).toBe(STATE);
    expect(calls[0]?.url).not.toContain(CODE);
    expect(await everything(response)).not.toContain(CODE);
    expect(await everything(response)).not.toContain(STATE);
  });

  it("makes no second call to vpay on signed_in — the gate on / mints", async () => {
    const { calls } = stubVpay(() =>
      json({ outcome: "signed_in", session: "NEW-SESSION-TOKEN" }),
    );
    await handleSsoCallback(callbackRequest(GOOD_QUERY));
    expect(calls).toHaveLength(1);
  });

  it("on reauthenticate, replaces the pending cookie with the new token and redirects to the IdP", async () => {
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
    const cookie = response.cookies.get(SSO_PENDING_COOKIE);
    expect(cookie?.value).toBe("PENDING-2");
    expect(cookie?.value).not.toBe(PENDING);
    expect(cookie?.maxAge).toBe(SSO_PENDING_MAX_AGE_SECONDS);
    expect(response.cookies.get(SESSION_COOKIE)).toBeUndefined();
    expect(await everything(response)).not.toContain(CODE);
  });

  it.each([
    "javascript:alert(1)",
    "https://evil.example/auth?prompt=login",
    "https://user:pw@idp.example/auth",
  ])(
    "refuses a reauthenticate whose url is %j, clearing the pending cookie",
    async (url) => {
      stubVpay(() =>
        json({
          outcome: "reauthenticate",
          session: "PENDING-2",
          authorization_url: url,
        }),
      );
      const response = await handleSsoCallback(callbackRequest(GOOD_QUERY));
      expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
      expect(response.cookies.get(SSO_PENDING_COOKIE)?.value).toBe("");
      expect(response.cookies.get(SSO_PENDING_COOKIE)?.maxAge).toBe(0);
    },
  );

  it.each([
    ["no code", `?state=${STATE}`],
    ["no state", `?code=${CODE}`],
    ["an empty code", `?code=&state=${STATE}`],
    ["an empty state", `?code=${CODE}&state=`],
    ["a duplicated code", `?code=${CODE}&code=other&state=${STATE}`],
    ["a duplicated state", `?code=${CODE}&state=${STATE}&state=other`],
    ["no parameters at all", ""],
  ])(
    "with %s: does not call vpay, clears the pending cookie, answers the generic failure",
    async (_n, query) => {
      const { fetchStub } = stubVpay(() =>
        json({ outcome: "signed_in", session: "NEW" }),
      );

      const response = await handleSsoCallback(callbackRequest(query));

      expect(fetchStub).not.toHaveBeenCalled();
      expect(response.status).toBe(303);
      expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
      expect(response.headers.get("cache-control")).toBe("no-store");
      expect(response.headers.get("referrer-policy")).toBe("no-referrer");
      const cookie = response.cookies.get(SSO_PENDING_COOKIE);
      expect(cookie?.value).toBe("");
      expect(cookie?.maxAge).toBe(0);
      expect(cookie).toBeDefined();
      if (cookie !== undefined) {
        expectHostCookie(cookie);
      }
      expect(response.cookies.get(SESSION_COOKIE)).toBeUndefined();
      const text = await everything(response);
      expect(text).not.toContain(CODE);
      expect(text).not.toContain(STATE);
    },
  );

  it.each([
    ["error beside nothing", `?error=access_denied&error_description=No+way`],
    [
      "error beside a code and state",
      `?code=${CODE}&state=${STATE}&error=access_denied`,
    ],
    ["an empty error", `?code=${CODE}&state=${STATE}&error=`],
  ])("does not call vpay when the provider sent %s", async (_n, query) => {
    const { fetchStub } = stubVpay(() =>
      json({ outcome: "signed_in", session: "NEW" }),
    );

    const response = await handleSsoCallback(callbackRequest(query));

    expect(fetchStub).not.toHaveBeenCalled();
    expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
    expect(response.cookies.get(SSO_PENDING_COOKIE)?.maxAge).toBe(0);
    const text = await everything(response);
    expect(text).not.toContain("access_denied");
    expect(text).not.toContain("No+way");
    expect(text).not.toContain(CODE);
  });

  it("with no pending cookie: does not call vpay and answers the generic failure", async () => {
    const { fetchStub } = stubVpay(() =>
      json({ outcome: "signed_in", session: "NEW" }),
    );
    // A session cookie is NOT a pending token, whatever it holds.
    const response = await handleSsoCallback(
      callbackRequest(GOOD_QUERY, { [SESSION_COOKIE]: "SOME-SESSION" }),
    );
    expect(fetchStub).not.toHaveBeenCalled();
    expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
    expect(response.cookies.get(SSO_PENDING_COOKIE)?.maxAge).toBe(0);
    expect(response.cookies.get(SESSION_COOKIE)).toBeUndefined();
  });

  it("with no client registration: does not call vpay", async () => {
    mocks.assembled.value = {
      config: null,
      problems: [],
      apiBaseUrl: CONFIG.apiBaseUrl,
      publicOrigin: null,
      ssoIssuerOrigin: ISSUER_ORIGIN,
    };
    const { fetchStub } = stubVpay(() => json({}));
    const response = await handleSsoCallback(callbackRequest(GOOD_QUERY));
    expect(fetchStub).not.toHaveBeenCalled();
    expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
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
    "on %s: clears the pending cookie and never lets the code or state out",
    async (_n, answer) => {
      stubVpay(answer);

      const response = await handleSsoCallback(callbackRequest(GOOD_QUERY));

      expect(response.status).toBe(303);
      expect(response.headers.get("location")).toBe(SSO_FAILED_PATH);
      expect(response.headers.get("cache-control")).toBe("no-store");
      expect(response.headers.get("referrer-policy")).toBe("no-referrer");
      const cookie = response.cookies.get(SSO_PENDING_COOKIE);
      expect(cookie?.value).toBe("");
      expect(cookie?.maxAge).toBe(0);
      // A refused flow replaced nothing, so it signs nobody out.
      expect(response.cookies.get(SESSION_COOKIE)).toBeUndefined();

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
