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

  it("ignores a caller that tries to weaken the announcement anyway", () => {
    // `LiveRegionProps` omits both attributes, so this cannot be written at a
    // typed call site — the cast is what a spread of a widened props object
    // does in practice. The decisive mutation is putting `{...rest}` back
    // after the two attributes in `live-region.tsx`: this case then reports
    // `aria-atomic` as "false", which is a payment outcome announced in
    // pieces or not at all.
    const hostile = {
      "data-testid": "live",
      "aria-live": "off",
      "aria-atomic": "false",
      children: "Failed",
    } as unknown as React.ComponentProps<typeof LiveRegion>;
    const { unmount } = render(<LiveRegion {...hostile} />);
    const el = screen.getByTestId("live");
    expect(el.getAttribute("aria-live")).toBe("polite");
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
