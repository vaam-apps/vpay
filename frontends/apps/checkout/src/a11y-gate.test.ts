/**
 * Asserts that the browser a11y suite is still a gate, and — the part that
 * matters most — that the stories it renders are actually STYLED.
 *
 * `pnpm --filter @vpay/checkout test-storybook` needs a Chromium binary, so
 * it runs in CI's `web` job and not in `just ci`. Nothing anyone runs
 * locally would notice it being switched off, or worse, still running while
 * measuring the wrong thing. This is a plain jsdom test in the suite
 * `just test-web` — and therefore `just ci` — does run.
 *
 * **The styling half is not hypothetical.** Building this suite, the stories
 * rendered completely unstyled for six consecutive runs and axe passed all
 * 22 every time, because `app/globals.css` placed
 * `@import "@vaam-apps/ui/styles/theme.css"` AFTER `@plugin "daisyui"` and
 * CSS drops an `@import` that follows another at-rule. Tailwind's own parser
 * is lenient, so `next build` and `styling-gate.test.ts` both inlined the
 * theme and both passed; Storybook's vite build did not, and emitted a
 * stylesheet with every `var(--color-base-100)` present and
 * `--color-base-100` defined nowhere. axe then measured every foreground
 * against the browser's default white while the shipped page is `#0a0b0d`,
 * and a deliberately unreadable `#3a3a3a` probe passed alongside the real
 * stories. A green a11y run against the wrong background is worse than no
 * run: it is a claim nobody will re-check.
 *
 * `styling-gate.test.ts` next door compiles `globals.css` through
 * `@tailwindcss/postcss` directly, which is the lenient parser — so it
 * cannot catch this, and did not. This file reads the **built Storybook's**
 * own stylesheet instead.
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

  it("main.ts still loads the addons that run and check the stories", () => {
    const main = code(readFileSync(join(APP, ".storybook/main.ts"), "utf8"));
    expect(main).toContain("@storybook/addon-a11y");
    expect(main).toContain("@storybook/addon-vitest");
  });

  it("globals.css imports the theme BEFORE any other at-rule", () => {
    // The whole defect, as a position check. An `@import` after `@plugin`
    // is dropped by a spec-compliant parser and the theme vanishes in
    // silence.
    // Comments stripped first: this file's own note explains the defect in
    // prose and names `@plugin`, and the first draft of this case failed on
    // its own documentation. Same distinction `verify-status` draws.
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
    // The one check that would have caught six passing-but-unstyled runs.
    // Skipped rather than failed when there is no build: `just ci` does not
    // build Storybook, and a test that demands one would make the standard
    // gate depend on a step that is deliberately not in it.
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
