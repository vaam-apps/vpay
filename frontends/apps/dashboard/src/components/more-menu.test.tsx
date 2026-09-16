/**
 * The "Menu" drawer: the nav tree it renders, the accessible name and
 * description vaul requires, the theme control it exists to make reachable,
 * the sign-out that must stay a POST inside it too, and the current-entry
 * rule it has to share with the rail.
 *
 * **Every case here opens the drawer.** A closed vaul drawer renders none of
 * its children — not hidden, absent — so a suite that only mounted
 * `MoreMenu` would assert against an empty portal and pass against a
 * component that does nothing. `openDrawer()` below is the guard: it clicks
 * the trigger and then fails outright if no dialog appears, so a drawer that
 * silently stops opening is a red test rather than a green one.
 */
import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

/** Mutable so one case can render a DETAIL route; `vi.mock` is hoisted
 * above every `const`, so the box it reads has to be hoisted with it. */
const nav = vi.hoisted(() => ({ pathname: "/payments" }));
vi.mock("next/navigation", () => ({
  usePathname: () => nav.pathname,
  useRouter: () => ({ push: vi.fn() }),
}));

const { MoreMenu } = await import("./more-menu");
const { AppShell } = await import("./app-shell");
const { NAV_ENTRIES } = await import("../dash/resources");

function renderMenu(currentPath = "/payments") {
  return render(
    <MoreMenu
      email="ops@example.test"
      merchantId="acct_test"
      signOut={() => Promise.resolve()}
      currentPath={currentPath}
    />,
  );
}

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
 * Click "Menu" and return the open panel, failing if it never opened.
 *
 * **`getAllBy`, and the first one, because `AppShell` mounts the trigger
 * twice on purpose.** One goes in `SideNav`'s `accountSlot`, which that
 * component renders **only** in the ≥1280px in-flow sidebar; the other sits
 * in `<main>` behind `xl:hidden` so every narrower width can still reach the
 * theme control and the nav tree. Exactly one is ever visible in a browser —
 * but jsdom applies no CSS, so both are here and a `getBy` throws "found
 * multiple elements".
 *
 * Which one this clicks does not matter to anything below: both render the
 * same `MoreMenu` with the same props, only one drawer can be open at a
 * time, and `MoreDetailDrawer` is modal — Radix `aria-hidden`s the rest of
 * the document while it is open, which is what keeps the sign-out assertion
 * below true with two triggers mounted.
 */
async function openDrawer(): Promise<HTMLElement> {
  const triggers = screen.getAllByRole("button", { name: /^menu$/i });
  triggers[0]!.click();
  return await screen.findByRole("dialog");
}

