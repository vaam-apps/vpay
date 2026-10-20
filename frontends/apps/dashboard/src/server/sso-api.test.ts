/**
 * The three single-sign-on calls, against a stubbed `fetch`.
 *
 * What is asserted is the request this app puts on the wire (path, method,
 * form body, the session header) and how each answer maps — in particular
 * that `getSsoStatus` cannot fail loudly, because the password form must
 * render whatever it does.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import { getSsoStatus, ssoCallback, ssoStart } from "./api";

const BASE = "http://vpay-server:8080";

/** A `fetch` stub that keeps its own signature, so `mock.calls` is typed. */
function stub(answer: () => Response | Promise<Response>) {
  return vi.fn((_input: string | URL | Request, _init?: RequestInit) =>
    Promise.resolve(answer()),
  );
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function refusal(status: number): Response {
  return json({ error: { message: "We could not sign you in." } }, status);
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("getSsoStatus", () => {
  it("reads enabled: true from GET /dash/v1/staff/sso", async () => {
    const fetchStub = stub(() => json({ enabled: true }));
    vi.stubGlobal("fetch", fetchStub);

    expect(await getSsoStatus(BASE)).toEqual({ enabled: true });
    expect(fetchStub.mock.calls[0]?.[0]).toBe(`${BASE}/dash/v1/staff/sso`);
    expect(fetchStub.mock.calls[0]?.[1]?.method).toBe("GET");
  });

  it("reads enabled: false", async () => {
    vi.stubGlobal(
      "fetch",
      stub(() => json({ enabled: false })),
    );
    expect(await getSsoStatus(BASE)).toEqual({ enabled: false });
  });

  it.each([404, 401, 429, 500, 502])("fails soft on a %i", async (status) => {
    vi.stubGlobal(
      "fetch",
      stub(() => refusal(status)),
    );
    expect(await getSsoStatus(BASE)).toEqual({ enabled: false });
  });

  it("fails soft on a network failure, and logs nothing", async () => {
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const info = vi.spyOn(console, "info").mockImplementation(() => {});
    const out = vi.spyOn(console, "log").mockImplementation(() => {});
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.reject(new Error("ECONNREFUSED"))),
    );

    expect(await getSsoStatus(BASE)).toEqual({ enabled: false });
    for (const spy of [log, warn, info, out]) {
      expect(spy).not.toHaveBeenCalled();
      spy.mockRestore();
    }
  });

  it.each([
    ["a body that is not JSON", () => new Response("<html>", { status: 200 })],
    ["null", () => json(null)],
    ["an array", () => json([true])],
    ["enabled as a string", () => json({ enabled: "true" })],
    ["enabled as 1", () => json({ enabled: 1 })],
    ["no enabled field", () => json({})],
  ])("fails soft on %s", async (_name, answer) => {
    vi.stubGlobal("fetch", stub(answer));
    expect(await getSsoStatus(BASE)).toEqual({ enabled: false });
  });
});

describe("ssoStart", () => {
  it("posts an empty form with no session header and returns both values", async () => {
    const fetchStub = stub(() =>
      json({ session: "pending-1", authorization_url: "https://idp/auth?x=1" }),
    );
    vi.stubGlobal("fetch", fetchStub);

    const result = await ssoStart(BASE);

    expect(result).toEqual({
      ok: true,
      value: {
        session: "pending-1",
        authorization_url: "https://idp/auth?x=1",
      },
    });
    expect(fetchStub.mock.calls[0]?.[0]).toBe(
      `${BASE}/dash/v1/staff/sso/start`,
    );
    const init = fetchStub.mock.calls[0]?.[1];
    expect(init?.method).toBe("POST");
    expect(init?.body).toBe("");
    const headers = init?.headers as Record<string, string>;
    expect(headers["x-vpay-staff-session"]).toBeUndefined();
    expect(headers["content-type"]).toBe("application/x-www-form-urlencoded");
  });

  it.each([401, 429, 404])("maps a %i to a failure carrying it", async (s) => {
    vi.stubGlobal(
      "fetch",
      stub(() => refusal(s)),
    );
    const result = await ssoStart(BASE);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.failure.status).toBe(s);
    }
  });

  it("maps a network failure to a status-0 failure rather than throwing", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.reject(new Error("boom"))),
    );
    const result = await ssoStart(BASE);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.failure.status).toBe(0);
    }
  });

  it.each([
    ["no session", { authorization_url: "https://idp/" }],
    ["an empty session", { session: "", authorization_url: "https://idp/" }],
    ["no url", { session: "p" }],
    ["a non-string url", { session: "p", authorization_url: 5 }],
  ])("refuses a 200 with %s", async (_name, body) => {
    vi.stubGlobal(
      "fetch",
      stub(() => json(body)),
    );
    const result = await ssoStart(BASE);
    expect(result.ok).toBe(false);
  });
});

describe("ssoCallback", () => {
  it("sends code and state as a form, and the pending token in the session header", async () => {
    const fetchStub = stub(() =>
      json({ outcome: "signed_in", session: "session-2" }),
    );
    vi.stubGlobal("fetch", fetchStub);

    const result = await ssoCallback(BASE, "pending-1", "the code", "st&te");

    expect(result).toEqual({
      ok: true,
      value: { outcome: "signed_in", session: "session-2" },
    });
    expect(fetchStub.mock.calls[0]?.[0]).toBe(
      `${BASE}/dash/v1/staff/sso/callback`,
    );
    const init = fetchStub.mock.calls[0]?.[1];
    expect(init?.method).toBe("POST");
    const body = new URLSearchParams(init?.body as string);
    expect(body.get("code")).toBe("the code");
    expect(body.get("state")).toBe("st&te");
    expect([...body.keys()].sort()).toEqual(["code", "state"]);
    expect(
      (init?.headers as Record<string, string>)["x-vpay-staff-session"],
    ).toBe("pending-1");
  });

  it("reads the reauthenticate outcome with its new pending token and url", async () => {
    vi.stubGlobal(
      "fetch",
      stub(() =>
        json({
          outcome: "reauthenticate",
          session: "pending-2",
          authorization_url: "https://idp/auth?prompt=login",
        }),
      ),
    );
    expect(await ssoCallback(BASE, "pending-1", "c", "s")).toEqual({
      ok: true,
      value: {
        outcome: "reauthenticate",
        session: "pending-2",
        authorization_url: "https://idp/auth?prompt=login",
      },
    });
  });

  it.each([401, 429, 404])("maps a %i to a failure carrying it", async (s) => {
    vi.stubGlobal(
      "fetch",
      stub(() => refusal(s)),
    );
    const result = await ssoCallback(BASE, "p", "c", "s");
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.failure.status).toBe(s);
    }
  });

  it("does not put the code or state into a failure it builds", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.reject(new Error("boom"))),
    );
    const result = await ssoCallback(BASE, "p", "CODE-XYZ", "STATE-XYZ");
    expect(JSON.stringify(result)).not.toContain("CODE-XYZ");
    expect(JSON.stringify(result)).not.toContain("STATE-XYZ");
  });

  it.each([
    ["an unknown outcome", { outcome: "welcome", session: "s" }],
    ["signed_in with no session", { outcome: "signed_in" }],
    ["signed_in with an empty session", { outcome: "signed_in", session: "" }],
    ["reauthenticate with no url", { outcome: "reauthenticate", session: "s" }],
    ["no outcome", { session: "s" }],
  ])("refuses a 200 with %s", async (_name, body) => {
    vi.stubGlobal(
      "fetch",
      stub(() => json(body)),
    );
    expect((await ssoCallback(BASE, "p", "c", "s")).ok).toBe(false);
  });
});
