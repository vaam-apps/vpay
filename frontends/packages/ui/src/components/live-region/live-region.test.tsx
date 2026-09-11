import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { LiveRegion } from "./live-region";

describe("LiveRegion", () => {
  it("announces politely and atomically by default", () => {
    const { unmount } = render(
      <LiveRegion data-testid="live">Waiting</LiveRegion>,
    );
    const el = screen.getByTestId("live");
    expect(el.getAttribute("aria-live")).toBe("polite");
    // Not a prop: a partial announcement of a payment outcome is worse than
    // none, so no call site may turn this off.
    expect(el.getAttribute("aria-atomic")).toBe("true");
    unmount();
  });

  it("can interrupt when a caller asks it to", () => {
    const { unmount } = render(
      <LiveRegion politeness="assertive" data-testid="live">
        Failed
      </LiveRegion>,
    );
    expect(screen.getByTestId("live").getAttribute("aria-live")).toBe(
      "assertive",
    );
    unmount();
  });
});
