/**
 * `GET /api/dash/<list>` for the procedure-backed lists, exercised as the
 * thing a browser reaches: a real `Request` in, a real `Response` out, one
 * stubbed `fetch` standing in for vpay. Its call moved onto the generated
 * CrateStack client on 2026-10-26; these are the properties that move must
 * not have cost, written the way `bff.test.ts` writes them for payments.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import type { DashboardConfig } from "../config/settings";
import { REFUNDS } from "../dash/resource-name";
import { procedureListResponse } from "./bff";
import { SESSION_COOKIE } from "./cookies";

const CONFIG: DashboardConfig = {
  apiBaseUrl: "http://vpay-server:8080",
  clientId: "vpay-dashboard",
  redirectUri: "http://localhost:3000/dash/v1/callback",
  scope: "dashboard:read",
  publicOrigin: "https://dash.example",
};

/** The value that must never leave this process. */
const BEARER = "bearer-token-that-must-never-be-served-5e2a";
const SESSION = "the-staff-session-token";

interface Seen {
  readonly url: string;
  readonly authorization: string | null;
  readonly body: string | null;
}

function stub(seen: Seen[]) {
  return vi.fn((input: string | URL | Request, init?: RequestInit) => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.href
          : input.url;
    seen.push({
      url,
      authorization: new Headers(init?.headers).get("authorization"),
      body: typeof init?.body === "string" ? init.body : null,
    });
    if (url.includes("/staff/session")) {
      return Promise.resolve(
        Response.json({
          staff_id: "stf_example_1",
          display_name: "Example Operator",
          email: "operator@example.test",
          merchant_id: "mch_example_1",
          password_change_required: false,
          access_token: BEARER,
          access_token_expires_at: new Date(Date.now() + 600_000).toISOString(),
          access_token_ttl_seconds: 900,
        }),
      );
    }
    // The procedure answers, and echoes the token wherever it can.
    return Promise.resolve(
      Response.json(
        {
          items: [{ id: "re_1" }],
          totalCount: 1,
          pageInfo: { hasNextPage: false, hasPreviousPage: false },
          leaked: BEARER,
        },
        { headers: { "x-echo": BEARER } },
      ),
    );
  });
}

const FROM_THE_PAGE = {
  host: "dash.example",
  "sec-fetch-site": "same-origin",
  cookie: `${SESSION_COOKIE}=${SESSION}`,
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the procedure-backed BFF routes", () => {
  it("reaches vpay not at all when the request carries no session", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal("fetch", stub(seen));

    const response = await procedureListResponse(
      new Request("https://dash.example/api/dash/refunds", {
        headers: { host: "dash.example", "sec-fetch-site": "same-origin" },
      }),
      REFUNDS,
      CONFIG,
    );

    expect(response.status).toBe(401);
    expect(seen).toEqual([]);
  });

  it("forwards limit and offset and nothing else the caller sent", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal("fetch", stub(seen));

    await procedureListResponse(
      new Request(
        "https://dash.example/api/dash/refunds?limit=5&offset=10&merchant_id=mch_other&status=failed&audience=x&scope=y",
        { headers: FROM_THE_PAGE },
      ),
      REFUNDS,
      CONFIG,
    );

    const call = seen.find((s) => s.url.includes("/$procs/"));
    expect(call?.url).toBe(
      "http://vpay-server:8080/dash/v1/$procs/searchRefunds",
    );
    expect(call?.authorization).toBe(`Bearer ${BEARER}`);
    expect(JSON.parse(call?.body ?? "null")).toEqual({
      page: { limit: 5, offset: 10 },
      filter: {},
    });
  });

  it("puts the bearer token in no header and no body", async () => {
    vi.stubGlobal("fetch", stub([]));

    const response = await procedureListResponse(
      new Request("https://dash.example/api/dash/refunds", {
        headers: FROM_THE_PAGE,
      }),
      REFUNDS,
      CONFIG,
    );
    const headers = [...response.headers.entries()]
      .map(([name, value]) => `${name}: ${value}`)
      .join("\n");
    const wire = `HTTP ${response.status}\n${headers}\n\n${await response.text()}`;

    expect(response.status).toBe(200);
    expect(headers).not.toContain(BEARER);
    expect(wire).not.toContain(BEARER);
    expect(JSON.parse(wire.slice(wire.indexOf("\n\n") + 2))).toMatchObject({
      object: "list",
      total_count: 1,
      limit: 20,
      offset: 0,
    });
  });
});
