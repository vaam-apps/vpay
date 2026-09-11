import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PageShell } from "./page-shell";

describe("PageShell", () => {
  it("renders the one-column frame both apps share", () => {
    render(<PageShell data-testid="shell">Body</PageShell>);
    expect(screen.getByTestId("shell").className).toContain("max-w-md");
  });
});