describe("the More drawer", () => {
  it("renders one row per NAV_ENTRIES entry, and nothing else", async () => {
    // Asserted against the ARRAY, never against the literal "Payments".
    // `NAV_ENTRIES` has one element today because `/dash/v1` serves one
    // resource; the point of this case is that it still holds when a later
    // lane adds the second. Both directions: every declared entry is drawn,
    // and nothing undeclared is — two lists that can disagree is the exact
    // failure `resources.ts` exists to prevent.
    renderMenu();
    const panel = await openDrawer();
    const tree = within(panel).getByRole("navigation", {
      name: "Menu destinations",
    });
    const links = within(tree).getAllByRole("link");

    expect(links).toHaveLength(NAV_ENTRIES.length);
    expect(links.map((a) => a.getAttribute("href"))).toEqual(
      NAV_ENTRIES.map((entry) => entry.href),
    );
    for (const entry of NAV_ENTRIES) {
      expect(
        within(tree).getByRole("link", { name: new RegExp(entry.label, "i") }),
        `the drawer offers ${entry.label}`,
      ).toHaveAttribute("href", entry.href);
    }
  });

  it("carries the title and description vaul requires, both wired up", async () => {
    // `DrawerTitle`/`DrawerDescription` are not decoration: vaul renders
    // Radix's dialog content underneath, which warns in dev without a title
    // and leaves `aria-describedby` pointing at nothing without a
    // description. Asserting the ELEMENTS exist would not catch a broken
    // wiring, so this asserts the dialog's own accessible NAME and that its
    // `aria-describedby` resolves to an element carrying the real text.
    renderMenu();
    const panel = await openDrawer();

    expect(panel).toHaveAccessibleName("Menu");

    const describedBy = panel.getAttribute("aria-describedby");
    expect(describedBy, "the drawer names a description element").toBeTruthy();
    const description = document.getElementById(describedBy ?? "");
    expect(description, `#${describedBy} exists`).not.toBeNull();
    expect(description?.textContent).toBe(
      "Navigation, theme, and your account.",
    );
  });

  it("puts the theme control inside the drawer, where it is reachable", async () => {
    // The reason this component exists. `SideNav` renders `accountSlot` only
    // in the >=1280px in-flow sidebar, so below that width the drawer's copy
    // is the ONLY theme control on the page. Reached through the roles a
    // user reaches it by, rather than by querying for the component.
    renderMenu();
    const panel = await openDrawer();
    const group = within(panel).getByRole("radiogroup");
    const options = within(group).getAllByRole("radio");

    expect(options.length).toBeGreaterThan(1);
    expect(options.map((o) => o.textContent).join(" ")).toMatch(/dark/i);
  });

  it("keeps sign-out a form POST inside the drawer, never a link", async () => {
    // Issue #88 item 4, applied to the drawer's own copy: a sign-out
    // reachable by GET is what a prefetcher or an `<img src>` fires. The
    // `<main>` copy is covered by `app-shell.test.tsx`; this is the second
    // mount, which that case cannot see.
    renderMenu();
    const panel = await openDrawer();
    const button = within(panel).getByRole("button", { name: /sign out/i });

    expect(button.getAttribute("type")).toBe("submit");
    expect(button.closest("form")).not.toBeNull();
    expect(
      within(panel).queryByRole("link", { name: /sign out/i }),
      "sign-out is never a link",
    ).toBeNull();
  });

  it("marks the current entry the way the rail does, on a detail route", async () => {
    // THE REGRESSION THIS CASE EXISTS FOR. The drawer shipped with
    // `entry.href === currentPath`, while `SideNav`'s rule is a prefix match
    // at a segment boundary — so on `/payments/pi_3Nk` the rail marked
    // Payments current and the drawer marked nothing.
    //
    // Deliberately NOT asserted by restating `isNavActive`'s expression,
    // which would pass against any rule so long as both copies were wrong in
    // the same way. This renders the whole shell at a detail path and asserts
    // the RAIL and the DRAWER agree href by href. `SideNav`'s `isActive` is
    // private to `@vaam-apps/ui`, so its rendered `aria-current` is the only
    // honest oracle available — and if that package ever changes its rule,
    // this case is what notices.
    const entry = NAV_ENTRIES[0];
    expect(entry, "there is a nav entry to test with").toBeDefined();
    nav.pathname = `${entry?.href ?? "/payments"}/pi_3Nk`;

    renderShell();
    const panel = await openDrawer();

    const marks = (root: ParentNode, href: string) =>
      [...root.querySelectorAll(`a[href="${href}"]`)].some(
        (a) => a.getAttribute("aria-current") === "page",
      );
    // The rail is everything outside the open panel: `SideNav` portals its
    // floating rails to `document.body`, so "the container" is not a
    // boundary here and the dialog is.
    const railLinks = [...document.body.querySelectorAll("a")].filter(
      (a) => !panel.contains(a),
    );
    const rail = {
      querySelectorAll: (sel: string) =>
        railLinks.filter((a) => a.matches(sel)),
    };

    for (const each of NAV_ENTRIES) {
      const railMarks = rail
        .querySelectorAll(`a[href="${each.href}"]`)
        .some((a) => a.getAttribute("aria-current") === "page");
      expect(
        marks(panel, each.href),
        `the drawer and the rail must agree about ${each.href} at ${nav.pathname}`,
      ).toBe(railMarks);
    }

    // And not vacuously: on this path the rail really does mark it, so
    // "neither marks anything" cannot pass the loop above.
    const railMarksFirst = rail
      .querySelectorAll(`a[href="${entry?.href ?? ""}"]`)
      .some((a) => a.getAttribute("aria-current") === "page");
    expect(
      railMarksFirst,
      `the rail marks ${entry?.href} current at ${nav.pathname}`,
    ).toBe(true);

    nav.pathname = "/payments";
  });

  it("never puts two sign-outs in the accessibility tree at once", async () => {
    // `AppShell` mounts `SignedInBar` three times — in `SideNav`'s
    // `accountSlot` (>=1280px only), in `<main>` behind `xl:hidden` for
    // every narrower width (which `dashboard.cy.ts` needs at its 1000px
    // viewport), and once in this drawer. That
    // is safe rather than a duplication bug, and this is the measurement
    // behind that claim rather than an argument for it: `MoreDetailDrawer`
    // is the dimmed, MODAL variant, so Radix marks the rest of the document
    // `aria-hidden` while it is open. Exactly one sign-out is reachable in
    // either state — the `<main>` copy leaves the tree as the drawer's copy
    // enters it.
    //
    // This is also what keeps `app-shell.test.tsx`'s `getByRole("button",
    // { name: /sign out/i })` unambiguous, which would otherwise throw
    // "found multiple elements" the moment a case opened the drawer.
    renderShell();
    // ONE when closed. The account block is mounted in `<main>` behind
    // `xl:hidden`, and `SideNav`'s `accountSlot` is given content only when
    // the sidebar is actually shown (>=1280px) — `app-shell.tsx` says why
    // at length. jsdom's `matchMedia` reports no match, so the rail copy is
    // not in this tree at all.
    //
    // That gating exists because the slot is rendered at EVERY width and
    // merely hidden by CSS, so a second copy would be FIRST in the DOM and
    // invisible — which is exactly how `dashboard.cy.ts` came to click a
    // sign-out it could not see.
    const closed = screen.getAllByRole("button", { name: /sign out/i });
    expect(closed, "closed: only the <main> copy").toHaveLength(1);

    await openDrawer();
    expect(
      screen.getAllByRole("button", { name: /sign out/i }),
      "open: only the drawer copy",
    ).toHaveLength(1);
  });

  it("leaves the document with exactly one <h1> when it is open", async () => {
    // `DrawerTitle` renders a heading, and `dashboard.cy.ts` asserts the
    // shell's `<h1>`/`<h2>` hierarchy in thirteen places. Radix's
    // `Dialog.Title` is an `<h2>`, so opening the drawer adds a second-level
    // heading and not a competing top-level one — pinned here because that
    // is a fact about the PACKAGE, which a version bump could change.
    renderShell();
    await openDrawer();

    const h1s = document.body.querySelectorAll("h1");
    expect(h1s).toHaveLength(1);
    expect(h1s[0]?.textContent).toBe("vpay dashboard");
    expect(
      [...document.body.querySelectorAll("h2")].map((h) => h.textContent),
    ).toContain("Menu");
  });
});
