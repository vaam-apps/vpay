/**
 * The generated client's procedures, as this app calls them — stubbed at the
 * network boundary and nowhere else, so the generated runtime, the re-mint
 * and the failure mapping are all the shipping code.
 *
 * Two cases are the ones this module exists for:
 *
 *  1. `re-mints once on a 401 and retries with the new token` together with
 *     `believes a second 401` and `never retries a 403` — the rule
 *     `withOneRemint` shares with `readDash`. A procedure path that retried
 *     a `403`, or looped on `401`, is issue #88 item 2 again.
 *  2. `exports the procedures and nothing of the models` — the generated
 *     package carries a class per model and vpay mounts none of their
 *     routes; this module is the only runtime importer and must not hand
 *     one out.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import type { DashboardConfig } from "../config/settings";
import * as cratestackClient from "./cratestack-client";
import { callProcedure, listProcedurePage } from "./cratestack-client";
import { REFUNDS } from "./resource-name";

const CONFIG: DashboardConfig = {
  apiBaseUrl: "http://vpay-server:8080",
  clientId: "vpay-dashboard",
  redirectUri: "http://localhost:3000/dash/v1/callback",
  scope: "dashboard:read",
  publicOrigin: null,
};

const EXPIRED = "the-token-on-the-row";
const FRESH = "the-token-the-exchange-minted";
const READER = { config: CONFIG, accessToken: EXPIRED, sessionToken: "s" };

/** One request the stub saw. */
interface Seen {
  readonly url: string;
  readonly method: string;
  readonly authorization: string | null;
  readonly body: unknown;
  readonly cache: RequestCache | undefined;
  readonly redirect: RequestRedirect | undefined;
}

/** A `fetch` standing in for vpay, including the re-mint's two legs. */
function stub(seen: Seen[], answers: (bearer: string | null) => Response) {
  return vi.fn((input: string | URL | Request, init?: RequestInit) => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.href
          : input.url;
    const authorization = new Headers(init?.headers).get("authorization");
    seen.push({
      url,
      method: init?.method ?? "GET",
      authorization,
      body:
        typeof init?.body === "string" && url.includes("/$procs/")
          ? (JSON.parse(init.body) as unknown)
          : null,
      cache: init?.cache,
      redirect: init?.redirect,
    });

    if (url.includes("/oauth/authorize")) {
      const state = new URL(url).searchParams.get("state") ?? "";
      return Promise.resolve(
        new Response(null, {
          status: 302,
          headers: {
            location: `${CONFIG.redirectUri}?code=a-code&state=${state}`,
          },
        }),
      );
    }
    if (url.includes("/oauth/token")) {
      return Promise.resolve(
        Response.json({
          access_token: FRESH,
          token_type: "Bearer",
          expires_in: 900,
          scope: CONFIG.scope,
        }),
      );
    }
    return Promise.resolve(answers(authorization));
  });
}

function refusal(status: number, message: string): Response {
  return Response.json(
    { error: { message } },
    { status, headers: { "request-id": "req_procs" } },
  );
}

function page(): Response {
  return Response.json({
    items: [{ id: "re_1" }],
    totalCount: 1,
    pageInfo: { hasNextPage: false, hasPreviousPage: false },
  });
}

/** Only the procedure calls, not the re-mint's legs. */
function procedureCalls(seen: Seen[]): Seen[] {
  return seen.filter((call) => call.url.includes("/$procs/"));
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the call itself", () => {
  it("posts the procedure's arguments with the bearer, uncached", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal("fetch", stub(seen, page));

    const result = await listProcedurePage(
      READER,
      REFUNDS,
      { limit: 20, offset: 40 },
      { status: "failed" },
    );

    expect(result.ok).toBe(true);
    expect(seen).toEqual([
      {
        url: "http://vpay-server:8080/dash/v1/$procs/searchRefunds",
        method: "POST",
        authorization: `Bearer ${EXPIRED}`,
        body: { page: { limit: 20, offset: 40 }, filter: { status: "failed" } },
        cache: "no-store",
        redirect: "error",
      },
    ]);
  });
});

describe("one 401, one re-mint", () => {
  it("re-mints once on a 401 and retries with the new token", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, (bearer) =>
        bearer === `Bearer ${FRESH}` ? page() : refusal(401, "expired"),
      ),
    );

    const result = await listProcedurePage(
      READER,
      REFUNDS,
      { limit: 20, offset: 0 },
      {},
    );

    expect(result.ok).toBe(true);
    expect(procedureCalls(seen).map((call) => call.authorization)).toEqual([
      `Bearer ${EXPIRED}`,
      `Bearer ${FRESH}`,
    ]);
  });

  it("believes a second 401, and answers the first refusal", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, () => refusal(401, "The bearer token is invalid.")),
    );

    const result = await listProcedurePage(
      READER,
      REFUNDS,
      { limit: 20, offset: 0 },
      {},
    );

    expect(result).toEqual({
      ok: false,
      failure: {
        status: 401,
        message: "The bearer token is invalid.",
        requestId: "req_procs",
      },
    });
    expect(procedureCalls(seen)).toHaveLength(2);
  });

  it("never retries a 403", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, () => refusal(403, "wrong scope")),
    );

    const result = await listProcedurePage(
      READER,
      REFUNDS,
      { limit: 20, offset: 0 },
      {},
    );

    expect(result.ok).toBe(false);
    expect(seen).toHaveLength(1);
  });
});

describe("what a failure says", () => {
  it("renders vpay's sentence and request id, never a raw body", async () => {
    vi.stubGlobal(
      "fetch",
      stub([], () => new Response("<html>bad gateway</html>", { status: 502 })),
    );

    const result = await callProcedure(READER, (procedures) =>
      procedures.searchCustomers({
        page: { limit: 1, offset: 0 },
        filter: {},
      }),
    );

    expect(result).toEqual({
      ok: false,
      failure: { status: 502, message: "vpay answered 502.", requestId: null },
    });
  });

  it("answers an unreachable vpay as status 0, not a throw", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.reject(new TypeError("fetch failed"))),
    );

    const result = await callProcedure(READER, (procedures) =>
      procedures.searchCustomers({
        page: { limit: 1, offset: 0 },
        filter: {},
      }),
    );

    expect(result.ok).toBe(false);
    expect(!result.ok && result.failure.status).toBe(0);
  });
});

describe("what this module hands out", () => {
  it("exports the procedures and nothing of the models", () => {
    expect(Object.keys(cratestackClient).sort()).toEqual([
      "callProcedure",
      "dashProcedures",
      "listProcedurePage",
      "procedureFailure",
    ]);
    const procedures = cratestackClient.dashProcedures(CONFIG, "t");
    const methods = Object.getOwnPropertyNames(
      Object.getPrototypeOf(procedures) as object,
    ).filter((name) => name !== "constructor");
    expect(methods.every((name) => name.startsWith("search"))).toBe(true);
  });
});
