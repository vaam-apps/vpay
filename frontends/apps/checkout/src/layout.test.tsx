/**
 * The root layout's `<head>`, which is a regression test before it is
 * anything else.
 *
 * This layout rendered its runtime theme override inside an **explicitly
 * written `<head>` element** for one revision. It looked right — the style
 * came out in the head of every response — and it broke the hosted payment
 * page in a real browser: React threw #418, *hydration failed because the
 * server rendered HTML did not match the client*, uncaught, and all three of
 * `frontends/tests/e2e/cypress/e2e/shop-hosted.cy.ts`'s tests died on it.
 * Measured three ways on 2026-09-07: the explicit `<head>` with a colour
 * configured failed, the same build with no `branding.yaml` (and therefore no
 * child in that `<head>`) passed, and `href`/`precedence` with a colour
 * configured passed.
 *
 * The reason is that an explicitly rendered `<head>` is an ordinary host
 * element whose children React reconciles exactly, and a Next App Router
 * document's head is full of nodes React did not put there. React 19's own
 * hoisting (`href` + `precedence`) writes the style into the head as a
 * *resource* instead, which is reconciled by `data-href` rather than by
 * position.
 *
 * So the assertions below are not about tidiness. `<head>` must not appear in
 * this layout's output, and the style must carry the two props that hoist it.
 */
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("next/headers", () => ({
  headers: () => Promise.resolve(new Headers({ "accept-language": "en" })),
}));

const BRANDING = `${import.meta.dirname}/../../../../config/checkout/branding.example.yaml`;
const CONFIG = `${import.meta.dirname}/../../../../config/checkout/config.example.yaml`;
process.env["VPAY_CHECKOUT_BRANDING_FILE"] = BRANDING;
process.env["VPAY_CHECKOUT_CONFIG_FILE"] = CONFIG;

const {
  default: RootLayout,
  THEME_OVERRIDE_HREF,
  THEME_OVERRIDE_PRECEDENCE,
} = await import("../app/layout");

async function tree(): Promise<ReactElement> {
  return (await RootLayout({ children: null })) as ReactElement;
}

async function markup(): Promise<string> {
  return renderToStaticMarkup(await tree());
}

/**
 * Every host element type this layout renders.
 *
 * The assertion has to be on the ELEMENT TREE and not on the markup: React's
 * own hoisting produces a `<head>` in the output — that is what hoisting is —
 * and the defect was a `<head>` this file rendered *itself*, whose children
 * React then had to reconcile against a document head it does not own.
 */
function hostTypes(node: ReactNode, seen: string[] = []): string[] {
  if (Array.isArray(node)) {
    for (const child of node as readonly ReactNode[]) {
      hostTypes(child, seen);
    }
    return seen;
  }
  if (!isValidElement(node)) {
    return seen;
  }
  if (typeof node.type === "string") {
    seen.push(node.type);
  }
  hostTypes((node.props as { children?: ReactNode }).children, seen);
  return seen;
}

describe("the root layout", () => {
  it("renders NO explicit <head> element — the thing that broke hydration", async () => {
    const types = hostTypes(await tree());
    expect(types).toContain("html");
    expect(types).toContain("body");
    expect(types).toContain("style");
    expect(
      types,
      "a <head> this layout renders itself is the defect",
    ).not.toContain("head");
  });

  it("hoists the theme override with href and precedence, so it is a resource and not a child", async () => {
    const html = await markup();
    expect(html).toContain(`href="${THEME_OVERRIDE_HREF}"`);
    expect(html).toContain(`precedence="${THEME_OVERRIDE_PRECEDENCE}"`);
    // And it is still the colour the mounted file asked for — daisyUI 5's
    // --color-primary takes a CSS colour directly, so this is the operator's
    // #rrggbb re-emitted rather than converted.
    expect(html).toContain("--color-primary:#f3c623;");
  });

  it("renders the theme this app is themed with, and the negotiated language", async () => {
    const html = await markup();
    // `@vaam-apps/ui`'s theme registers under daisyUI's built-in name
    // "dark" (2026-09-12 cutover) — it was "bumblebee" before that, which
    // no longer compiles at all now that `app/globals.css` sets
    // `themes: false`.
    expect(html).toContain('data-theme="dark"');
    expect(html).toContain('lang="en"');
  });
});
