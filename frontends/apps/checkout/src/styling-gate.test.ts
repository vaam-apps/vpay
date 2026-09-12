/**
 * The gate `just ci` cannot provide, because it never builds this app.
 *
 * The dashboard carries the twin of this file
 * (`frontends/apps/dashboard/src/styling-gate.test.ts`) and its header has
 * the full derivation; this is the checkout's own copy because the two
 * apps' PostCSS pipelines are separate processes over separate
 * `app/globals.css` entries and share nothing (see that file's own
 * comment). Both load the same two load-bearing lines from
 * `@vaam-apps/ui`'s README — `themes: false` and the `@source` path — and
 * both fail SILENTLY if either is dropped: no error, no warning, just a
 * page that looks broken. Neither `next build` succeeding nor a passing
 * jsdom render would show either failure; only the compiled CSS does.
 *
 * This app's own runtime brand-colour retargeting
 * (`src/config/theme.ts`, decision 3, 2026-09-12) sets one custom property
 * at runtime and is unaffected by, and does not affect, either fact checked
 * here — it retints on top of whatever theme this pipeline compiles, and
 * `theme.ts`'s own suite is what proves *that* layer holds.
 *
 * Measured 2026-09-12
 * (`<SCRATCHPAD>/dashui-measured-facts.md` §5): Tailwind v4's `@source` scan
 * runs under a plain `postcss([tailwind()]).process(css, { from, to })`
 * call in Node, so this can be a vitest test rather than a manual,
 * unrepeatable check.
 *
 * # What each case proves, and the mutation that proves the case is real
 *
 * (a) `@source "../node_modules/@vaam-apps/ui/dist"` is what makes Tailwind
 *     look inside `node_modules` at all. This asserts a utility that only
 *     the package's compiled `dist` writes and that this app's own `src`/
 *     `app` never do — see `CANDIDATE_CLASS` below, and read its comment
 *     before touching this file: it is deliberately not one literal string.
 *
 *     Mutation run by hand and reverted, 2026-09-12: deleting the `@source`
 *     line from `app/globals.css` and recompiling took the count from 2 to
 *     **0**, and the file size from 201 041 bytes to 97 446. This case
 *     fails on that mutation — re-verified against this exact test file.
 *
 * (b) `themes: false` stops daisyUI's own built-in `dark` theme
 *     (`oklch(25.33% 0.016 252.42)`) from out-specificity-ing
 *     `@vaam-apps/ui`'s theme, which registers under the same name. This
 *     asserts `--color-base-100` resolves to the package's `#0a0b0d` and to
 *     **nothing else** in the whole compiled sheet.
 *
 *     Mutation run by hand and reverted, 2026-09-12: removing `themes: false`
 *     and recompiling produced THREE values for `--color-base-100`:
 *     `#0a0b0d`, `oklch(100% 0 0)` and `oklch(25.33% 0.016 252.42)` — the
 *     last is daisyUI's stock dark, at higher specificity, exactly as the
 *     README warns. This case fails on that mutation.
 *
 * Both cases assert a non-zero match count before asserting what the
 * matches are — the anti-vacuity discipline `dashui-measured-facts.md` §4
 * names: a contrast helper ported unchanged parsed zero colours and still
 * passed, because nothing had asserted the count first.
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
 * A colour utility `@vaam-apps/ui`'s own components write and this app
 * never does (verified — `git grep` over `src`/`app` finds nothing).
 *
 * **Built from parts at runtime, deliberately — do not inline it as one
 * literal string.** Tailwind v4's content scanner extracts class-shaped
 * tokens from any text file it reaches (including a `.ts` file's own
 * comments) and treats a match as usage, with no notion of "a real render"
 * versus "a string in a comment". Measured directly, 2026-09-12: with the
 * `@source` line removed from `app/globals.css`, a **sibling** test file
 * that merely mentioned this class name in a comment was enough on its own
 * to make Tailwind regenerate the utility — the case would then have passed
 * whether or not the gate it claims to check actually holds. Splitting the
 * string here is what was verified, by hand, to make case (a) fail
 * correctly on the `@source` mutation.
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
