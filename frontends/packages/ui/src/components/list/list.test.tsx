import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { List } from "./list";

describe("List", () => {
  it("renders its fixed layout classes on a real <ul>", () => {
    render(
      <List data-testid="list">
        <li>Item</li>
      </List>,
    );
    const el = screen.getByTestId("list");
    expect(el.tagName).toBe("UL");
    expect(el.className).toContain("space-y-1");
    expect(screen.getByRole("list")).toBe(el);
    // The quiet is an ink, not an opacity — see `list.tsx`. An `opacity-*`
    // here multiplies with `Text tone="muted"`'s and put a list line at
    // 2.71:1 with both call sites looking reasonable.
    expect(el.className).toContain("text-muted-ink");
    expect(el.className).not.toContain("opacity-");
  });
});
