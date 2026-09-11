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
  });
});
