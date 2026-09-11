import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Input } from "./input";

/**
 * `Input` had no test of its own — `field.test.tsx` renders one to prove
 * Base UI's label association, which covers the composition but not this
 * component's own `cva` map. Plan §5 Lane A's acceptance is "a case per
 * component".
 */
describe("Input", () => {
  it("is bordered by default — daisyUI 5 removed input-bordered", () => {
    const { unmount } = render(<Input aria-label="MSISDN" />);
    const el = screen.getByRole("textbox", { name: "MSISDN" });
    expect(el.className).toContain("input");
    expect(el.className).not.toContain("input-bordered");
    expect(el.className).not.toContain("input-ghost");
    unmount();
  });

  it('tone="ghost" is the borderless variant, tone="error" the invalid one', () => {
    const { unmount } = render(<Input tone="ghost" aria-label="A" />);
    expect(screen.getByRole("textbox", { name: "A" }).className).toContain(
      "input-ghost",
    );
    unmount();

    const second = render(<Input tone="error" aria-label="B" />);
    expect(screen.getByRole("textbox", { name: "B" }).className).toContain(
      "input-error",
    );
    second.unmount();
  });

  it("a className override wins over the variant, via cn", () => {
    const { unmount } = render(
      <Input tone="ghost" className="input-error" aria-label="C" />,
    );
    const el = screen.getByRole("textbox", { name: "C" });
    expect(el.className).toContain("input-error");
    expect(el.className).not.toContain("input-ghost");
    unmount();
  });
});
