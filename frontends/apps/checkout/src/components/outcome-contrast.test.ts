/**
 * The colour pairs the checkout outcome screens actually render, measured
 * from the compiled theme rather than walked in a browser — issue #73's
 * other half, once `frontends/tests/e2e`'s `shop-hosted.cy.ts` proved
 * axe-core cannot walk THIS page's DOM for a background colour at all: see
 * that file's header comment — daisyUI's own `:root` scroll-lock CSS
 * carries an unconditional `background-image`, and axe-core's
 * `color-contrast` rule gives up the moment any ancestor has one, which
 * `:root` always is. This file answers the question that left open the way
 * the deleted `@vpay/ui`'s `theme-contrast.test.ts` used to for the old
 * theme's raw palette: compute the WCAG ratio from the compiled `--color-*`
 * / `--state-*` values directly. Nothing here renders a component, walks a
 * DOM, or asks a browser — there is no `background-image` for this method
 * to trip over.
 *
 * **Rewritten, not just re-pointed, for the `@vaam-apps/ui` cutover
 * (2026-09-12).** Two things changed about what `OutcomePanel`
 * (`screens.tsx`) actually paints, both measured against
 * `<SP>/vaam-ui/package/dist/styles/theme.css` and `dist/components/
 * patterns/inline-banner.js` directly, not assumed:
 *
 * 1. The outcome text is an `InlineBanner`, not `@vpay/ui`'s `Alert`.
 *    `InlineBanner` does not paint a `--color-<tone>` / `--color-<tone>-
 *    content` pair — it paints `--state-<hue>-fg` on `--state-<hue>-bg`,
 *    and `--state-<hue>-bg` is `rgb(r g b / 0.1)`: an ALPHA colour over
 *    whatever surface sits behind it, not an opaque daisyUI semantic
 *    colour. Measuring the old pair after this cutover would measure a
 *    pair the screen no longer paints — a test that looks right while
 *    checking the wrong thing, which `src/testing/contrast.ts`'s own
 *    header explains in more depth.
 * 2. `@vpay/tokens`' `checkoutOutcomeTone` (`"error"|"warning"|"success"`)
 *    is not `InlineBanner`'s vocabulary. `checkoutOutcomeVariant`
 *    (`"danger"|"warning"|"success"`) is — added alongside it specifically
 *    so this measurement, and `screens.tsx`, have a name that already
 *    agrees with the component library, with a test in `tokens/src/
 *    index.test.ts` asserting the two tables cannot drift apart on which
 *    outcome is alarming.
 *
 * The forward button is unaffected: `Button`'s default `variant="primary"`
 * still compiles to daisyUI's `.btn-primary`, still an opaque
 * `background-color: var(--color-primary)` / `color:
 * var(--color-primary-content)` pair with no alpha and no compositing —
 * confirmed in `dist/components/primitives/button.js` directly.
 *
 * `succeeded`/`failed` are what `shop-hosted.cy.ts` drives in a real
 * browser (and could not get a verdict from). `canceled` is measured here
 * and nowhere else: no spec, in this file or `backends/tests`, ever
 * cancels a `PaymentIntent` out from under an open checkout, so a browser
 * has never rendered that screen at all — this is its only contrast
 * evidence.
 */
import { readFileSync } from "node:fs";

import { checkoutOutcomeVariant } from "@vpay/tokens";
import tailwind from "@tailwindcss/postcss";
import postcss from "postcss";
import { describe, expect, it } from "vitest";

import { contrastRatio, hexToSrgb255, themeColours } from "../testing/contrast";

// A relative path to this app's OWN compiled stylesheet, not a package
// resolution: after the cutover there is no `@vaam-apps/ui/styles.css` to
// resolve (the package publishes only `./styles/theme.css`, meant to be
// `@import`ed from an app's own entry, per its own README) and no
// `import.meta.resolve` workaround is needed either, since this is not a
// package specifier.
const GLOBALS_CSS = new URL("../../app/globals.css", import.meta.url);

const compiled = await postcss([tailwind()]).process(
  readFileSync(GLOBALS_CSS, "utf8"),
  { from: GLOBALS_CSS.pathname },
);

/**
 * `--color-base-100`, read from `dist/styles/theme.css` directly rather
 * than derived: it is the one colour every `--state-*-bg` token's alpha
 * composites against on this page, and it is never overridden by
 * `branding.yaml` (only `--color-primary`/`-content` are, at runtime, by
 * `src/config/theme.ts` — a `<style>` this static compile never sees).
 *
 * **Honest limit, stated once here rather than left implicit**: this
 * assumes an outcome's `InlineBanner` sits directly on the page background.
 * If a banner is ever placed inside a `Card` (`bg-surface-1` =
 * `--color-base-200`) this backdrop is the wrong one, and every ratio below
 * would need recomputing against `--color-base-200` instead.
 */
