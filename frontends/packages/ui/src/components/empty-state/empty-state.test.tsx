import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { EmptyState } from "./empty-state";

describe("EmptyState", () => {
  it("says both what is empty and why", () => {
    const { unmount } = render(
      <EmptyState
        title="No payments"
        description="No payment matched these filters."
      />,
    );
    expect(
      screen.getByRole("heading", { level: 3, name: "No payments" }),
    ).toBeTruthy();
    expect(screen.getByText("No payment matched these filters.")).toBeTruthy();
    // An empty result is not a failure, and must not be announced as one.
    expect(screen.queryByRole("alert")).toBeNull();
    unmount();
  });

  it("renders a way out when one is given", () => {
    const { unmount } = render(
      <EmptyState
        title="No payments"
        description="Widen the range."
        action={<button type="button">Clear filters</button>}
      />,
    );
    expect(screen.getByRole("button", { name: "Clear filters" })).toBeTruthy();
    unmount();
  });
});
