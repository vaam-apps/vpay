/**
 * Asserts that the browser a11y suite is still a gate, and — the part that
 * matters most — that the stories it renders are actually STYLED.
 *
 * Modelled on `frontends/apps/checkout/src/a11y-gate.test.ts`; see that
 * file's own doc comment for why this exists at all: `pnpm --filter
 * @vpay/dashboard test-storybook` needs a Chromium binary, so it runs in
 * CI's `web` job and not in `just ci`. Nothing anyone runs locally would
 * notice it being switched off, or worse, still running while measuring the
 * wrong thing. This file is a plain jsdom test in the suite `just test-web`
 * — and therefore `just ci` — does run.
 *
 * This app's config differs from the checkout's in two ways this file
 * checks for instead of copying blindly:
 *
 * 1. **No hand-written `@vaam-apps/ui/styles/theme.css` alias.** Measured
 *    (`.storybook/main.ts`'s own doc comment carries the numbers): with no
 *    alias at all, this app's built stylesheet still defines
 *    `--color-base-100` — unlike the checkout's build, which drops it to
 *    zero without one. So case 5 below asserts the theme survives the BUILD
 *    without asserting a particular alias is the reason.
 * 2. **A `next/navigation` alias that does not exist in the checkout's
 *    config at all**, because the checkout stories never render a component
 *    that calls a Next router hook. Case 3 asserts it stays wired, because
 *    without it `PaymentsFilters` and `AppShell` throw the moment they
 *    render and axe never reaches them.
 */
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";

import { describe, expect, it } from "vitest";

function appRoot(): string {
  let dir = process.cwd();
  while (!existsSync(join(dir, "next.config.ts"))) {
    const parent = dirname(dir);
    if (parent === dir) throw new Error("no next.config.ts above cwd");
    dir = parent;
  }
  return dir;
}

const APP = appRoot();
const code = (s: string) =>
  s.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, "");

describe("the browser a11y suite is still a gate", () => {
  it("preview.ts still asks the addon to FAIL on a violation", () => {
    const preview = code(
      readFileSync(join(APP, ".storybook/preview.ts"), "utf8"),
    );
    expect(preview).toMatch(/test:\s*"error"/);
    expect(preview).not.toMatch(/test:\s*"(off|todo)"/);
  });

  it("preview.ts still paints the document shell the real page paints", () => {
    // `bg-base-100` is what PAINTS the background; the theme only defines
    // the variable. Without this the stories render on the browser default
    // and every contrast verdict is about a ground the payer never sees.
    const preview = code(
      readFileSync(join(APP, ".storybook/preview.ts"), "utf8"),
    );
    expect(preview).toContain("bg-base-100");
    expect(preview).toContain("data-theme");
    expect(preview).toContain('import "../app/globals.css"');
  });

  it("main.ts still loads the addons that run and check the stories, and still aliases next/navigation", () => {
    const main = code(readFileSync(join(APP, ".storybook/main.ts"), "utf8"));
    expect(main).toContain("@storybook/addon-a11y");
    expect(main).toContain("@storybook/addon-vitest");
    // Without this alias, `PaymentsFilters` (`useRouter`) and `AppShell`
    // (`usePathname`) throw "invariant expected app router to be mounted"
    // the instant they render outside a mounted Next app router — measured
    // on this exact config with the entry removed.
    expect(main).toContain('"next/navigation"');
    expect(main).toContain("next-navigation-mock");
  });

  it("globals.css imports the theme BEFORE any other at-rule", () => {
    // The whole defect PR #135 found, as a position check. An `@import`
    // after `@plugin` is dropped by a spec-compliant parser and the theme
    // vanishes in silence — this app's `globals.css` already carries the
    // fix; this case is what would catch a regression of it.
    const css = code(readFileSync(join(APP, "app/globals.css"), "utf8"));
    const themeImport = css.indexOf('@import "@vaam-apps/ui/styles/theme.css"');
    const plugin = css.indexOf("@plugin");
    expect(themeImport, "the theme import is present").toBeGreaterThan(-1);
    expect(plugin, "@plugin is present").toBeGreaterThan(-1);
    expect(
      themeImport,
      "@import after @plugin is dropped by a strict CSS parser — the theme disappears with no error",
    ).toBeLessThan(plugin);
  });

  it("the BUILT storybook stylesheet actually defines the theme", () => {
    // The one check that would have caught the checkout's six passing-but
    // -unstyled runs, reproduced here even though THIS app's build does not
    // currently need a hand-written alias to pass it (see this file's own
    // module doc and `.storybook/main.ts`'s doc comment for what was
    // measured). Skipped rather than failed when there is no build: `just
    // ci` does not build Storybook, and a test that demands one would make
    // the standard gate depend on a step that is deliberately not in it.
    const dir = join(APP, "storybook-static/assets");
    if (!existsSync(dir)) {
      return;
    }
    const sheets = readdirSync(dir).filter((f) => f.endsWith(".css"));
    expect(sheets.length, "the build emitted a stylesheet").toBeGreaterThan(0);
    const css = sheets.map((f) => readFileSync(join(dir, f), "utf8")).join("");
    expect(
      css,
      "`bg-base-100` is generated but `--color-base-100` is defined nowhere — the theme import was dropped",
    ).toMatch(/--color-base-100:\s*#/);
  });
});
