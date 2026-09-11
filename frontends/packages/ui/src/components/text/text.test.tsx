import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Text } from "./text";

describe("Text", () => {
  it("applies tone and size without ever writing a raw opacity/text-error inline", () => {
    render(
      <Text tone="muted" size="xs">
        Support line
      </Text>,
    );
    const el = screen.getByText("Support line");
    expect(el.className).toContain("opacity-60");
    expect(el.className).toContain("text-xs");
  });

  it("carries the amount variants the checkout page needs, without a call-site class", () => {
    // These exist so `screens.tsx` writes no class of its own (plan §3: raw
    // utilities live only inside this package). The payment amount is the
    // one number a payer checks before approving.
    render(
      <Text size="3xl" weight="semibold" numeric data-testid="amount">
        5,000
      </Text>,
    );
    const amount = screen.getByTestId("amount");
    expect(amount.className).toContain("text-3xl");
    expect(amount.className).toContain("tabular-nums");
    // `text-lg` must NOT survive alongside `text-3xl`: two font sizes on one
    // element is a cascade coin-flip, which is what `cn()` is for.
    expect(amount.className).not.toContain("text-lg");

    render(
      <Text wrap="anywhere" data-testid="reference">
        cs_test_fixture000000000001
      </Text>,
    );
    expect(screen.getByTestId("reference").className).toContain("break-all");
  });
});