const PAGE_BACKGROUND_255 = hexToSrgb255("#0a0b0d") as [number, number, number];

const colours = themeColours(compiled.root, { over: PAGE_BACKGROUND_255 });

/** The button every outcome screen with a destination renders — see the header comment. */
const FORWARD_BUTTON_HUE = "primary";

describe("checkout outcome screens, contrast measured from the compiled theme (issue #73)", () => {
  // (0) THE ANTI-VACUITY CASE. `themeColours` silently skips any
  // declaration whose value it cannot parse — that is how a themeColours
  // still shaped for the OLD `oklch(...)`-only theme would report a clean
  // run over `@vaam-apps/ui`'s hex-and-rgb() theme having resolved ZERO
  // colours. A count, before any ratio, on the real compiled sheet (over
  // 600 custom-property declarations measured directly with
  // `postcss([tailwind()]).process` on this app's own `globals.css`).
  it("parses a non-zero number of colours out of the compiled sheet", () => {
    expect(
      colours.size,
      "themeColours resolved nothing — the parser and the theme disagree",
    ).toBeGreaterThan(20);
  });

  // (1) Every token the screens actually name resolves. Unchanged in
  // spirit from the version this replaces; changed in vocabulary.
  it("resolves a colour for every outcome kind's hue, and for the forward button's", () => {
    for (const hue of [
      ...Object.values(checkoutOutcomeVariant),
      FORWARD_BUTTON_HUE,
    ]) {
      const fgKey = hue === FORWARD_BUTTON_HUE ? hue : `state-${hue}-fg`;
      const bgKey =
        hue === FORWARD_BUTTON_HUE ? `${hue}-content` : `state-${hue}-bg`;
      expect(colours.get(fgKey), `--${fgKey}`).toBeDefined();
      expect(colours.get(bgKey), `--${bgKey}`).toBeDefined();
    }
  });

  // (2) A KNOWN NUMBER, so a parser that returns plausible garbage is
  // caught. #e8eaed on #0a0b0d, computed independently
  // (`<SP>/dashui-D-contrast-calc.mjs`) from the two hex values
  // `dist/styles/theme.css` declares for `--color-primary` /
  // `--color-primary-content` — not from this file's own machinery.
  it("agrees with a hand-computed ratio on the forward button's pair", () => {
    const primary = colours.get("primary");
    const primaryContent = colours.get("primary-content");
    expect(primary && primaryContent).toBeTruthy();
    const ratio = contrastRatio(
      primary as [number, number, number],
      primaryContent as [number, number, number],
    );
    expect(ratio).toBeCloseTo(16.34, 1);
  });

  // (3) The pairs InlineBanner actually paints: --state-<hue>-fg over
  // --state-<hue>-bg COMPOSITED over --color-base-100, because the bg is
  // rgb(… / 0.1) and (per the honest limit above) nothing but the page
  // background is assumed behind it.
  //
  // **Second honest limit**: `rgb(… / 0.28)` borders are not measured here
  // at all (`themeColours` skips every `-border` key outright). WCAG's
  // 3:1 non-text-contrast rule applies to them, and a border sits over an
  // already-composited fill — a second compositing problem this file
  // declines to approximate rather than presenting a guess as a
  // measurement.
  for (const [kind, hue] of Object.entries(checkoutOutcomeVariant)) {
    it(`${kind} outcome: the InlineBanner clears WCAG AA (4.5:1) as actually rendered`, () => {
      const fg = colours.get(`state-${hue}-fg`);
      const bg = colours.get(`state-${hue}-bg`);
      expect(
        fg && bg,
        `--state-${hue}-fg and --state-${hue}-bg (composited over --color-base-100)`,
      ).toBeTruthy();
      const ratio = contrastRatio(
        fg as [number, number, number],
        bg as [number, number, number],
      );
      expect(
        ratio,
        `${kind} outcome (hue "${hue}"): ${ratio.toFixed(2)}:1`,
      ).toBeGreaterThanOrEqual(4.5);
    });
  }

  it('the "Back to {merchant}" forward button clears WCAG AA (4.5:1) as actually rendered', () => {
    const fill = colours.get(FORWARD_BUTTON_HUE);
    const ink = colours.get(`${FORWARD_BUTTON_HUE}-content`);
    expect(fill && ink).toBeTruthy();
    const ratio = contrastRatio(
      fill as [number, number, number],
      ink as [number, number, number],
    );
    expect(
      ratio,
      `forward button (hue "${FORWARD_BUTTON_HUE}"): ${ratio.toFixed(2)}:1`,
    ).toBeGreaterThanOrEqual(4.5);
  });
});
