import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { axeViolations } from "./testing/axe";
import { KitchenSink } from "./testing/kitchen-sink";

describe("@vpay/ui structural accessibility", () => {
  /**
   * The negative control, and it runs FIRST on purpose.
   *
   * "Zero violations" is only evidence if the harness can report a violation
   * at all — a mis-scoped container, a `runOnly` list of rule ids that no
   * longer exist, or an `axeViolations` that swallowed its result would each
   * produce a green run over a broken page. `docs/status.md` recorded this
   * check as having been done by hand once; a check nobody can re-run is a
   * claim, so it is a test now.
   */
  it("reports a violation when there is one (the harness is not asleep)", async () => {
    const { unmount } = render(
      <div>
        <button type="button" />
      </div>,
    );
    const violations = await axeViolations(document.body);
    expect(violations.map((violation) => violation.id)).toContain(
      "button-name",
    );
    unmount();
  });

  it("has zero violations for label, button-name, aria-*, region and list", async () => {
    const { unmount } = render(<KitchenSink />);
    const violations = await axeViolations(document.body);
    expect(violations).toEqual([]);
    unmount();
  });
});
