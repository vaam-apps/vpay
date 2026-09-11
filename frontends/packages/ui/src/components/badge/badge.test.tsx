import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Badge } from "./badge";

describe("Badge", () => {
  it("renders its tone and size classes", () => {
    render(
      <Badge tone="success" size="sm">
        Paid
      </Badge>,
    );
    const el = screen.getByText("Paid");
    expect(el.className).toContain("badge-success");
    expect(el.className).toContain("badge-sm");
  });
});
