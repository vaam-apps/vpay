import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Pagination } from "./pagination";

describe("Pagination", () => {
  it("renders both controls as links inside a named nav", () => {
    const { unmount } = render(
      <Pagination
        label="Payments paging"
        previous={<a href="/payments?ending_before=pi_2">Previous</a>}
        next={<a href="/payments?starting_after=pi_9">Next</a>}
      />,
    );
    const nav = screen.getByRole("navigation", { name: "Payments paging" });
    expect(nav.tagName).toBe("NAV");
    const previous = screen.getByRole("link", { name: "Previous" });
    expect(previous.getAttribute("href")).toBe("/payments?ending_before=pi_2");
    expect(previous.className).toContain("link");
    expect(screen.getByRole("link", { name: "Next" })).toBeTruthy();
    unmount();
  });

  it("says where the end is rather than rendering a control that does nothing", () => {
    const { unmount } = render(
      <Pagination
        label="Payments paging"
        previous={null}
        next={<a href="/payments?starting_after=pi_9">Next</a>}
      />,
    );
    expect(screen.getByText("Newest first")).toBeTruthy();
    expect(screen.queryByRole("link", { name: "Previous" })).toBeNull();
    unmount();
  });

  it("renders nothing at all when there is nowhere to go", () => {
    const { container, unmount } = render(
      <Pagination label="Payments paging" previous={null} next={null} />,
    );
    expect(container.innerHTML).toBe("");
    unmount();
  });
});
