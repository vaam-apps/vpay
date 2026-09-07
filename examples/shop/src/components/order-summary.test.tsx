// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { OrderStatusBadge } from "@/components/order-summary";
import type { OrderStatus } from "@/lib/order-view";

afterEach(() => {
  cleanup();
});

const STATUSES: readonly OrderStatus[] = [
  "unpaid",
  "paid",
  "failed",
  "cancelled",
];

/**
 * Decisive mutation, plan §5 Lane C: "OrderStatusBadge renders one tone for
 * every status | fail". `unpaid`/`paid`/`failed` must each carry their own
 * `badge-{tone}` class — the badge's tone is the ONLY visual signal
 * distinguishing a settled order from an open one, since the daisyUI badge
 * itself has no icon.
 *
 * `failed` and `cancelled` are asserted EQUAL, on purpose: the CSS this
 * replaced coloured both `--bad` (see `order-summary.tsx`'s own comment),
 * because both mean "did not settle" to a buyer. Collapsing every status to
 * one tone — the mutation this test exists to catch — would make `paid` and
 * `failed` indistinguishable too, which is the property that actually
 * matters and is what the second assertion below checks.
 */
describe("OrderStatusBadge", () => {
  it("gives unpaid, paid and failed each their own badge tone", () => {
    const toneOf = (status: OrderStatus): string | undefined => {
      const { unmount } = render(<OrderStatusBadge status={status} />);
      const badge = screen.getByTestId("order-status");
      const tone = [...badge.classList].find(
        (token) => token.startsWith("badge-") && token !== "badge-",
      );
      unmount();
      return tone;
    };

    const tones = new Map(STATUSES.map((status) => [status, toneOf(status)]));

    expect(tones.get("unpaid")).toBe("badge-warning");
    expect(tones.get("paid")).toBe("badge-success");
    expect(tones.get("failed")).toBe("badge-error");
    expect(tones.get("cancelled")).toBe("badge-error");

    // The property a collapsed-tone mutation would actually break: `paid`
    // must never share a tone with either status that did not settle.
    expect(tones.get("paid")).not.toBe(tones.get("unpaid"));
    expect(tones.get("paid")).not.toBe(tones.get("failed"));
  });

  it("renders the exact label text `order-status` — the e2e specs assert on — for each status", () => {
    const LABEL: Readonly<Record<OrderStatus, string>> = {
      unpaid: "Unpaid",
      paid: "Paid",
      failed: "Failed",
      cancelled: "Cancelled",
    };
    for (const status of STATUSES) {
      const { unmount } = render(<OrderStatusBadge status={status} />);
      expect(screen.getByTestId("order-status")).toHaveTextContent(
        LABEL[status],
      );
      unmount();
    }
  });
});
