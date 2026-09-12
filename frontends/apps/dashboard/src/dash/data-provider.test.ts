/**
 * The three query rules the plan (§4) requires this provider to carry over
 * from code that is already reviewed, plus the two refusals it owes.
 *
 * Each rule exists because breaking it produces a **400 naming a parameter
 * the operator never typed**, or a silently wrong page. None of them is
 * Refine's default behaviour, so none of them survives without a case.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import { dashDataProvider, DashHttpError } from "./data-provider";

const BASE = "/api/dash";

function stubFetch(body: unknown, status = 200) {
  const calls: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string) => {
      calls.push(url);
      return Promise.resolve({
        ok: status >= 200 && status < 300,
        status,
        json: () => Promise.resolve(body),
      } as Response);
    }),
  );
  return calls;
}

const EMPTY_PAGE = {
  object: "list",
  data: [],
  has_more: false,
  cursor: { newer: null, older: null },
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the query contract, carried over rather than re-invented", () => {
  it("takes the FIRST value of a repeated parameter, never the last and never joined", async () => {
    const calls = stubFetch(EMPTY_PAGE);
    await dashDataProvider(BASE).getList({
      resource: "payment_intents",
      filters: [
        { field: "status", operator: "eq", value: ["succeeded", "canceled"] },
      ],
    });
    const url = calls[0] ?? "";
    expect(
      url,
      "joined values are a 400 naming a parameter nobody typed",
    ).toContain("status=succeeded");
    expect(url).not.toContain("canceled");
  });

  it("drops an unparseable date rather than forwarding it", async () => {
    const calls = stubFetch(EMPTY_PAGE);
    await dashDataProvider(BASE).getList({
      resource: "payment_intents",
      filters: [
        { field: "created_from", operator: "gte", value: "not-a-date" },
        { field: "created_to", operator: "lte", value: "2026-09-12" },
      ],
    });
    const url = calls[0] ?? "";
    expect(url).not.toContain("not-a-date");
    expect(url).toContain("created_to=2026-09-12");
  });

  it("sends `after` and not `before` when a caller supplies both", async () => {
    // The server would answer 400. The client should not put it in that
    // position in the first place.
    const calls = stubFetch(EMPTY_PAGE);
    await dashDataProvider(BASE).getList({
      resource: "payment_intents",
      meta: { after: "pi_newer", before: "pi_older" },
    });
    const url = calls[0] ?? "";
    expect(url).toContain("after=pi_newer");
    expect(url).not.toContain("before=");
  });

  it("reports total as 0 — /dash/v1 counts nothing and a guess would be a lie", async () => {
    stubFetch({ ...EMPTY_PAGE, data: [{ id: "pi_1" }] });
    const page = await dashDataProvider(BASE).getList({
      resource: "payment_intents",
    });
    expect(
      page.total,
      "data.length would describe the page, not the account",
    ).toBe(0);
  });

  it("passes the cursor through as the BFF resolved it, inversion included", async () => {
    stubFetch({
      ...EMPTY_PAGE,
      cursor: { newer: "pi_newer", older: "pi_older" },
    });
    const page = (await dashDataProvider(BASE).getList({
      resource: "payment_intents",
    })) as unknown as { cursor: { next: string | null; prev: string | null } };
    // `older` is forward (towards older rows) and `newer` is back. The
    // backward-page `has_more` inversion is resolved in the seam, and this
    // provider must not re-derive it.
    expect(page.cursor.next).toBe("pi_older");
    expect(page.cursor.prev).toBe("pi_newer");
  });
});

describe("the refusals it owes", () => {
  it("throws on a resource /dash/v1 does not serve", async () => {
    stubFetch(EMPTY_PAGE);
    await expect(
      dashDataProvider(BASE).getList({ resource: "refunds" }),
    ).rejects.toBeInstanceOf(DashHttpError);
  });

  for (const method of ["create", "update", "deleteOne"] as const) {
    it(`throws on ${method} rather than pretending /dash/v1 accepts a write`, () => {
      // `/dash/v1` answers 403 to every non-GET at its boundary. A provider
      // that "worked" would be a lie that surfaces only at submit time.
      const provider = dashDataProvider(BASE);
      expect(() => (provider[method] as () => unknown)()).toThrow(
        DashHttpError,
      );
    });
  }

  it("carries the status so onError can tell 401 from an outage", async () => {
    stubFetch({}, 503);
    await expect(
      dashDataProvider(BASE).getList({ resource: "payment_intents" }),
    ).rejects.toMatchObject({ statusCode: 503 });
  });

  it("never sends a credential of its own — the cookie is the browser's", async () => {
    const fetchSpy = vi.fn((_url: string, _init?: RequestInit) =>
      Promise.resolve({
        ok: true,
        status: 200,
        json: () => Promise.resolve(EMPTY_PAGE),
      } as Response),
    );
    vi.stubGlobal("fetch", fetchSpy);
    await dashDataProvider(BASE).getList({ resource: "payment_intents" });
    const init = fetchSpy.mock.calls[0]?.[1];
    expect(init?.credentials).toBe("same-origin");
    const headers = (init?.headers ?? {}) as Record<string, string>;
    expect(
      Object.keys(headers).map((k) => k.toLowerCase()),
      "a bearer token here would mean the browser holds one",
    ).not.toContain("authorization");
  });
});
