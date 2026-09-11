import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Section } from "./section";

describe("Section", () => {
  it("is a region named by its own heading", () => {
    const { unmount } = render(<Section title="Charge">No charge.</Section>);
    // `getByRole('region')` only matches a <section> that HAS an accessible
    // name. A heading merely sitting inside one does not give it a name —
    // the `aria-labelledby` wiring does, and this is what proves it is there.
    const region = screen.getByRole("region", { name: "Charge" });
    expect(region.tagName).toBe("SECTION");
    expect(
      screen.getByRole("heading", { level: 2, name: "Charge" }),
    ).toBeTruthy();
    unmount();
  });

  it("renders the heading level it is asked for", () => {
    const { unmount } = render(<Section title="Support" level={3} />);
    expect(
      screen.getByRole("heading", { level: 3, name: "Support" }),
    ).toBeTruthy();
    unmount();
  });
});
