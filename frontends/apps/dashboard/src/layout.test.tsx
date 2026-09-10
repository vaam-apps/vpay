/**
 * The root layout: the theme, the brand, and the nav-honesty gate.
 *
 * `docs/plans/2026-09-07-ui-revamp.md` §5 Lane D names the theme regression
 * as a decisive mutation: "the layout's `data-theme` reverts to `corporate`"
 * must fail. Measured while writing it (`docs/plans/exp26-notes/lane-d.md`):
 * `frontends/packages/ui/src/styles.css` configures daisyUI with exactly one
 * theme (`bumblebee`), so a `data-theme` that does not say `bumblebee`
 * compiles no styling at all — the page renders completely unthemed in a real
 * browser. jsdom would not show that, which is why this pins the attribute on
 * the rendered markup rather than trusting a render to "look right".
 */
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import RootLayout from "../app/layout";
import { NAV_LINKS } from "./nav";

function markup(): string {
  return renderToStaticMarkup(
    RootLayout({ children: <p>content</p> }) as ReactElement,
  );
}

describe("the root layout", () => {
  it("sets the bumblebee theme — the only theme @vpay/ui actually compiles", () => {
    const html = markup();
    expect(html).toContain('data-theme="bumblebee"');
    expect(html).not.toContain('data-theme="corporate"');
  });

  it("renders the brand as the page's one <h1>, inside the <nav>", () => {
    // Real element nesting, on the rendered markup — not the unrendered
    // element tree, which stops at `Heading`/`PageShell` (function
    // components) and never reaches the `<h1>` they render internally.
    expect(markup()).toMatch(
      /<nav>[\s\S]*<h1[^>]*>vpay dashboard<\/h1>[\s\S]*<\/nav>/,
    );
  });

  it("renders the brand text exactly once", () => {
    const matches = markup().match(/vpay dashboard/g) ?? [];
    expect(matches).toHaveLength(1);
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
function pageExists(route: string): boolean {
  const segments = route.split("/").filter(Boolean);
  return ["tsx", "ts", "jsx", "js"].some((ext) =>
    existsSync(join(APP_DIR, ...segments, `page.${ext}`)),
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
  it("every internal href in the rendered layout has a page under app/", () => {
    const hrefs = [...markup().matchAll(/href="([^"]*)"/g)].map(
      (m) => m[1] ?? "",
    );
    const internal = hrefs.filter((h) => h.startsWith("/"));
    const dangling = internal.filter(
      (h) => !pageExists(h.split(/[?#]/)[0] ?? ""),
    );

    expect(dangling).toEqual([]);
  });

  it("every entry in NAV_LINKS has a page under app/, rendered or not", () => {
    const dangling = NAV_LINKS.filter((link) => !pageExists(link.href));
    expect(dangling).toEqual([]);
  });

  it("the nav actually renders the links it declares", () => {
    // Otherwise the constant could pass while the layout linked elsewhere.
    const html = markup();
    for (const link of NAV_LINKS) {
      expect(html).toContain(`href="${link.href}"`);
    }
  });

  it("the helper it relies on is not vacuously true", () => {
    // A negative control: if `pageExists` answered `true` for everything the
    // tests above would pass whatever the layout linked to.
    expect(pageExists("/payments")).toBe(true);
    expect(pageExists("/webhooks")).toBe(false);
  });
});
