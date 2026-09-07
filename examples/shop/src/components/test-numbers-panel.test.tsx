// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { TestNumbersPanel } from "@/components/test-numbers-panel";

afterEach(() => {
  cleanup();
});

/**
 * Decisive mutation, plan §5 Lane C: "TestNumbersPanel's caveat role="alert"
 * is removed | fail". `orange_money` is the rail `src/lib/test-numbers.ts`
 * carries a `caveat` for — see its own comment on why the numbers do not
 * work from a browser today. A payer who never sees that caveat announced
 * would act on a table this panel already knows is misleading for that
 * rail, which is exactly what `role="alert"` exists to prevent for an
 * assistive-technology user who is not looking at the screen when the panel
 * mounts.
 */
describe("TestNumbersPanel", () => {
  it("announces the orange_money caveat as an alert", () => {
    render(<TestNumbersPanel rails={["mtn_momo", "orange_money"]} />);

    const caveat = screen.getByTestId("test-numbers-caveat-orange_money");
    expect(caveat).toHaveAttribute("role", "alert");
    expect(caveat).toHaveTextContent("Read this before you try them.");

    // mtn_momo carries no caveat in the fixture data — its testid must not
    // exist, so a mutation that hard-codes the caveat for every rail is
    // also caught.
    expect(
      screen.queryByTestId("test-numbers-caveat-mtn_momo"),
    ).not.toBeInTheDocument();
  });

  it("renders nothing for a deployment with no offered rails", () => {
    const { container } = render(<TestNumbersPanel rails={[]} />);
    expect(container).toBeEmptyDOMElement();
  });
});
