/**
 * The gate every page behind the sign-in opens with, run against a stubbed
 * `fetch` and a stubbed cookie jar.
 *
 * `gate.test.ts` holds the pure decisions; this holds what the Next-bound half
 * does with them — which redirect, which cookie, which read — because the
 * decision "a session that has proved only a password goes to the code form,
 * not to the password form" is made *here*, from two reads, and no pure
 * function sees it.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const jar = vi.hoisted(() => ({ session: undefined as string | undefined }));

vi.mock("next/headers", () => ({
  cookies: () =>
    Promise.resolve({
      get: (name: string) =>
        name === "__Host-vpay_dash_session" && jar.session !== undefined
          ? { name, value: jar.session }
          : undefined,
    }),
}));
vi.mock("next/navigation", () => ({
  redirect: (to: string) => {
    throw new Error(`redirect:${to}`);
  },
}));
vi.mock("../config/runtime", () => ({
  dashboardConfig: () => ({
    config: {
      apiBaseUrl: "http://vpay:8080",
      clientId: "vpay-dashboard",
      redirectUri: "http://localhost:3000/dash/v1/callback",
      scope: "dashboard:read",
      publicOrigin: null,
    },
    problems: [],
    apiBaseUrl: "http://vpay:8080",
  }),
}));

import { hangingVpay } from "../testing/hanging-fetch";
import { alreadySignedIn, requireStaff } from "./session";

const SESSION = {
  staff_id: "stf_1",
  display_name: "Ada",
  email: "ada@acme.test",
  merchant_id: "acme",
  password_change_required: false,
  access_token: "header.payload.sig",
  access_token_expires_at: new Date(Date.now() + 600_000).toISOString(),
  access_token_ttl_seconds: 900,
};

/** A `fetch` input as the URL it names (`String(request)` would not be one). */
function urlOf(input: string | URL | Request): string {
  if (typeof input === "string") {
    return input;
  }
  return input instanceof URL ? input.href : input.url;
}

/** A routing `fetch`: path -> status and body. Records every path asked. */
function vpay(routes: Record<string, { status: number; body?: unknown }>) {
  const asked: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((input: string | URL | Request) => {
      const url = new URL(urlOf(input));
      asked.push(url.pathname);
      const hit = routes[url.pathname];
      if (hit === undefined) {
        return Promise.reject(new Error(`unrouted ${url.pathname}`));
      }
      return Promise.resolve(
        new Response(JSON.stringify(hit.body ?? {}), {
          status: hit.status,
          headers: { "request-id": "req_gate" },
        }),
      );
    }),
  );
  return asked;
}

async function destination(): Promise<string> {
  try {
    const gate = await requireStaff();
    return `render:${gate.kind}`;
  } catch (error) {
    return (error as Error).message;
  }
}

