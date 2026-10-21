import { describe, expect, it } from "vitest";
import {
  CHECKOUT_OUTCOME,
  PAYMENT_STATUS,
  checkoutOutcomeTone,
  checkoutOutcomeVariant,
  statusLabel,
  statusShortLabel,
} from "./index.js";

describe("status tokens", () => {
  it("covers every status with a label", () => {
    for (const s of PAYMENT_STATUS) {
      expect(statusLabel[s], `label for ${s}`).toBeTruthy();
    }
  });

  it("covers every status with a short label of two words at most", () => {
    for (const s of PAYMENT_STATUS) {
      const short = statusShortLabel[s];
      expect(short, `short label for ${s}`).toBeTruthy();
      expect(short.split(" ").length, short).toBeLessThanOrEqual(2);
    }
  });

  it("never labels processing as nearly-done", () => {
    expect(statusShortLabel.processing.toLowerCase()).not.toContain("almost");
    expect(statusLabel.processing.toLowerCase()).not.toContain("almost");
    expect(statusLabel.processing.toLowerCase()).not.toContain("complete");
  });
});

describe("checkout outcome tones", () => {
  it("covers every outcome", () => {
    for (const outcome of CHECKOUT_OUTCOME) {
      expect(checkoutOutcomeTone[outcome], `tone for ${outcome}`).toBeTruthy();
    }
  });

  it("never renders a FAILED payment as neutral — the defect this table exists for", () => {
    // A payer standing in a shop reading a grey box does not know the payment
    // did not happen. This is the assertion; the rest of the table is taste.
    expect(checkoutOutcomeTone.failed).toBe("error");
  });

  it("keeps success for the one outcome that succeeded", () => {
    const successes = CHECKOUT_OUTCOME.filter(
      (o) => checkoutOutcomeTone[o] === "success",
    );
    expect(successes).toEqual(["succeeded"]);
  });

  it("tones a canceled payment warning, not error — decision D4, 2026-09-07", () => {
    // Canceled is the payer's own action; failed is not. Softening one and
    // not the other is the point, so this asserts both sides of it.
    expect(checkoutOutcomeTone.canceled).toBe("warning");
    expect(checkoutOutcomeTone.failed).toBe("error");
  });
});

describe("checkout outcome variants (@vaam-apps/ui vocabulary)", () => {
  it("covers every outcome", () => {
    for (const outcome of CHECKOUT_OUTCOME) {
      expect(
        checkoutOutcomeVariant[outcome],
        `variant for ${outcome}`,
      ).toBeTruthy();
    }
  });

  it("agrees with checkoutOutcomeTone on which outcome is a success", () => {
    const toneSuccesses = CHECKOUT_OUTCOME.filter(
      (o) => checkoutOutcomeTone[o] === "success",
    );
    const variantSuccesses = CHECKOUT_OUTCOME.filter(
      (o) => checkoutOutcomeVariant[o] === "success",
    );
    expect(variantSuccesses).toEqual(toneSuccesses);
  });

  it("keeps failed the alarming tone in both vocabularies", () => {
    expect(checkoutOutcomeTone.failed).toBe("error");
    expect(checkoutOutcomeVariant.failed).toBe("danger");
  });

  it("keeps canceled a warning in both, not the alarming tone", () => {
    expect(checkoutOutcomeTone.canceled).toBe("warning");
    expect(checkoutOutcomeVariant.canceled).toBe("warning");
  });
});

describe("the dashboard's other state vocabularies", () => {
  it("labels every value of every list, and invents none", async () => {
    const t = await import("./index.js");
    const pairs: [readonly string[], Record<string, string>][] = [
      [t.REFUND_STATUS, t.refundStatusLabel],
      [t.DELIVERY_STATE, t.deliveryStateLabel],
      [t.CHECKOUT_STATUS, t.checkoutStatusLabel],
    ];
    for (const [list, labels] of pairs) {
      expect(Object.keys(labels).sort()).toEqual([...list].sort());
      for (const value of list) {
        expect(labels[value], value).toBeTruthy();
      }
    }
  });
});
