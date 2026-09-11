/**
 * The colour pairs the checkout outcome screens actually render, measured
 * from the compiled theme rather than walked in a browser — issue #73's
 * other half, once `frontends/tests/e2e`'s `shop-hosted.cy.ts` proved
 * axe-core cannot walk THIS page's DOM for a background colour at all: see
 * that file's header comment — daisyUI 5's own `:root` scroll-lock CSS
 * carries an unconditional `background-image`, and axe-core's
 * `color-contrast` rule gives up the moment any ancestor has one, which
 * `:root` always is. Three rounds of mutation there, including an
 * identical, axe-parseable foreground/background pair, never produced a
 * `violation` — only ever `incomplete`. This file answers the question that
 * left open, the way `@vpay/ui`'s `theme-contrast.test.ts` already answers
 * it for the theme's raw palette: compute the WCAG ratio from the compiled
 * `--color-*` values directly. Nothing here renders a component, walks a
 * DOM, or asks a browser — there is no `background-image` for this method
 * to trip over.
 *
 * `OutcomePanel` (`screens.tsx`) renders exactly two things whose colour
 * comes from the theme: an `Alert` toned by `@vpay/tokens`'
 * `checkoutOutcomeTone[kind]`, and — on every outcome with a forward
 * button — a `Button` with no `variant`, `@vpay/ui`'s default, which the
 * compiled stylesheet resolves to `.btn-primary`. Confirmed in daisyUI
 * 5.7.28's own `alert.css`/`button.css`: both compile to a plain `color:
 * var(--color-<tone>-content)` on `background-color: var(--color-<tone>)`
 * — no alpha blend, no `color-mix()`, nothing this measurement doesn't
 * already model — so `@vpay/ui/testing/contrast`'s machinery (the same
 * real Tailwind + real daisyUI PostCSS compile, the same OKLCh→sRGB
 * conversion `theme-contrast.test.ts` uses) measures the EXACT pair each
 * screen paints, not an approximation of it.
 *
 * `succeeded`/`failed` are what `shop-hosted.cy.ts` drives in a real
 * browser (and could not get a verdict from). `canceled` is measured here
 * and nowhere else: no spec, in this file or `backends/tests`, ever
 * cancels a `PaymentIntent` out from under an open checkout, so a browser
 * has never rendered that screen at all — this is its only contrast
 * evidence.
 */
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

import { checkoutOutcomeTone } from "@vpay/tokens";
import { contrastRatio, themeColours } from "@vpay/ui/testing/contrast";
import tailwind from "@tailwindcss/postcss";
import postcss from "postcss";
import { describe, expect, it } from "vitest";

// `import.meta.resolve` is unsupported by vitest's Vite-based module
// runner (measured: "[module runner] import.meta.resolve is not
// supported"); `createRequire` resolves through plain Node module
// resolution instead, which still honours `@vpay/ui`'s package.json
// `exports` map.
const STYLES = createRequire(import.meta.url).resolve("@vpay/ui/styles.css");

const compiled = await postcss([tailwind()]).process(
  readFileSync(STYLES, "utf8"),
  { from: STYLES },
);
const colours = themeColours(compiled.root);

/** The button every outcome screen with a destination renders — see above. */
const FORWARD_BUTTON_TONE = "primary";

describe("checkout outcome screens, contrast measured from the compiled theme (issue #73)", () => {
  it("resolves a colour for every outcome kind's tone, and for the forward button's", () => {
    for (const tone of [
      ...Object.values(checkoutOutcomeTone),
      FORWARD_BUTTON_TONE,
    ]) {
      expect(colours.get(tone), `--color-${tone}`).toBeDefined();
      expect(
        colours.get(`${tone}-content`),
        `--color-${tone}-content`,
      ).toBeDefined();
    }
  });

  for (const [kind, tone] of Object.entries(checkoutOutcomeTone)) {
    it(`${kind} outcome: the Alert clears WCAG AA (4.5:1) as actually rendered`, () => {
      const fill = colours.get(tone);
      const ink = colours.get(`${tone}-content`);
      expect(
        fill && ink,
        `--color-${tone} and --color-${tone}-content`,
      ).toBeTruthy();
      const ratio = contrastRatio(
        fill as [number, number, number],
        ink as [number, number, number],
      );
      expect(
        ratio,
        `${kind} outcome (tone "${tone}"): ${ratio.toFixed(2)}:1`,
      ).toBeGreaterThanOrEqual(4.5);
    });
  }

  it('the "Back to {merchant}" forward button clears WCAG AA (4.5:1) as actually rendered', () => {
    const fill = colours.get(FORWARD_BUTTON_TONE);
    const ink = colours.get(`${FORWARD_BUTTON_TONE}-content`);
    expect(fill && ink).toBeTruthy();
    const ratio = contrastRatio(
      fill as [number, number, number],
      ink as [number, number, number],
    );
    expect(
      ratio,
      `forward button (tone "${FORWARD_BUTTON_TONE}"): ${ratio.toFixed(2)}:1`,
    ).toBeGreaterThanOrEqual(4.5);
  });
});
