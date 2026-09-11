import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Link } from "./link";

/** Stands in for `next/link`: the element and the navigation are the app's. */
function RouterLink(props: React.ComponentPropsWithoutRef<"a">) {
  return <a data-router="next" {...props} />;
}

describe("Link", () => {
  it("renders an anchor with the daisyUI link class by default", () => {
    const { unmount } = render(<Link href="/payments">Payments</Link>);
    const el = screen.getByRole("link", { name: "Payments" });
    expect(el.tagName).toBe("A");
    expect(el.className).toContain("link");
    expect(el.getAttribute("href")).toBe("/payments");
    unmount();
  });

  it("composes with a router's own link element instead of replacing it", () => {
    // Both survive — a `render` that dropped the caller's props would take
    // the href with it, and the link would navigate nowhere.
    const { unmount } = render(
      <Link render={<RouterLink href="/payments/pi_1" />} tone="hover">
        Detail
      </Link>,
    );
    const el = screen.getByRole("link", { name: "Detail" });
    expect(el.getAttribute("data-router")).toBe("next");
    expect(el.getAttribute("href")).toBe("/payments/pi_1");
    expect(el.className).toContain("link-hover");
    unmount();
  });
});
