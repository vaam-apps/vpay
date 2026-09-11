import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Timeline } from "./timeline";

const EVENTS = [
  { id: "evt_1", at: "2026-09-11 09:00:00", label: "charge.succeeded" },
  { id: "evt_2", at: "2026-09-11 09:00:04", label: "charge.settled" },
];

describe("Timeline", () => {
  it("renders one list item per event, newest ordering left to the caller", () => {
    const { unmount } = render(
      <Timeline items={EVENTS} emptyMessage="No events yet." />,
    );
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
    expect(screen.getByText("charge.succeeded")).toBeTruthy();
    expect(screen.getByText("2026-09-11 09:00:04")).toBeTruthy();
    unmount();
  });

  it("says so rather than rendering a blank section when there is nothing", () => {
    const { unmount } = render(
      <Timeline items={[]} emptyMessage="No events yet." />,
    );
    expect(screen.getByText("No events yet.")).toBeTruthy();
    expect(screen.queryByRole("list")).toBeNull();
    unmount();
  });
});
