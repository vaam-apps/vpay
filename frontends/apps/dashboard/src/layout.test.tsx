/**
 * The root layout: the theme, the brand, and the nav-honesty gate.
 *
 * `docs/plans/2026-09-07-ui-revamp.md` §5 Lane D names the theme regression
 * as a decisive mutation: "the layout's `data-theme` reverts to `corporate`"
 * must fail. Measured while writing it (`docs/plans/exp26-notes/lane-d.md`):
 * `frontends/packages/ui/src/styles.css` configured daisyUI with exactly one
 * theme (`bumblebee`), so a `data-theme` that did not say `bumblebee`
 * compiled no styling at all — the page rendered completely unthemed in a
 * real browser. jsdom would not show that, which is why this pins the
 * attribute on the rendered markup rather than trusting a render to "look
 * right".
 *
 * **Updated 2026-09-12:** `@vpay/ui` was deleted; the dashboard now composes
 * `@vaam-apps/ui`. That package's `theme.css` registers its one theme under
 * daisyUI's built-in name `dark` (its own README and file header both say
 * so — it is the only theme the package compiles), so the decisive mutation
 * and the reasoning are unchanged in substance: a `data-theme` that does not
 * say `dark` still compiles no styling at all. Only the theme's name moved.
 */
import { existsSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import RootLayout from "../app/layout";
import { DASH_RESOURCES, NAV_ENTRIES } from "./dash/resources";

function markup(): string {
  return renderToStaticMarkup(
    RootLayout({ children: <p>content</p> }) as ReactElement,
  );
}

describe("the root layout", () => {
  it("sets the dark theme — the only theme @vaam-apps/ui actually compiles", () => {
    const html = markup();
    expect(html).toContain('data-theme="dark"');
    expect(html).not.toContain('data-theme="corporate"');
    expect(html).not.toContain('data-theme="bumblebee"');
  });

  it("renders no <h1> of its own — the brand moved to the signed-in rail", () => {
    // It used to be an <h1> inside a <nav> this layout rendered for EVERY
    // route, signed in or not, so the sign-in form carried a link to a page
    // nobody at that point could open. `AppShell` renders it now, inside
    // `app/(dash)` — the only group a signed-in staff member is in — and
    // `app-shell.test.tsx` asserts it is still exactly one <h1>.
    expect(markup()).not.toMatch(/<h1/);
  });

  it("carries the brand in <title> rather than in every route's markup", () => {
    expect(markup()).not.toContain("vpay dashboard");
  });

  it("renders no signed-in control — this layout is on /login too", () => {
    // "Sign out" in chrome shared with a page where nobody is signed in is
    // the same kind of claim as a nav entry for a page nobody wrote.
    expect(markup()).not.toMatch(/sign out/i);
  });
});

const APP_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", "app");

/**
 * Does `app/` actually have a page for this route?
 *
 * App Router: `/` is `app/page.tsx`, `/payments` is `app/payments/page.tsx`.
 * Route groups are not handled because this app has none; a dynamic segment
 * is resolved by looking for a `[…]` directory, which is what
 * `/payments/{id}` is served by.
 */
/**
 * Does a URL path have a page file?
 *
 * **Route groups are why this is not a plain `join`.** `app/(dash)/payments`
 * serves `/payments`: a `(group)` segment is a filesystem directory that
 * contributes nothing to the URL. A naive join looks for
 * `app/payments/page.tsx`, finds nothing, and reports every signed-in route
 * as dangling — which is exactly what it did when the payments routes moved
 * into `(dash)`.
 *
 * So this tries the direct path first, then the same path under each
 * top-level group. It stays a **negative** control as well: `/webhooks` has
 * no page in any group and must still answer `false`.
 */
function pageExists(route: string): boolean {
  const segments = route.split("/").filter(Boolean);
  const exts = ["tsx", "ts", "jsx", "js"];
  const direct = exts.some((ext) =>
    existsSync(join(APP_DIR, ...segments, `page.${ext}`)),
  );
  if (direct) {
    return true;
  }
  const groups = readdirSync(APP_DIR, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && entry.name.startsWith("("))
    .map((entry) => entry.name);
  return groups.some((group) =>
    exts.some((ext) =>
      existsSync(join(APP_DIR, group, ...segments, `page.${ext}`)),
    ),
  );
}

/**
 * The nav-honesty rule, as a gate rather than as a comment.
 *
 * `docs/flows/dashboard.md`: "the navigation is only ever allowed to link to
 * slices that exist. A menu entry for a page nobody wrote is the same lie as
 * an empty table." That rule was stated in this app's README, in
 * `app/layout.tsx`'s doc comment and in the flow doc, and enforced by nothing
 * at all until this test: a dangling `<a href="/payments">` left the whole
 * suite, `lint` and `dashboard.cy.ts` green (measured —
 * `docs/plans/exp26-notes/lane-d-review.md`, finding 2).
 *
 * It is checked **twice**, and the pair is the point. The rendered markup
 * catches a link written straight into the JSX. `NAV_LINKS` catches one
 * added to the constant but rendered only in a branch this test does not
 * exercise — which is what a conditional nav would make possible.
 */
describe("the nav links only to pages that exist", () => {
  it("every route the rail offers has a page under app/", () => {
    const dangling = NAV_ENTRIES.filter((entry) => !pageExists(entry.href));
    expect(dangling).toEqual([]);
  });

  it("every resource Refine routes on has a page under app/", () => {
    // Lane 4's decisive mutation: add a `resources` entry for a page nobody
    // wrote and this must fail. It is the same mutation that used to fail
    // against `NAV_LINKS`, moved onto the array that now does both jobs.
    const dangling = DASH_RESOURCES.filter(
      (entry) =>
        typeof entry.resource.list === "string" &&
        !pageExists(entry.resource.list),
    );
    expect(dangling).toEqual([]);
  });

  it("the rail and Refine read the SAME array, so they cannot disagree", () => {
    // The failure this replaces: two lists, one of them stale, and a link to
    // a page that no longer exists. `NAV_ENTRIES` is derived from
    // `DASH_RESOURCES` rather than written beside it.
    const listRoutes = DASH_RESOURCES.map((entry) => entry.resource.list)
      .filter((route): route is string => typeof route === "string")
      .sort();
    expect(NAV_ENTRIES.map((entry) => entry.href).sort()).toEqual(listRoutes);
    expect(listRoutes.length).toBeGreaterThan(0);
  });

  it("the helper it relies on is not vacuously true", () => {
    // A negative control: if `pageExists` answered `true` for everything the
    // tests above would pass whatever the layout linked to.
    expect(pageExists("/payments")).toBe(true);
    expect(pageExists("/webhooks")).toBe(false);
  });
});
