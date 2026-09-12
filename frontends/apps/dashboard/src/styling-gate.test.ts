/**
 * The gate `just ci` cannot provide, because it never builds this app.
 *
 * `app/globals.css`'s own comment names two facts as load-bearing and BOTH
 * fail silently if broken — "no error, no warning, just a page that looks
 * broken" — and says in as many words: "No automated gate in this app
 * re-checks either fact yet." This is that gate.
 *
 * Measured 2026-09-12 (`<SCRATCHPAD>/dashui-measured-facts.md` §5): Tailwind
 * v4's `@source` scan runs under a plain
 * `postcss([tailwind()]).process(css, { from, to })` call in Node — it does
 * not need `next build` — so this can be a vitest test rather than a manual,
 * unrepeatable check. `next build` succeeding would not have caught either
 * failure: a missing `@source` or a wrong theme both compile without error
 * and without a warning; only the compiled CSS itself shows the defect.
 *
 * # What each case proves, and the mutation that proves the case is real
 *
 * (a) `@source "../node_modules/@vaam-apps/ui/dist"` is what makes Tailwind
 *     look inside `node_modules` at all — omitted, "every `@vaam-apps/ui`
 *     component renders unstyled" (the package's own `index.d.ts`). This
 *     asserts a utility that only the package's compiled `dist` writes and
 *     that this app's own `src`/`app` never do — see `CANDIDATE_CLASS`
 *     below, and read its comment before touching this file.
 *
 *     Mutation run by hand and reverted, 2026-09-12: deleting the `@source`
 *     line from `app/globals.css` and recompiling took the count from 2 to
 *     **0**, and the file size from 196 152 bytes to 83 941. This case
 *     fails on that mutation — re-verified against this exact test file.
 *
 * (b) `themes: false` is what stops daisyUI's own built-in `dark` theme
 *     (`oklch(25.33% 0.016 252.42)`) from out-specificity-ing
 *     `@vaam-apps/ui`'s theme, which registers under the same name. This
 *     asserts `--color-base-100` resolves to the package's `#0a0b0d` and to
 *     **nothing else** in the whole compiled sheet.
 *
 *     Mutation run by hand and reverted, 2026-09-12: removing `themes: false`
 *     (`@plugin "daisyui" { themes: false; }` → `@plugin "daisyui";`) and
 *     recompiling produced THREE values for `--color-base-100`:
 *     `#0a0b0d`, `oklch(100% 0 0)` and `oklch(25.33% 0.016 252.42)` — the
 *     last is daisyUI's stock dark, at higher specificity, exactly as the
 *     README warns. This case fails on that mutation.
 *
 * A test that only asserted a ratio or a single occurrence without first
 * counting how many values exist would pass having measured nothing —
 * `<SCRATCHPAD>/dashui-measured-facts.md` §4 records exactly that failure
 * mode for a contrast helper ported unchanged. Both cases below assert a
 * non-zero match count before asserting what the matches are.
 */
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import tailwind from "@tailwindcss/postcss";
import postcss from "postcss";
import { beforeAll, describe, expect, it } from "vitest";

const APP_DIR = join(dirname(fileURLToPath(import.meta.url)), "..");
const CSS_PATH = join(APP_DIR, "app", "globals.css");

/**
 * A colour utility `@vaam-apps/ui`'s own components write (`select.js`,
 * `chip-select.js`, `command-menu.js`, `progress.js`) and this app never
 * does (verified — `git grep` over `src`/`app` finds nothing).
 *
 * **Built from parts at runtime, deliberately, and this is load-bearing —
 * do not inline it as one literal string.** Tailwind v4's own content
 * scanner has no notion of "a real render" versus "a string sitting in a
 * comment": it extracts class-shaped tokens from any text file it reaches
 * and treats a match as usage. Spelling the candidate as one contiguous
 * literal ANYWHERE in this app's source — including in a doc comment, even
 * one explaining that it shouldn't count — would make Tailwind generate the
 * utility from *this file*, independent of whether `@source` ever reached
 * the package. The case would then pass whether or not the gate it claims
 * to be actually holds. Splitting the string is what was verified, by hand,
 * to make case (a) fail on the `@source` mutation above; a first draft that
 * spelled the class literally in this file's own header comment did not
 * fail on that mutation, and was rewritten because of it.
 */
const CANDIDATE_CLASS = ["text", "subtle", "fore" + "ground"].join("-");

let compiled: string;

beforeAll(async () => {
  const source = readFileSync(CSS_PATH, "utf8");
  const result = await postcss([tailwind()]).process(source, {
    from: CSS_PATH,
    to: CSS_PATH,
  });
  compiled = result.css;
}, 30_000);

describe("the compiled globals.css — the styling gate just ci cannot provide", () => {
  it("the @source scan reached @vaam-apps/ui/dist: a utility no app source writes is present", () => {
    const rule = new RegExp(`\\.${CANDIDATE_CLASS}\\s*\\{`, "g");
    const matches = compiled.match(rule) ?? [];
    expect(matches.length).toBeGreaterThan(0);
  });

  it("every --color-base-100 the build emits is one the package itself declares", () => {
    const matches = [...compiled.matchAll(/--color-base-100:\s*([^;]+);/g)].map(
      (m) => m[1]?.trim(),
    );
    expect(matches.length).toBeGreaterThan(0);

    /**
     * **Derived from the package, not hardcoded — and that is the fix, not a
     * relaxation.** This case asserted `new Set(["#0a0b0d"])` until
     * 2026-09-12, when `@vaam-apps/ui@0.1.2` added an opt-in `light` theme
     * (`dark` keeps `default: true`) and the build legitimately began
     * emitting `#fcfcfd` as well. A hardcoded literal cannot tell that
     * apart from the failure this case exists to catch.
     *
     * What it exists to catch is `themes: false` being dropped from
     * `globals.css`, which lets daisyUI's own built-in themes emit their
     * own `--color-base-100` — and daisyUI authors those in `oklch()`,
     * while this package authors in hex. So reading the package's own
     * theme.css for the permitted set is both version-proof and STRICTER
     * than the literal was: a stock daisyUI value fails on the set
     * membership, and the explicit `oklch` assertion below names the
     * failure mode so a future reader does not have to infer it.
     */
    const themeCss = readFileSync(
      createRequire(import.meta.url).resolve("@vaam-apps/ui/styles/theme.css"),
      "utf8",
    );
    const declared = new Set(
      [...themeCss.matchAll(/--color-base-100:\s*([^;]+);/g)].map((m) =>
        m[1]?.trim(),
      ),
    );
    expect(declared.size, "the package declares at least one").toBeGreaterThan(
      0,
    );
    for (const value of matches) {
      expect(
        declared.has(value),
        `${value} is not declared by @vaam-apps/ui — stock daisyUI has leaked in, which means themes:false is gone`,
      ).toBe(true);
    }
    // The shipped default is still the dark one both apps pin with
    // `data-theme`. A release that changed it would be a visual change to
    // every screen and should not pass silently.
    expect(matches, "the dark default is still emitted").toContain("#0a0b0d");
    expect(
      compiled.match(/--color-base-100:\s*oklch\(/g),
      "daisyUI's own themes are still switched off",
    ).toBeNull();
  });
});
