import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import Home from "../../app/page";
import NotFound from "../../app/not-found";
import { ConsoleUnavailable } from "./console-unavailable";

describe("the unavailable notice", () => {
  it("says the console signs in with a paired key, and that it is not available yet", () => {
    render(<ConsoleUnavailable />);
    expect(
      screen.getByRole("heading", {
        level: 2,
        name: "This console signs in with a paired key",
      }),
    ).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(
      "not available until VP-6b",
    );
  });

  it("offers no way to sign in — no form, no field, no password", () => {
    const { container } = render(<ConsoleUnavailable />);
    expect(container.querySelector("form, input, button")).toBeNull();
    expect(container.textContent?.toLowerCase()).not.toContain(
      "sign in with a password",
    );
  });

  it("owns the document's one <h1>, hidden from sight", () => {
    render(<ConsoleUnavailable />);
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
  });

  it("is what `/` and every unknown path render", () => {
    const home = render(<Home />);
    const homeHtml = home.container.innerHTML;
    home.unmount();
    const missing = render(<NotFound />);
    expect(missing.container.innerHTML).toBe(homeHtml);
    expect(homeHtml).toContain("signs in with a paired key");
  });
});
