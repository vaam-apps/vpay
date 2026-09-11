import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { VisuallyHidden } from "./visually-hidden";

describe("VisuallyHidden", () => {
  it("is text for a screen reader and not for the eye", () => {
    render(<VisuallyHidden data-testid="vh">Amount:</VisuallyHidden>);
    const el = screen.getByTestId("vh");
    expect(el.tagName).toBe("SPAN");
    expect(el.className).toContain("sr-only");
    // Still in the accessibility tree — `sr-only` clips, it does not hide.
    expect(el.getAttribute("aria-hidden")).toBeNull();
  });
});
