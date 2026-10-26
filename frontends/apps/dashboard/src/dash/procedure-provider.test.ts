/**
 * The browser's `procedures` provider: `@cratestack/refine` over this app's
 * own `GET /api/dash/<list>` routes, never over vpay and never with a body.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import { failureFromError } from "./failure";
import { procedureDataProvider } from "./procedure-provider";
import { CHECKOUT_SESSIONS, WEBHOOK_DELIVERIES } from "./resource-name";

interface Seen {
  readonly url: string;
  readonly method: string;
  readonly credentials: RequestCredentials | undefined;
}

function stub(seen: Seen[], answer: () => Response) {
  return vi.fn((input: string | URL | Request, init?: RequestInit) => {
    seen.push({
      url:
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url,
      method: init?.method ?? "GET",
      credentials: init?.credentials,
    });
    return Promise.resolve(answer());
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the browser transport", () => {
  it("GETs the BFF route that serves the resource, on the same origin", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, () =>
        Response.json({
          object: "list",
          data: [{ id: "whd_1" }, { id: "whd_2" }],
          total_count: 42,
          limit: 20,
          offset: 20,
        }),
      ),
    );

    const result = await procedureDataProvider("/api/dash/").getList({
      resource: WEBHOOK_DELIVERIES,
      pagination: { currentPage: 2, pageSize: 20, mode: "server" },
    });

    expect(seen).toEqual([
      {
        url: "/api/dash/deliveries?limit=20&offset=20",
        method: "GET",
        credentials: "same-origin",
      },
    ]);
    expect(result).toEqual({
      data: [{ id: "whd_1" }, { id: "whd_2" }],
      total: 42,
    });
  });

  it("refuses a filter, because the BFF forwards none", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, () => Response.json({})),
    );

    const read = procedureDataProvider("/api/dash").getList({
      resource: CHECKOUT_SESSIONS,
      filters: [{ field: "status", operator: "eq", value: "open" }],
    });

    await expect(read).rejects.toMatchObject({ statusCode: 400 });
    expect(seen).toEqual([]);
  });

  it("carries the BFF's status into refine, for onError to read", async () => {
    vi.stubGlobal(
      "fetch",
      stub([], () => new Response(null, { status: 401 })),
    );

    const error: unknown = await procedureDataProvider("/api/dash")
      .getList({ resource: CHECKOUT_SESSIONS })
      .catch((caught: unknown) => caught);

    expect(error).toMatchObject({ statusCode: 401 });
    expect(failureFromError(error).status).toBe(401);
  });
});
