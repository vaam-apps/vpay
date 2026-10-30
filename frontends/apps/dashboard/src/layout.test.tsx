/**
 * The root layout: the theme and the brand, and the gate that names the pages
 * this app has.
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
 *
 * **Corrected 2026-09-23:** the paragraph above, and this file's first case
 * title ("the only theme @vaam-apps/ui actually compiles"), had been wrong
 * since `@vaam-apps/ui@0.1.2` (2026-09-12) — the same release that made
 * `styling-gate.test.ts` stop hardcoding one `--color-base-100`. The package
 * (0.2.4 today) registers **two** themes in `dist/styles/theme.css`: `dark`,
 * with `default: true`, and an opt-in `light`, which the shell's
 * `ThemeSwitcher` writes. A `data-theme="light"` compiles; a `data-theme`
 * naming neither still compiles no styling at all, which is the regression
 * this file guards. The first case now reads the set of registered names off
 * the package's own `theme.css` rather than asserting there is one.
 */
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import RootLayout from "../app/layout";

function markup(): string {
  return renderToStaticMarkup(
    RootLayout({ children: <p>content</p> }) as ReactElement,
  );
}

describe("the root layout", () => {
  it("sets the dark theme — @vaam-apps/ui's default, and one of the two it compiles", () => {
    const themeCss = readFileSync(
      createRequire(import.meta.url).resolve("@vaam-apps/ui/styles/theme.css"),
      "utf8",
    );
    const registered = [
      ...themeCss.matchAll(
        /@plugin\s+"daisyui\/theme"\s*\{[^}]*?name:\s*"([^"]+)"/g,
      ),
    ].map((m) => m[1]);
    expect(registered.sort()).toEqual(["dark", "light"]);

    const html = markup();
    expect(html).toContain('data-theme="dark"');
    const pinned = [...html.matchAll(/data-theme="([^"]*)"/g)].map((m) => m[1]);
    expect(pinned).toEqual(["dark"]);
    for (const name of pinned) {
      expect(registered).toContain(name);
    }
  });

  it("renders no <h1> of its own — the page owns the document's one", () => {
    // `CenteredShell` renders it (`sr-only`), on every page behind it.
    expect(markup()).not.toMatch(/<h1/);
  });

  it("carries the brand in <title> rather than in every route's markup", () => {
    expect(markup()).not.toContain("vpay dashboard");
  });

  it("renders no signed-in control — /login shares this layout", () => {
    // "Sign out" in chrome shared with a page where nobody is signed in is
    // the same kind of claim as a nav entry for a page nobody wrote.
    expect(markup()).not.toMatch(/sign out/i);
  });

  it("runs the theme script as the body's first child, and puts nothing in <head>", () => {
    // In <head> it was the one node React expected there, so any node put in
    // front of it failed hydration (React #418). Cypress puts a whitespace
    // text node there in the documents it drives: on 2026-10-21 that failed 17
    // of dashboard.cy.ts's 18 tests. First in <body> it still runs before
    // anything paints. See app/layout.tsx.
    const html = markup();
    // React writes the <head> element itself; the layout puts nothing in it.
    expect(html).toContain("<head></head>");
    expect(html).toMatch(
      /<body[^>]*><script>\(function\(\)\{try\{var k="vaam-ui:theme"/,
    );
  });
});

const APP_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", "app");

/** Every `page.*` under `app/`, as a path relative to it. */
function pagesUnder(dir: string, prefix = ""): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    if (entry.isDirectory()) {
      return pagesUnder(join(dir, entry.name), `${prefix}${entry.name}/`);
    }
    return /^page\.(tsx|ts|jsx|js)$/.test(entry.name)
      ? [`${prefix}${entry.name}`]
      : [];
  });
}

/**
 * The pages this app has (VPA-5a), as a list a change must edit.
 *
 * Until VP-6a this was `["page.tsx"]` and the first page after it came with
 * VP-6b; VPA-5a brings back the sign-in — `/login`, its code step and its
 * password step — and a landing page at `/`. A page added beyond these is a
 * page that reads vpay with nothing to read it with, or a second sign-in; the
 * pages that read `/dash/v1` arrive with their own ticket, which changes this
 * list in the same commit, and that is what makes the change visible.
 *
 * The negative control below is the same function over a directory that does
 * have pages, so an always-empty `pagesUnder` cannot pass the real assertion.
 */
describe("the app has the pages the sign-in needs, and no others", () => {
  it("serves the account page and the three sign-in steps", () => {
    expect(pagesUnder(APP_DIR).sort()).toEqual([
      "login/page.tsx",
      "login/password/page.tsx",
      "login/totp/page.tsx",
      "page.tsx",
    ]);
  });

  it("the helper it relies on is not vacuously true", () => {
    // A negative control: a tree with a nested page and a stray one is found.
    const root = mkdtempSync(join(tmpdir(), "dash-pages-"));
    try {
      mkdirSync(join(root, "payments", "[id]"), { recursive: true });
      writeFileSync(join(root, "page.tsx"), "");
      writeFileSync(join(root, "payments", "[id]", "page.tsx"), "");
      writeFileSync(join(root, "payments", "payments-list.tsx"), "");
      expect(pagesUnder(root).sort()).toEqual([
        "page.tsx",
        "payments/[id]/page.tsx",
      ]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
