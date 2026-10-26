/**
 * The procedure-backed manifest, driven through the real
 * `createCratestackDataProvider` — the only stub is the transport.
 *
 * The decisive case is the first `describe`: a filter key the procedure does
 * not declare must be refused, **before** anything is sent. Delete the
 * allowlist check in `filterFor` and `refuses a filter the procedure does
 * not declare` goes red, as does the `getMany` case — whose `id__in` would
 * otherwise reach the procedure as an ignored field and render every row as
 * if it were the ones asked for.
 */
import { createCratestackDataProvider } from "@cratestack/refine";
import type { CratestackPage } from "@cratestack/refine";
import { describe, expect, it, vi } from "vitest";

import { failureFromError } from "./failure";
import {
  procedureResources,
  ProcedureRefusal,
  type ProcedureLister,
  type ProcedurePageInput,
} from "./procedure-resources";
import { CUSTOMERS, REFUNDS, WEBHOOK_DELIVERIES } from "./resource-name";

interface Call {
  readonly resource: string;
  readonly page: ProcedurePageInput;
  readonly filter: Readonly<Record<string, string>>;
}

function page(
  items: unknown[],
  totalCount: number | null,
): CratestackPage<unknown> & { pageInfo: unknown } {
  return {
    items,
    totalCount,
    pageInfo: { hasNextPage: false, hasPreviousPage: false },
  };
}

function providerOver(
  calls: Call[],
  answer: () => Promise<CratestackPage<unknown>> = () =>
    Promise.resolve(page([{ id: "re_1" }], 41)),
) {
  const lister = vi.fn<ProcedureLister>((resource, input, filter) => {
    calls.push({ resource, page: input, filter });
    return answer();
  });
  return createCratestackDataProvider(procedureResources(lister));
}

describe("what reaches the procedure", () => {
  it("sends refine's page as the procedure's limit and offset", async () => {
    const calls: Call[] = [];
    await providerOver(calls).getList({
      resource: REFUNDS,
      pagination: { currentPage: 3, pageSize: 20, mode: "server" },
    });
    expect(calls).toEqual([
      { resource: REFUNDS, page: { limit: 20, offset: 40 }, filter: {} },
    ]);
  });

  it("names the procedure's own filter fields", async () => {
    const calls: Call[] = [];
    await providerOver(calls).getList({
      resource: REFUNDS,
      filters: [
        { field: "status", operator: "eq", value: "failed" },
        { field: "created", operator: "gte", value: "2026-10-01T00:00:00Z" },
        { field: "created", operator: "lte", value: "2026-10-26T00:00:00Z" },
      ],
    });
    expect(calls[0]?.filter).toEqual({
      status: "failed",
      created_gte: "2026-10-01T00:00:00Z",
      created_lte: "2026-10-26T00:00:00Z",
    });
  });

  it("knows that deliveries filter on state, not status", async () => {
    const calls: Call[] = [];
    await providerOver(calls).getList({
      resource: WEBHOOK_DELIVERIES,
      filters: [{ field: "state", operator: "eq", value: "exhausted" }],
    });
    expect(calls[0]?.filter).toEqual({ state: "exhausted" });
  });
});

describe("what it refuses before anything is sent", () => {
  it("refuses a filter the procedure does not declare", async () => {
    const calls: Call[] = [];
    const read = providerOver(calls).getList({
      resource: WEBHOOK_DELIVERIES,
      filters: [{ field: "status", operator: "eq", value: "failed" }],
    });
    await expect(read).rejects.toMatchObject({
      statusCode: 400,
      message: 'webhook_deliveries has no filter "status".',
    });
    expect(calls).toEqual([]);
  });

  it("refuses getMany, whose id__in no procedure declares", async () => {
    const calls: Call[] = [];
    const read = providerOver(calls).getMany!({
      resource: CUSTOMERS,
      ids: ["cus_1", "cus_2"],
    });
    await expect(read).rejects.toMatchObject({ statusCode: 400 });
    expect(calls).toEqual([]);
  });

  it("refuses a sort, because every procedure orders newest first", async () => {
    const calls: Call[] = [];
    const read = providerOver(calls).getList({
      resource: REFUNDS,
      sorters: [{ field: "amount", order: "desc" }],
    });
    await expect(read).rejects.toMatchObject({ statusCode: 400 });
    expect(calls).toEqual([]);
  });

  it("has no detail read and no writes", async () => {
    const calls: Call[] = [];
    const provider = providerOver(calls);
    await expect(
      provider.getOne({ resource: REFUNDS, id: "re_1" }),
    ).rejects.toMatchObject({ statusCode: 404 });
    await expect(
      provider.update({ resource: REFUNDS, id: "re_1", variables: {} }),
    ).rejects.toMatchObject({ statusCode: 403 });
    await expect(
      provider.deleteOne({ resource: REFUNDS, id: "re_1" }),
    ).rejects.toMatchObject({ statusCode: 403 });
    await expect(
      provider.create({ resource: REFUNDS, variables: {} }),
    ).rejects.toThrow();
    expect(calls).toEqual([]);
  });
});

describe("what comes back", () => {
  it("answers the procedure's rows and its exact count", async () => {
    const result = await providerOver([]).getList({ resource: REFUNDS });
    expect(result).toEqual({ data: [{ id: "re_1" }], total: 41 });
  });

  it("reports 0 for a count the procedure says it does not know", async () => {
    // Pinned because it is a gap, not a feature: past the end of the set a
    // procedure answers `totalCount: null`, and `@cratestack/refine` falls
    // back to `items.length`. The screens read the count from the page for
    // exactly this reason (`procedure-list.ts`); if this ever answers
    // `null`, that workaround can go.
    const result = await providerOver([], () =>
      Promise.resolve(page([], null)),
    ).getList({ resource: REFUNDS });
    expect(result.total).toBe(0);
  });

  it("keeps the refusal's status and request id through refine", async () => {
    const failure = {
      status: 503,
      message: "vpay is restarting",
      requestId: "req_refused",
    };
    const read = providerOver([], () =>
      Promise.reject(new ProcedureRefusal(failure)),
    ).getList({ resource: REFUNDS });
    const error: unknown = await read.catch((caught: unknown) => caught);
    expect(error).toMatchObject({ statusCode: 503 });
    expect(failureFromError(error)).toEqual(failure);
  });
});
