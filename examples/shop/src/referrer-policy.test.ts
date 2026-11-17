/**
 * The shop's `Referrer-Policy` (issue #65).
 *
 * vpay's checkout page in a popup pins the window it reports to by matching
 * `document.referrer` against the merchant's registered origins, and says
 * nothing when the referrer is empty. A shop that sends no referrer therefore
 * loses vpay's own `vpay:complete` without any error anywhere — the popup
 * still works, through the `success_url` fallback, which is exactly why a
 * regression here would go unnoticed. `next.config.ts` says why in full.
 */
import { describe, expect, it } from "vitest";
import config, { REFERRER_POLICY } from "../next.config";

/** Policies under which a cross-origin navigation still carries this shop's origin. */
const KEEPS_ORIGIN_CROSS_ORIGIN = new Set([
  "strict-origin-when-cross-origin",
  "strict-origin",
  "origin",
  "origin-when-cross-origin",
]);

describe("the shop's Referrer-Policy", () => {
  it("is one that sends vpay this shop's origin, and never a path", () => {
    expect(KEEPS_ORIGIN_CROSS_ORIGIN.has(REFERRER_POLICY)).toBe(true);
  });

  it("is served on every route, so any page that opens the popup carries it", async () => {
    expect(config.headers).toBeTypeOf("function");
    const rules = (await config.headers?.()) ?? [];
    const everyRoute = rules.filter((rule) => rule.source === "/:path*");
    expect(everyRoute).toHaveLength(1);
    const policies = rules.flatMap((rule) =>
      rule.headers
        .filter((header) => header.key.toLowerCase() === "referrer-policy")
        .map((header) => ({ source: rule.source, value: header.value })),
    );
    // Exactly one, on the catch-all: a second, narrower rule could override
    // it for the very route that opens the popup.
    expect(policies).toEqual([{ source: "/:path*", value: REFERRER_POLICY }]);
  });
});
