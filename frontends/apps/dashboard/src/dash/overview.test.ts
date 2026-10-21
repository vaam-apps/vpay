/**
 * The Overview's reads, stubbed at the network boundary and nowhere else.
 *
 * What is held here is the screen's one promise: every number on it is a
 * `totalCount` vpay returned for a filter vpay applied, and a refused read
 * is a refusal — never a `0`. A version that fell back to `0` would draw a
 * clean-looking Overview over an outage, which is the failure `CLAUDE.md`
 * names first.
 */
import { PAYMENT_STATUS } from "@vpay/tokens";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { DashboardConfig } from "../config/settings";
import type { StaffContext } from "../server/session";
import { firstFailure, LATEST, readOverview } from "./overview";

const CONFIG: DashboardConfig = {
  apiBaseUrl: "http://vpay-server:8080",
  clientId: "vpay-dashboard",
  redirectUri: "http://localhost:3000/dash/v1/callback",
  scope: "dashboard:read",
  publicOrigin: null,
};

const STAFF = {
  session: {} as StaffContext["session"],
  accessToken: "token",
  sessionToken: "session",
  config: CONFIG,
} satisfies StaffContext;

/** One procedure call the stub answered. */
interface Call {
  procedure: string;
  filter: Record<string, string>;
  limit: number;
}

/**
 * A `fetch` that answers every `$procs` call with `count(procedure, filter)`
 * as `totalCount`, or a `503` when `count` returns `null`.
 */
function stub(
  calls: Call[],
  count: (procedure: string, filter: Record<string, string>) => number | null,
) {
  return vi.fn((input: string | URL | Request, init?: RequestInit) => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.href
          : input.url;
    const procedure = url.split("/$procs/")[1] ?? "";
    const body = JSON.parse(
      typeof init?.body === "string" ? init.body : "{}",
    ) as {
      page: { limit: number };
      filter: Record<string, string>;
    };
    calls.push({ procedure, filter: body.filter, limit: body.page.limit });
    const total = count(procedure, body.filter);
    if (total === null) {
      return Promise.resolve(
        new Response(
          JSON.stringify({ error: { message: "vpay is restarting" } }),
          { status: 503, headers: { "x-request-id": "req_overview" } },
        ),
      );
    }
    return Promise.resolve(
      new Response(
        JSON.stringify({
          items: [],
          totalCount: total,
          pageInfo: { hasNextPage: false, hasPreviousPage: false },
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    );
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the overview's counts", () => {
  it("asks each procedure for the filter its tile links to, one row at a time", async () => {
    const calls: Call[] = [];
    vi.stubGlobal(
      "fetch",
      stub(calls, () => 3),
    );
    await readOverview(STAFF);

    const asked = calls.map((call) => ({
      procedure: call.procedure,
      filter: call.filter,
    }));
    expect(asked).toEqual(
      expect.arrayContaining([
        { procedure: "searchPaymentIntents", filter: {} },
        ...PAYMENT_STATUS.map((status) => ({
          procedure: "searchPaymentIntents",
          filter: { status },
        })),
        { procedure: "searchRefunds", filter: { status: "failed" } },
        { procedure: "searchRefunds", filter: { status: "pending" } },
        { procedure: "searchWebhookDeliveries", filter: { state: "failed" } },
        {
          procedure: "searchWebhookDeliveries",
          filter: { state: "exhausted" },
        },
        { procedure: "searchCheckoutSessions", filter: { status: "open" } },
        { procedure: "searchCustomers", filter: {} },
      ]),
    );
    expect(calls).toHaveLength(1 + PAYMENT_STATUS.length + 6);
    // Only the unfiltered payments read lists rows; every count asks for one.
    for (const call of calls) {
      const listsRows =
        call.procedure === "searchPaymentIntents" &&
        Object.keys(call.filter).length === 0;
      expect(call.limit).toBe(listsRows ? LATEST : 1);
    }
  });

  it("reports each count exactly as vpay's totalCount, never summed", async () => {
    vi.stubGlobal(
      "fetch",
      stub([], (procedure, filter) =>
        procedure === "searchPaymentIntents"
          ? filter["status"] === undefined
            ? 41
            : filter["status"].length
          : 7,
      ),
    );
    const overview = await readOverview(STAFF);
    // 41 is what vpay said for "every status", and it is NOT the sum of the
    // per-status figures below — the screen must show 41.
    expect(overview.payments).toMatchObject({ ok: true, value: { total: 41 } });
    for (const status of PAYMENT_STATUS) {
      expect(overview.byStatus[status]).toEqual({
        ok: true,
        value: status.length,
      });
    }
    expect(overview.failedDeliveries).toEqual({ ok: true, value: 7 });
    expect(firstFailure(overview)).toBeNull();
  });

  it("keeps a refused count refused — never a zero", async () => {
    vi.stubGlobal(
      "fetch",
      stub([], (procedure) => (procedure === "searchRefunds" ? null : 0)),
    );
    const overview = await readOverview(STAFF);
    expect(overview.failedRefunds.ok).toBe(false);
    expect(overview.pendingRefunds.ok).toBe(false);
    // The others are genuinely zero, and say so.
    expect(overview.customers).toEqual({ ok: true, value: 0 });
    const failure = firstFailure(overview);
    expect(failure?.status).toBe(503);
    expect(failure?.requestId).toBe("req_overview");
  });
});
