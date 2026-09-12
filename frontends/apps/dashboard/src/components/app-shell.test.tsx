/**
 * The signed-in chrome: one `<h1>`, a rail derived from the resource
 * registry, and a sign-out that is a POST.
 *
 * The brand heading used to live in the root layout and was asserted there.
 * It moved into this component with the rail, so its guarantees moved too —
 * otherwise the checks would have quietly stopped covering anything.
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("next/navigation", () => ({ usePathname: () => "/payments" }));

const { AppShell } = await import("./app-shell");
const { NAV_ENTRIES } = await import("../dash/resources");

function renderShell() {
  return render(
    <AppShell
      email="ops@example.test"
      merchantId="acct_test"
      signOut={() => Promise.resolve()}
    >
      <p>content</p>
    </AppShell>,
  );
}

describe("the signed-in shell", () => {
  it("renders the brand as the page's one <h1>", () => {
    const { container } = renderShell();
    const headings = container.querySelectorAll("h1");
    expect(headings).toHaveLength(1);
    expect(headings[0]?.textContent).toBe("vpay dashboard");
  });

  it("renders a link for every route the registry declares, and no others", () => {
    const { container } = renderShell();
    const hrefs = [...container.querySelectorAll("a[href^='/']")].map((a) =>
      a.getAttribute("href"),
    );
    for (const entry of NAV_ENTRIES) {
      expect(hrefs, `the rail links to ${entry.href}`).toContain(entry.href);
    }
    // Nothing the registry does not declare. Two lists that can disagree is
    // the failure Lane 4 exists to prevent.
    const declared = new Set<string>(NAV_ENTRIES.map((entry) => entry.href));
    for (const href of hrefs) {
      expect(declared.has(href ?? ""), `${href} is not a declared route`).toBe(
        true,
      );
    }
  });

  it("puts sign-out in a form, never behind a link", () => {
    // Issue #88 item 4: a sign-out reachable by GET is what an `<img src>` or
    // a link scanner fires. It was a POST before this shell existed and it
    // stays one.
    renderShell();
    const button = screen.getByRole("button", { name: /sign out/i });
    expect(button.getAttribute("type")).toBe("submit");
    expect(button.closest("form")).not.toBeNull();
  });

  it("shows who is signed in OUTSIDE the rail, where it is always visible", () => {
    // `SideNav` does not render `accountSlot` below `lg`. The identity lived
    // there for one revision and `dashboard.cy.ts`'s
    // `cy.contains(staffEmail()).should("be.visible")` failed at a 1000px
    // viewport — taking the eight tests after it in that sequence with it.
    // So this asserts the email is in the <main> region, not merely in the
    // document.
    const { container } = renderShell();
    const main = container.querySelector("main");
    expect(main?.textContent).toContain("ops@example.test");
    expect(main?.textContent).toContain("acct_test");
  });

  it("puts sign-out in the same always-visible region", () => {
    const { container } = renderShell();
    const main = container.querySelector("main");
    expect(main?.textContent).toMatch(/sign out/i);
  });

  it("renders its children", () => {
    renderShell();
    expect(screen.getByText("content")).toBeInTheDocument();
  });
});
