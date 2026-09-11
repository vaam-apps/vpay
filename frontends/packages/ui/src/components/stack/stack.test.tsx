import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Stack } from "./stack";

describe("Stack", () => {
  it("applies its direction, align and gap variants", () => {
    render(
      <Stack direction="column" gap="lg" data-testid="stack">
        <span>a</span>
      </Stack>,
    );
    const el = screen.getByTestId("stack");
    expect(el.className).toContain("flex-col");
    expect(el.className).toContain("gap-6");
  });

  it("renders the element it is asked for, so a landmark survives", () => {
    // A stack that IS the page header is a `banner` landmark. Rendering it
    // as a `<div>` removes that landmark with no test and no lint failing,
    // which is how `checkout-view.tsx` and `return-view.tsx` lost theirs.
    render(
      <Stack as="header" justify="between" data-testid="header">
        <span>a</span>
      </Stack>,
    );
    const el = screen.getByTestId("header");
    expect(el.tagName).toBe("HEADER");
    expect(screen.getByRole("banner")).toBe(el);
    expect(el.className).toContain("justify-between");
  });
});
