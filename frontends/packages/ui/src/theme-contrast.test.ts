/**
 * Every tone this product paints text on, re-measured from the compiled
 * stylesheet.
 *
 * Plan §7 row 6 asks for a contrast gate and says why jsdom cannot be it:
 * "jsdom computes no styles from the built stylesheet and axe's contrast rule
 * silently reports nothing there — a green jsdom contrast run is not
 * evidence". So this does not render anything. It compiles `styles.css` with
 * the real `@tailwindcss/postcss` and the real daisyUI, reads the theme block
 * daisyUI emits, converts each `oklch()` the way a browser does, and computes
 * the WCAG ratio. The numbers are daisyUI's, not this repository's — bump
 * daisyUI and this test re-measures rather than restating.
 *
 * It exists because bumblebee's own `--color-error-content` on
 * `--color-error` is 3.53:1, and the screen it is on is the one that tells a
 * payer their money did not move. See the comment at the head of
 * `styles.css`. That was found by looking at a screenshot; a number in a test
 * is how it stays found.
 *
 * Not covered here, and not claimed: whether a rendered glyph actually sits
 * on the colour this file says it does. That is a real browser's job
 * (`cypress-axe`, plan §7 row 6, still unbuilt) — but a tone pair that fails
 * HERE cannot pass there, so this is a floor rather than a proxy.
 */
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import postcss from "postcss";
import tailwind from "@tailwindcss/postcss";
import { describe, expect, it } from "vitest";

import { contrastRatio, themeColours } from "./testing/contrast";

const STYLES = join(dirname(fileURLToPath(import.meta.url)), "styles.css");

/**
 * The tones this product can actually put text on, and only those.
 *
 * daisyUI paints every one of these as "this colour, with its own foreground
 * on top": `.alert-error` is `color: var(--color-error-content)` over
 * `background-color: var(--color-error)`, and `.btn-primary`, `.badge-*` and
 * `.checkbox-*` follow the same rule. The list is derived from what the
 * components' own `cva` maps expose — `Alert`'s `tone`, `Badge`'s `tone`,
 * `Button`'s `variant`, `Checkbox`'s `tone` — union `@vpay/tokens`'
 * `statusTone` and `checkoutOutcomeTone`, which are the only things that
 * choose a tone at runtime.
 *
 * `secondary` and `accent` are deliberately NOT here, and not silently: no
 * component exposes either, so nothing this repository renders can land on
 * them. Measured anyway, for whoever adds one — bumblebee's `secondary` pair
 * is **4.09:1**, below AA, and would need the same correction `error` got.
 * The case below fails if a component starts offering one.
 */
const TONES = ["primary", "neutral", "info", "success", "warning", "error"];

/** Tones bumblebee ships that nothing renders — see the note above. */
const UNRENDERED_TONES = ["secondary", "accent"];

const compiled = await postcss([tailwind()]).process(
  readFileSync(STYLES, "utf8"),
  {
    from: STYLES,
  },
);
const colours = themeColours(compiled.root);

describe("the shipped theme, measured rather than trusted", () => {
  it("emits a colour and a foreground for every tone this product renders", () => {
    for (const tone of TONES) {
      expect(colours.get(tone), `--color-${tone}`).toBeDefined();
      expect(
        colours.get(`${tone}-content`),
        `--color-${tone}-content`,
      ).toBeDefined();
    }
  });

  for (const tone of TONES) {
    it(`clears WCAG AA (4.5:1) for --color-${tone}-content on --color-${tone}`, () => {
      const fill = colours.get(tone);
      const ink = colours.get(`${tone}-content`);
      expect(fill && ink).toBeTruthy();
      const ratio = contrastRatio(
        fill as [number, number, number],
        ink as [number, number, number],
      );
      expect(ratio, `${tone}: ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(
        4.5,
      );
    });
  }

  it("does not let a component start using a tone nobody has measured", () => {
    // The list above is only honest while it is complete. `cva` maps are the
    // one place a tone becomes reachable, so this reads them.
    const dir = join(dirname(fileURLToPath(import.meta.url)), "components");
    // Recursive since 2026-09-11: every component is a FOLDER now, and its
    // `cva` map lives in a sibling `*.variants.ts` rather than in the `.tsx`
    // — a non-recursive `readdirSync` of `components/` returns only directory
    // names, so this case read an empty string and passed vacuously.
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
      .join("\n");
    expect(sources.length, "the component sources were found").toBeGreaterThan(
      1000,
    );
    for (const tone of UNRENDERED_TONES) {
      expect(
        sources,
        `a component now offers "${tone}" — measure it above`,
      ).not.toMatch(
        new RegExp(
          `\\b(alert|badge|btn|checkbox|radio|select|input)-${tone}\\b`,
        ),
      );
    }
  });

  it("keeps the override that makes error and info readable, rather than daisyUI 5 bumblebee’s own", () => {
    // A regression guard with a name, not just a threshold: daisyUI 5's own
    // bumblebee ships 3.53:1 for error and 4.27:1 for info. If this ever
    // reports daisyUI's numbers again, the override at the head of
    // styles.css has been dropped or out-cascaded.
    const error = contrastRatio(
      colours.get("error") as [number, number, number],
      colours.get("error-content") as [number, number, number],
    );
    const info = contrastRatio(
      colours.get("info") as [number, number, number],
      colours.get("info-content") as [number, number, number],
    );
    expect(
      error,
      `error ${error.toFixed(2)}:1 — daisyUI 5 bumblebee's own is 3.53`,
    ).toBeGreaterThan(4.5);
    expect(
      info,
      `info ${info.toFixed(2)}:1 — daisyUI 5 bumblebee's own is 4.27`,
    ).toBeGreaterThan(4.5);
  });
});