beforeEach(() => {
  jar.session = "sess-1";
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("the gate", () => {
  it("sends a browser with no cookie to the sign-in without asking vpay anything", async () => {
    jar.session = undefined;
    const asked = vpay({});
    expect(await destination()).toBe("redirect:/login");
    expect(asked).toEqual([]);
  });

  it("lets a signed-in session through with the token on its row", async () => {
    vpay({ "/dash/v1/staff/session": { status: 200, body: SESSION } });
    expect(await destination()).toBe("render:ready");
  });

  it("sends a session still owing the printed password to /login/password", async () => {
    vpay({
      "/dash/v1/staff/session": {
        status: 200,
        body: { ...SESSION, password_change_required: true },
      },
    });
    expect(await destination()).toBe("redirect:/login/password");
  });

  it("mints a token when the row has none, and reads no stage", async () => {
    const asked = vpay({
      "/dash/v1/staff/session": {
        status: 200,
        body: { ...SESSION, access_token: undefined },
      },
    });
    // The authorize leg is not routed, so the mint fails as an outage; what is
    // asserted is that the gate went to /authorize and not to the stage read.
    expect(await destination()).toBe("render:outage");
    expect(asked).toContain("/dash/v1/oauth/authorize");
    expect(asked).not.toContain("/dash/v1/staff/session/stage");
  });
});

describe("a signed-in session whose token vpay will not mint", () => {
  // The session read is 200 and `/oauth/authorize` is 401 — a staff member
  // whose merchant is not the one the dashboard client is bound to
  // (`vpay_api::staff::oauth::authorize`'s fourth refusal). Until the VPA-5a
  // review the gate answered this with `redirect("/signed-out")`; the browser
  // went `/` -> `/signed-out` (which clears nothing on a non-navigation
  // request) -> `/login` (`alreadySignedIn`) -> `/`, hitting vpay on every
  // pass and showing nothing.
  const NO_TOKEN = { ...SESSION, access_token: undefined };
  const REFUSAL = {
    status: 401,
    body: { error: { message: "We could not sign you in." } },
  };

  it("renders the refusal and redirects nowhere — THE DECISIVE MUTATION: restore redirect(SIGNED_OUT_PATH)", async () => {
    const asked = vpay({
      "/dash/v1/staff/session": { status: 200, body: NO_TOKEN },
      "/dash/v1/oauth/authorize": REFUSAL,
    });
    const gate = await requireStaff();
    expect(gate).toMatchObject({
      kind: "refused",
      failure: { status: 401, message: "We could not sign you in." },
    });
    // One session read and one mint, no stage read, nothing past them.
    expect(asked).toEqual([
      "/dash/v1/staff/session",
      "/dash/v1/oauth/authorize",
    ]);
  });

  it("does not fall back to the stale token it holds — vpay has just said no", async () => {
    // Token inside the re-mint margin, still unexpired: the one case where a
    // fallback could have rendered. A 401 on the mint is a refusal of the
    // session, and the token on its row is not to be read on.
    vpay({
      "/dash/v1/staff/session": {
        status: 200,
        body: {
          ...SESSION,
          access_token_expires_at: new Date(Date.now() + 60_000).toISOString(),
        },
      },
      "/dash/v1/oauth/authorize": REFUSAL,
    });
    expect(await destination()).toBe("render:refused");
  });

  it("does fall back to the stale token when vpay merely could not be reached for the mint", async () => {
    vpay({
      "/dash/v1/staff/session": {
        status: 200,
        body: {
          ...SESSION,
          access_token_expires_at: new Date(Date.now() + 60_000).toISOString(),
        },
      },
      "/dash/v1/oauth/authorize": { status: 503 },
    });
    expect(await destination()).toBe("render:ready");
  });

  it("is an outage, not a refusal, when there is no token to fall back to and vpay answers 503", async () => {
    vpay({
      "/dash/v1/staff/session": { status: 200, body: NO_TOKEN },
      "/dash/v1/oauth/authorize": { status: 503 },
    });
    expect(await destination()).toBe("render:outage");
  });
});

describe("a session the read refused", () => {
  it("goes to the code form when the stage says it is still owed a code — pending_totp", async () => {
    // Somebody who typed a password and then opened `/` in a new tab is owed
    // the code form, not the password form again. THE DECISIVE MUTATION:
    // send every 401 to /signed-out.
    vpay({
      "/dash/v1/staff/session": { status: 401 },
      "/dash/v1/staff/session/stage": {
        status: 200,
        body: { stage: "pending_totp" },
      },
    });
    expect(await destination()).toBe("redirect:/login/totp");
  });

  it("is signed out when the stage read is refused too", async () => {
    vpay({
      "/dash/v1/staff/session": { status: 401 },
      "/dash/v1/staff/session/stage": { status: 401 },
    });
    expect(await destination()).toBe("redirect:/signed-out");
  });

  it("is signed out when the deployment no longer serves human_login — 404 is the method off", async () => {
    // The staff routes are not mounted; the session read is the honest 404. The
    // cookie has no session behind it and never will, so it is forgotten.
    vpay({
      "/dash/v1/staff/session": { status: 404 },
      "/dash/v1/staff/session/stage": { status: 404 },
    });
    expect(await destination()).toBe("redirect:/signed-out");
  });

  it("is an outage, with the cookie kept, when vpay answers 503 to the session read", async () => {
    const asked = vpay({ "/dash/v1/staff/session": { status: 503 } });
    expect(await destination()).toBe("render:outage");
    // Not even a stage read: a vpay that cannot answer one cannot answer the other.
    expect(asked).toEqual(["/dash/v1/staff/session"]);
  });

  it("is an outage when the stage read is what vpay could not answer", async () => {
    vpay({
      "/dash/v1/staff/session": { status: 401 },
      "/dash/v1/staff/session/stage": { status: 502 },
    });
    expect(await destination()).toBe("render:outage");
  });
});

describe("a vpay that never answers", () => {
  it("is an outage with the cookie kept — the session read times out", async () => {
    const { timeOut } = hangingVpay();
    const pending = requireStaff();
    // Let the gate reach its `fetch` before the deadline fires.
    await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
    timeOut();
    expect(await pending).toMatchObject({
      kind: "outage",
      failure: { status: 0 },
    });
  });
});

describe("the login page's question: is this browser already signed in?", () => {
  it("yes for a session, including one that still owes the printed password", async () => {
    vpay({
      "/dash/v1/staff/session": {
        status: 200,
        body: { ...SESSION, password_change_required: true },
      },
    });
    expect(await alreadySignedIn()).toBe(true);
  });

  it("no for a refused session, and no for no cookie", async () => {
    vpay({ "/dash/v1/staff/session": { status: 401 } });
    expect(await alreadySignedIn()).toBe(false);
    jar.session = undefined;
    expect(await alreadySignedIn()).toBe(false);
  });
});
