import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Logo } from "./logo";

describe("Logo", () => {
  it("pins the operator mark to a height Tailwind preflight would otherwise drop", () => {
    render(
      <Logo
        src="https://operator.example/mark.svg"
        alt="Operator"
        data-testid="logo"
      />,
    );
    const el = screen.getByTestId("logo");
    expect(el.tagName).toBe("IMG");
    expect(el.getAttribute("alt")).toBe("Operator");
    expect(el.className).toContain("h-8");
    expect(el.className).toContain("w-auto");
  });
});
