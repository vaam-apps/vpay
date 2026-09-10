import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import {
  Heading,
  List,
  Logo,
  PageShell,
  Stack,
  Text,
  VisuallyHidden,
} from "./layout";

describe("layout primitives", () => {
  it("Heading renders the requested level with the shared typography classes", () => {
    render(<Heading level={2}>Title</Heading>);
    const el = screen.getByRole("heading", { level: 2, name: "Title" });
    expect(el.className).toContain("text-xl");
  });

  it("Stack applies its direction, align and gap variants", () => {
    render(
      <Stack direction="column" gap="lg" data-testid="stack">
        <span>a</span>
      </Stack>,
    );
    const el = screen.getByTestId("stack");
    expect(el.className).toContain("flex-col");
    expect(el.className).toContain("gap-6");
  });

  it("Stack renders the element it is asked for, so a landmark survives", () => {
    // A stack that IS the page header is a `banner` landmark. Rendering it
    // as a `<div>` removes that landmark with no test and no lint failing,
    // which is how `checkout-view.tsx` and `return-view.tsx` lost theirs.
    render(
      <Stack as="header" justify="between" data-testid="header">
        <span>a</span>
      </Stack>,
    );
    const el = screen.getByTestId("header");
    expect(el.tagName).toBe("HEADER");
    expect(screen.getByRole("banner")).toBe(el);
    expect(el.className).toContain("justify-between");
  });

  it("Text applies tone and size without ever writing a raw opacity/text-error inline", () => {
    render(
      <Text tone="muted" size="xs">
        Support line
      </Text>,
    );
    const el = screen.getByText("Support line");
    expect(el.className).toContain("opacity-60");
    expect(el.className).toContain("text-xs");
  });

  it("Text carries the amount variants the checkout page needs, without a call-site class", () => {
    // These exist so `screens.tsx` writes no class of its own (plan §3: raw
    // utilities live only inside this package). The payment amount is the
    // one number a payer checks before approving.
    render(
      <Text size="3xl" weight="semibold" numeric data-testid="amount">
        5,000
      </Text>,
    );
    const amount = screen.getByTestId("amount");
    expect(amount.className).toContain("text-3xl");
    expect(amount.className).toContain("tabular-nums");
    // `text-lg` must NOT survive alongside `text-3xl`: two font sizes on one
    // element is a cascade coin-flip, which is what `cn()` is for.
    expect(amount.className).not.toContain("text-lg");

    render(
      <Text wrap="anywhere" data-testid="reference">
        cs_test_fixture000000000001
      </Text>,
    );
    expect(screen.getByTestId("reference").className).toContain("break-all");
  });

  it("VisuallyHidden is text for a screen reader and not for the eye", () => {
    render(<VisuallyHidden data-testid="vh">Amount:</VisuallyHidden>);
    const el = screen.getByTestId("vh");
    expect(el.tagName).toBe("SPAN");
    expect(el.className).toContain("sr-only");
    // Still in the accessibility tree — `sr-only` clips, it does not hide.
    expect(el.getAttribute("aria-hidden")).toBeNull();
  });

  it("Logo pins the operator mark to a height Tailwind preflight would otherwise drop", () => {
    render(
      <Logo
        src="https://operator.example/mark.svg"
        alt="Operator"
        data-testid="logo"
      />,
    );
    const el = screen.getByTestId("logo");
    expect(el.tagName).toBe("IMG");
    expect(el.getAttribute("alt")).toBe("Operator");
    expect(el.className).toContain("h-8");
    expect(el.className).toContain("w-auto");
  });

  it("List and PageShell render their fixed layout classes", () => {
    render(
      <PageShell data-testid="shell">
        <List data-testid="list">
          <li>Item</li>
        </List>
      </PageShell>,
    );
    expect(screen.getByTestId("shell").className).toContain("max-w-md");
    expect(screen.getByTestId("list").className).toContain("space-y-1");
  });
});
