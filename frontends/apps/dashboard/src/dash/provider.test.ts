/**
 * The seam, exercised against a stubbed `fetch` and nothing else.
 *
 * Stubbed at the network boundary like `dash-read.test.ts` and
 * `oauth.test.ts`, so every assertion here is about the request this app
 * actually put on the wire and the page it made of the answer — not about a
 * mock of its own collaborators agreeing with itself.
 *
 * **The decisive case is `a backward page reports NEWER rows, not older
 * ones`.** Delete the direction test inside `pageCursors` — make `hasNewer`
 * and `hasOlder` read `has_more` the same way on both — and it fails. It is
 * the one rule the framework this seam exists for has no equivalent of: a
 * data provider's cursor is two opaque page params, and nothing in one learns
 * that the flag it was handed changed meaning halfway down the list.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import type { DashboardConfig } from "../config/settings";
import { INTENT, OTHER_INTENT } from "../testing/fixtures";
import { NO_QUERY } from "../payments-query";
import { dashProvider, PAYMENT_INTENTS } from "./provider";

const CONFIG: DashboardConfig = {
  apiBaseUrl: "http://vpay-server:8080",
  clientId: "vpay-dashboard",
  redirectUri: "http://localhost:3000/dash/v1/callback",
  scope: "dashboard:read",
  publicOrigin: null,
};

const SESSION = {
  config: CONFIG,
  accessToken: "the-token-on-the-row",
  sessionToken: "the-staff-session-token",
};

/** A `fetch` that answers every `/dash/v1` read with `body`, recording urls. */
function stub(urls: string[], body: unknown, status = 200) {
  return vi.fn((input: string | URL | Request) => {
    urls.push(
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.href
          : input.url,
    );
    return Promise.resolve(
      new Response(JSON.stringify(body), {
        status,
        headers: { "content-type": "application/json" },
      }),
    );
  });
}

