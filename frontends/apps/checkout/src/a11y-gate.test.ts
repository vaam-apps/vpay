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

  /**
   * **Nothing gated a story-level suppression here until 2026-09-13.**
   * `parameters.a11y.test = "todo"` on a single story switches the addon off
   * for that story ENTIRELY — not just the rule that is failing — and the
   * suite still reports it as passing. The dashboard's Storybook shipped
   * exactly that on its two `AppShell` stories, which left the chrome around
   * every one of its screens with no colour-contrast verdict at all; the
   * probe that proved it passed at `#3a3a3a` on `#0a0b0d`.
   *
   * This app has no suppressions, and this pins that at zero: the expected
   * arrays are empty, so the FIRST one added fails `just ci` and has to be
   * argued for in review rather than merged quietly.
   */
  it("no story switches the a11y addon off, and nothing disables an axe rule", () => {
    const dir = join(APP, "src/components");
    const files = readdirSync(dir).filter((f) => f.endsWith(".stories.tsx"));
    expect(files.length, "there are story files to check").toBeGreaterThan(0);

    for (const file of files) {
      const src = code(readFileSync(join(dir, file), "utf8"));
      expect(
        src,
        `${file}: a story-level test:"off"/"todo" turns axe off for that story entirely`,
      ).not.toMatch(/test:\s*"(off|todo)"/);

      const disabled = [
        ...src.matchAll(/id:\s*"([^"]+)"\s*,\s*enabled:\s*false/g),
      ].map((m) => m[1]);
      expect(
        disabled,
        `${file}: axe rules disabled by a story — this app pins the set at none`,
      ).toEqual([]);
    }
  });

  it("the BUILT storybook stylesheet actually defines the theme", (ctx) => {
    // The one check that would have caught six passing-but-unstyled runs.
    //
    // **`ctx.skip()`, NOT a bare `return` — corrected 2026-09-13.** A bare
    // `return` reported this case as PASSED with nothing measured, and CI is
    // exactly where that happens: the `web` job runs `pnpm -r test` BEFORE
    // `just build-storybook`, so `storybook-static/` never exists when this
    // file runs there. The one check standing behind this app's whole
    // styled-or-not claim was therefore green-and-blind on every CI run —
    // this repository's named failure mode, inside the test written to
    // prevent it. Found in the dashboard's copy of this file and fixed in
    // both. Skipping says so in the count; `just build-storybook` now
    // carries the assertion that runs against a real artefact, for both
    // apps (see the recipe's own comment).
    const dir = join(APP, "storybook-static/assets");
    if (!existsSync(dir)) {
      ctx.skip();
      return;
    }
    const sheets = readdirSync(dir).filter((f) => f.endsWith(".css"));
    expect(sheets.length, "the build emitted a stylesheet").toBeGreaterThan(0);
    const css = sheets.map((f) => readFileSync(join(dir, f), "utf8")).join("");
    expect(
      css,
      "`bg-base-100` is generated but `--color-base-100` is defined nowhere. " +
        "Either `app/globals.css` dropped the theme `@import`, or " +
        "`storybook-static/` is STALE — a `just build-storybook` that failed " +
        "this same check leaves its theme-less artefact on disk on purpose, " +
        "so it can be inspected, and every later run of this test then reads " +
        "that instead of the source. Re-run `just build-storybook`: if it " +
        "exits 0, the tree was fine and the artefact was stale.",
    ).toMatch(/--color-base-100:\s*#/);
  });
});
