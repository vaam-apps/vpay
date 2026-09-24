/**
 * The signed-in chrome: one `<h1>`, a rail derived from the resource
 * registry, a sign-out that is a POST, and an account block the phone bar's
 * "More" sheet can reach.
 *
 * The brand heading used to live in the root layout and was asserted there.
 * It moved into this component with the rail, so its guarantees moved too —
 * otherwise the checks would have quietly stopped covering anything.
 *
 * jsdom applies no CSS, so every shape `SideNav` renders is "visible" here
 * at once: the in-flow sidebar and both portalled toolbars. Which one a
 * browser shows, and that exactly one sign-out is exposed at each width, is
 * measured in real Chromium by the `Shell*` stories in
 * `dashboard-screens.stories.tsx`, not asserted here.
 */
import { render, screen, within } from "@testing-library/react";
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

/**
 * Open the phone bar's More sheet and return it, failing if it never opens.
 *
 * The phone bar and not the rail because it is the stricter case. With no
 * `accountSlot` the rail gains no control at all, but the phone bar keeps
 * one whenever destinations overflow its four slots, which this app's five
 * do: named "More destinations", opening a `menu` of links. A control
 * named exactly "More", opening a dialog, is there only because the
 * account block was passed, and that dialog is where the block lands.
 *
 * Found through the bar's own hook (`data-floating-rail-axis`), because the
 * vertical rail carries a second "More" control and jsdom shows both. A
 * closed vaul sheet renders none of its children, so everything asserted
 * about the account block below is asserted on an OPEN sheet.
 */
async function openPhoneSheet(): Promise<HTMLElement> {
  const bar = document.body.querySelector<HTMLElement>(
    '[data-floating-rail-axis="horizontal"]',
  );
  expect(bar, "the phone bar is portalled into the document").not.toBeNull();
  within(bar as HTMLElement)
    .getByRole("button", { name: "More" })
    .click();
  return await screen.findByRole("dialog", { name: "More" });
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
    // EVERY mount, not just the first. With the sheet shut the account block
    // is rendered twice — in the sidebar's `accountSlot` (hidden by CSS below
    // `xl`) and in `<main>` (from `sm` to `xl`) — so a `getBy` would check
    // one and leave the other unasserted, which is exactly how a GET-able
    // sign-out could slip in on one breakpoint only. The sheet's own copy is
    // the next case's.
    const buttons = screen.getAllByRole("button", { name: /sign out/i });
    expect(buttons, "the sidebar's copy and <main>'s").toHaveLength(2);
    for (const button of buttons) {
      expect(button.getAttribute("type")).toBe("submit");
      expect(button.closest("form")).not.toBeNull();
    }
  });

  it("puts the account block in the phone bar's More sheet", async () => {
    // THE REGRESSION THIS CASE EXISTS FOR. Until `@vaam-apps/ui` 0.4.0 this
    // app gave `accountSlot` content only while a `matchMedia` said the
    // sidebar was showing, and floated its own Menu pill everywhere else.
    // Re-adding that gate empties this sheet — and, with no account block,
    // the phone bar's last control goes back to a menu named "More
    // destinations", so `openPhoneSheet` fails first.
    renderShell();
    const sheet = await openPhoneSheet();

    expect(within(sheet).getByText("ops@example.test")).toBeInTheDocument();
    expect(within(sheet).getByText("acct_test")).toBeInTheDocument();

    const signOut = within(sheet).getByRole("button", { name: /sign out/i });
    expect(signOut.getAttribute("type")).toBe("submit");
    expect(signOut.closest("form")).not.toBeNull();
    expect(
      within(sheet).queryByRole("link", { name: /sign out/i }),
      "sign-out is never a link",
    ).toBeNull();

    const theme = within(sheet).getByRole("radiogroup", { name: "Theme" });
    expect(
      within(theme)
        .getAllByRole("radio")
        .map((radio) => radio.textContent)
        .join(" "),
    ).toMatch(/dark/i);

    // Modal: Radix hides the rest of the document from the accessibility
    // tree while the sheet is open, so the sidebar's and `<main>`'s copies
    // leave it as the sheet's enters.
    expect(
      screen.getAllByRole("button", { name: /sign out/i }),
      "open: only the sheet's copy is exposed",
    ).toEqual([signOut]);
  });

  it("leaves the document with exactly one <h1> while the sheet is open", async () => {
    // The sheet's title is Radix's `Dialog.Title`, an <h2>, so opening it
    // adds no second <h1>: one <h1> naming the app, and an <h2> naming each
    // screen, which `dashboard.cy.ts` relies on when it finds a screen by
    // `cy.contains("h2", …)` (fifteen times as of 2026-09-24; it asserts no
    // <h1>). Pinned because it is a fact about the PACKAGE, which a
    // version bump could change.
    renderShell();
    await openPhoneSheet();

    const h1s = document.body.querySelectorAll("h1");
    expect(h1s).toHaveLength(1);
    expect(h1s[0]?.textContent).toBe("vpay dashboard");
  });

  it("shows who is signed in OUTSIDE the rail as well", () => {
    // From 640 to 1279px the identity is in `<main>` and nowhere else on
    // screen without a tap, and `dashboard.cy.ts` asserts
    // `cy.get("main").contains(staffEmail()).should("be.visible")` at its
    // 1000px viewport — taking the eight tests that follow it in that
    // `testIsolation: false` sequence with it if it is missing. So this
    // asserts the email is in the <main> region, not merely in the
    // document.
    const { container } = renderShell();
    const main = container.querySelector("main");
    expect(main?.textContent).toContain("ops@example.test");
    expect(main?.textContent).toContain("acct_test");
  });

  it("puts sign-out in the same region", () => {
    const { container } = renderShell();
    const main = container.querySelector("main");
    expect(main?.textContent).toMatch(/sign out/i);
  });

  it("renders its children", () => {
    renderShell();
    expect(screen.getByText("content")).toBeInTheDocument();
  });
});
