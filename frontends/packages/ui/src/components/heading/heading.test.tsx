import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Heading } from "./heading";

describe("Heading", () => {
  it("renders the requested level with the shared typography classes", () => {
    render(<Heading level={2}>Title</Heading>);
    const el = screen.getByRole("heading", { level: 2, name: "Title" });
    expect(el.className).toContain("text-xl");
  });

  it("defaults to level 1", () => {
    render(<Heading>Confirm your payment</Heading>);
    expect(
      screen.getByRole("heading", { level: 1, name: "Confirm your payment" }),
    ).toBeTruthy();
  });
});
