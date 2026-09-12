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
    // `text-muted-ink`, not `opacity-60`, since 2026-09-12. An opacity
    // multiplies with an ancestor's — `List` set 0.7 and this set 0.6, so a
    // muted line inside a list rendered at 0.42, #9d9d9d, 2.71:1. Asserting
    // the ink rather than the opacity is what stops that returning.
    expect(el.className).toContain("text-muted-ink");
    expect(el.className).not.toContain("opacity-");
    expect(el.className).toContain("text-xs");
  });

  it("paints an error in the ink measured for a light surface, not daisyUI's fill", () => {
    // `text-error` is `--color-error`, which daisyUI intends as a BACKGROUND
    // fill; on `--color-base-100` it is 2.92:1. `--color-error-ink` is the
    // pair `styles.css` measures for this direction.
    render(<Text tone="error">Enter a valid phone number.</Text>);
    const el = screen.getByText("Enter a valid phone number.");
    expect(el.className).toContain("text-error-ink");
    // Class tokens, not a substring: `\btext-error\b` also matches inside
    // `text-error-ink`, because `-` is a word boundary.
    expect(el.className.split(/\s+/)).not.toContain("text-error");
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
