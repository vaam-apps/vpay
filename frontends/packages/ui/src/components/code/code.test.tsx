import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Code } from "./code";

describe("Code", () => {
  it("renders a real <code> element", () => {
    const { unmount } = render(
      <Code data-testid="id">pi_test_000000000000000001</Code>,
    );
    expect(screen.getByTestId("id").tagName).toBe("CODE");
    unmount();
  });

  it("breaks a long identifier only when asked to", () => {
    const { unmount } = render(
      <>
        <Code data-testid="plain">pi_1</Code>
        <Code wrap="anywhere" data-testid="long">
          pi_test_000000000000000001
        </Code>
      </>,
    );
    expect(screen.getByTestId("plain").className).not.toContain("break-all");
    expect(screen.getByTestId("long").className).toContain("break-all");
    unmount();
  });
});
