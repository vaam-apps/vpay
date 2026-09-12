/**
 * Ink on a surface — the pair direction `theme-contrast.test.ts` does not
 * measure, and the one that was wrong.
 *
 * That file measures `--color-<tone>-content` on `--color-<tone>`: the pair
 * daisyUI's filled components paint, an alert's text on an alert's
 * background. `text-error` is not that pair. It puts the FILL colour on
 * `--color-base-100`, and bumblebee's error fill is **2.92:1** there. Three
 * `@vpay/ui` stories and the checkout's MSISDN rejection message — the
 * `role="alert"` line telling a payer their number was refused — were below
 * WCAG AA for exactly that reason, with every gate in this repository
 * green. `just test-storybook`, running axe in a real browser, is what
 * found them on 2026-09-12; this file is what keeps them found without
 * needing a browser.
 *
 * It is a separate file because `just verify-ui` caps a file in this
 * package at 200 lines and `theme-contrast.test.ts` is already 181.
 */
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import postcss from "postcss";
import tailwind from "@tailwindcss/postcss";
import { describe, expect, it } from "vitest";

import { contrastRatio, themeColours } from "./testing/contrast";

const STYLES = join(dirname(fileURLToPath(import.meta.url)), "styles.css");

const compiled = await postcss([tailwind()]).process(
  readFileSync(STYLES, "utf8"),
  { from: STYLES },
);
const colours = themeColours(compiled.root);

/** The inks `styles.css` defines, and every surface a component paints them on. */
const INKS = ["error-ink", "muted-ink"];
/** `base-200` as well as `base-100`: `Card` paints one, and both inks are used inside cards. */
const SURFACES = ["base-100", "base-200"];

describe("an ink on a surface, measured rather than trusted", () => {
  for (const ink of INKS) {
    for (const surface of SURFACES) {
      it(`clears WCAG AA (4.5:1) for --color-${ink} on --color-${surface}`, () => {
        const fill = colours.get(surface);
        const text = colours.get(ink);
        expect(text, `--color-${ink} is defined`).toBeDefined();
        expect(fill, `--color-${surface} is defined`).toBeDefined();
        const ratio = contrastRatio(
          fill as [number, number, number],
          text as [number, number, number],
        );
        expect(
          ratio,
          `${ink} on ${surface}: ${ratio.toFixed(2)}:1`,
        ).toBeGreaterThanOrEqual(4.5);
      });
    }
  }

  it("still measures daisyUI's error FILL as unreadable on white, so the reason for these tokens stays visible", () => {
    // Not a threshold anybody has to trust: if a daisyUI bump ever makes
    // `--color-error` legible as text, this fails and someone gets to
    // delete `--color-error-ink` rather than carry it out of habit.
    const ratio = contrastRatio(
      colours.get("base-100") as [number, number, number],
      colours.get("error") as [number, number, number],
    );
    expect(
      ratio,
      `--color-error on base-100 is ${ratio.toFixed(2)}:1 — if this is now >= 4.5, --color-error-ink is obsolete`,
    ).toBeLessThan(4.5);
  });

  it("does not let a component paint daisyUI's error FILL as text, or spell quiet as an opacity", () => {
    const dir = join(dirname(fileURLToPath(import.meta.url)), "components");
    const sources = readdirSync(dir, { recursive: true })
      .map((entry) => entry.toString())
      .filter(
        (f) =>
          (f.endsWith(".tsx") || f.endsWith(".ts")) &&
          !f.endsWith("index.ts") &&
          !f.includes(".test.") &&
          !f.includes(".stories."),
      )
      .map((f) => readFileSync(join(dir, f), "utf8"))
      // Strip comments: several of these files explain the defect in prose,
      // and prose is not a class name. Same distinction `verify-status`
      // draws by lexing.
      .map((src) =>
        src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, ""),
      )
      .join("\n");
    expect(sources.length, "the component sources were found").toBeGreaterThan(
      1000,
    );
    // Class TOKENS: a substring test also matches `text-error-ink`.
    const classes = new Set(
      (sources.match(/[A-Za-z0-9:/[\]._-]+/g) ?? []).map((t) => t),
    );
    expect(
      classes.has("text-error"),
      "`text-error` is daisyUI's fill on a light surface (2.92:1) — use `text-error-ink`",
    ).toBe(false);
    expect(
      sources,
      "an opacity multiplies with an ancestor's (List 0.7 x Text 0.6 = 2.71:1) — use `text-muted-ink`",
    ).not.toMatch(/\bopacity-[0-9]+\b/);
  });
});