/** The two-row page every paging case below reads. */
function page(hasMore: boolean) {
  return {
    object: "list",
    data: [INTENT, OTHER_INTENT],
    has_more: hasMore,
    url: "/dash/v1/payment_intents",
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("getList", () => {
  it("asks vpay for the resource, with the filters the query names", async () => {
    const urls: string[] = [];
    vi.stubGlobal("fetch", stub(urls, page(false)));

    await dashProvider(SESSION).getList({
      resource: PAYMENT_INTENTS,
      query: {
        ...NO_QUERY,
        status: "succeeded",
        createdFrom: "2026-09-01",
        after: "pi_zero",
      },
    });

    expect(urls).toHaveLength(1);
    const url = new URL(urls[0] ?? "");
    expect(url.pathname).toBe("/dash/v1/payment_intents");
    expect(url.searchParams.get("status")).toBe("succeeded");
    expect(url.searchParams.get("created_gte")).toBe("2026-09-01T00:00:00Z");
    expect(url.searchParams.get("starting_after")).toBe("pi_zero");
    // The filter mapper's rule, reached through the seam: a cursor pair is
    // never both sent, because vpay refuses the two together.
    expect(url.searchParams.has("ending_before")).toBe(false);
  });

  it("carries has_more through unchanged rather than deriving it", async () => {
    vi.stubGlobal("fetch", stub([], page(true)));
    const result = await dashProvider(SESSION).getList({
      resource: PAYMENT_INTENTS,
      query: NO_QUERY,
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.hasMore).toBe(true);
      expect(result.value.data).toHaveLength(2);
    }
  });

  it("hands a refusal back as a refusal, never as an empty page", async () => {
    // The failure mode this whole app is built against: a read that could not
    // be made must not arrive at a screen looking like zero rows.
    vi.stubGlobal(
      "fetch",
      stub([], { error: { message: "vpay is unavailable." } }, 503),
    );
    const result = await dashProvider(SESSION).getList({
      resource: PAYMENT_INTENTS,
      query: NO_QUERY,
    });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.failure.status).toBe(503);
    }
  });

  describe("the cursor pair", () => {
    it("offers nowhere to go back to on the first page", async () => {
      vi.stubGlobal("fetch", stub([], page(true)));
      const result = await dashProvider(SESSION).getList({
        resource: PAYMENT_INTENTS,
        query: NO_QUERY,
      });
      expect(result.ok && result.value.cursor).toEqual({
        newer: null,
        older: OTHER_INTENT.id,
      });
    });

    it("a backward page reports NEWER rows, not older ones", async () => {
      // THE DECISIVE CASE FOR THIS LANE.
      //
      // `vpay_db` walks `seq ASC` from `ending_before` and reverses the rows
      // before answering, so `has_more` on a backward page means "more NEWER
      // rows exist" — it says nothing whatever about the older ones, which
      // certainly exist because this page was reached from them.
      //
      // Delete that inversion — read `has_more` the same way in both
      // directions — and both halves below go wrong at once: `older` becomes
      // `null` at a point where the page underneath demonstrably exists, and
      // `newer` becomes non-null at the newest end of the list, pointing at
      // an empty page. An empty page reads as data having been lost.
      const backwards = { ...NO_QUERY, before: "pi_zero" };
      const provider = dashProvider(SESSION);

      vi.stubGlobal("fetch", stub([], page(false)));
      const atTheTop = await provider.getList({
        resource: PAYMENT_INTENTS,
        query: backwards,
      });
      expect(atTheTop.ok && atTheTop.value.cursor).toEqual({
        newer: null,
        older: OTHER_INTENT.id,
      });

      vi.stubGlobal("fetch", stub([], page(true)));
      const midway = await provider.getList({
        resource: PAYMENT_INTENTS,
        query: backwards,
      });
      expect(midway.ok && midway.value.cursor).toEqual({
        newer: INTENT.id,
        older: OTHER_INTENT.id,
      });
    });

    it("offers nothing at all for an empty result set", async () => {
      vi.stubGlobal(
        "fetch",
        stub([], { object: "list", data: [], has_more: false }),
      );
      const result = await dashProvider(SESSION).getList({
        resource: PAYMENT_INTENTS,
        query: NO_QUERY,
      });
      expect(result.ok && result.value.cursor).toEqual({
        newer: null,
        older: null,
      });
    });
  });
});

describe("getOne", () => {
  it("reads the record by id, under the bearer it was given", async () => {
    const urls: string[] = [];
    vi.stubGlobal("fetch", stub(urls, { object: "dashboard.payment_detail" }));

    await dashProvider(SESSION).getOne({
      resource: PAYMENT_INTENTS,
      id: "pi_example_1",
    });

    expect(urls).toEqual([
      "http://vpay-server:8080/dash/v1/payment_intents/pi_example_1",
    ]);
  });

  it("encodes an id rather than pasting it into the path", async () => {
    // An id is one path segment. `../staff/session` pasted in unencoded is a
    // different upstream route, reached with a token that is allowed to read
    // it.
    const urls: string[] = [];
    vi.stubGlobal("fetch", stub(urls, {}));

    await dashProvider(SESSION).getOne({
      resource: PAYMENT_INTENTS,
      id: "../staff/session",
    });

    expect(urls[0]).toBe(
      "http://vpay-server:8080/dash/v1/payment_intents/..%2Fstaff%2Fsession",
    );
  });

  it("hands a 404 back as a 404 for the route to map", async () => {
    // The page turns this into `notFound()`, and the mapping has to stay
    // exact: a foreign merchant's id and an absent one answer the identical
    // 404 upstream, and anything here that told them apart would make this
    // app an oracle for which ids exist in other tenants.
    vi.stubGlobal(
      "fetch",
      stub([], { error: { message: "No such payment_intent." } }, 404),
    );
    const result = await dashProvider(SESSION).getOne({
      resource: PAYMENT_INTENTS,
      id: "pi_example_1",
    });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.failure.status).toBe(404);
    }
  });
});
