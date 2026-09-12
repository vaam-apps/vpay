/**
 * `branding.yaml`'s `primary_color` → a daisyUI theme override.
 *
 * daisyUI 5 compiles a theme into CSS custom properties that are **standard
 * CSS colour values** — `--color-primary: oklch(49.12% 0.309 275.75)` —
 * where daisyUI 4 used unwrapped OKLCh components (`--p: 49.12% .3096
 * 275.75`). That is the whole reason this module used to be 176 lines: it
 * converted an operator's `#rrggbb` into OKLCh by hand, with Björn
 * Ottosson's published matrices, because daisyUI 4's variable would accept
 * nothing else.
 *
 * `--color-primary` takes any CSS colour, so `#rrggbb` now needs no
 * conversion at all — it is emitted as written, once re-validated. What
 * daisyUI still does not derive is the *foreground*: the colour painted on
 * top of `--color-primary` (a `btn-primary`'s text). This module keeps that
 * one computation, expressed with the platform's own `color-mix()` rather
 * than with reimplemented OKLCh arithmetic — 80% of the way from the colour
 * to white if it is dark, to black if it is light, which is daisyUI's own
 * rule (`generateForegroundColorFrom`).
 *
 * `hexToLinearRgb`/`isDark` are kept from the old module: `hexToLinearRgb`
 * is the `#rrggbb` validator as much as it is a conversion step, and
 * `isDark` is
 * daisyUI's own "which contrast is bigger" test, not something `color-mix`
 * can decide on its own. The sRGB→OKLCh *output formatting* — `toOklch`,
 * `foreground`, `cut`, `format`, the six-colour comparison against daisyUI's
 * own culori-based converter — is gone: `color-mix` is a browser primitive
 * and its contrast is asserted directly (`theme.test.ts`), which is the
 * property anyone actually cares about.
 *
 * There is no `#rgb` short form and no named colour: `settings.ts` admits
 * `#rrggbb` and nothing else, so this module never has to guess.
 *
 * **Retargeted to `@vaam-apps/ui`'s theme (2026-09-12, decision 3).** The
 * override used to be scoped to daisyUI's own `bumblebee`; that theme no
 * longer compiles (`app/globals.css` now sets `themes: false`, per
 * `dashui-measured-facts.md` §3), so `THEME` is now `"dark"` — the name
 * `@vaam-apps/ui/styles/theme.css` registers its one custom theme under
 * (`@plugin "daisyui/theme" { name: "dark"; … }`). The override string
 * still targets `${THEME}` rather than a literal, so nothing else in this
 * function changed. The AA-contrast maths below is unaffected either way —
 * it is a pure function of the merchant's own hex, never of the page's
 * background — but it clears a HARDER bar now regardless: it computes the
 * contrast between a colour and a foreground mixed 20% toward white/black,
 * not against this theme's own near-black `--color-base-100` (`#0a0b0d`),
 * and `theme.test.ts` still asserts every case at AA (4.5:1), unweakened.
 */

/** sRGB 0–1 → linear-light 0–1. The IEC 61966-2-1 transfer function. */
function linearize(channel: number): number {
  return channel <= 0.04045
    ? channel / 12.92
    : Math.pow((channel + 0.055) / 1.055, 2.4);
}

/**
 * `#rrggbb` → three linear-light channels, or `null` if it is not that.
 *
 * Exported (2026-09-12) so `src/testing/contrast.ts` — which measures the
 * *compiled theme's* colours rather than an operator's — can reuse this
 * exact transfer function instead of carrying a second copy of it.
 * `theme.test.ts` deliberately does NOT import this: it re-implements the
 * same maths independently, so it verifies the rule rather than the
 * function that encodes it.
 */
export function hexToLinearRgb(hex: string): [number, number, number] | null {
  if (!/^#[0-9a-fA-F]{6}$/.test(hex)) {
    return null;
  }
  const channel = (start: number): number =>
    Number.parseInt(hex.slice(start, start + 2), 16) / 255;
  return [linearize(channel(1)), linearize(channel(3)), linearize(channel(5))];
}

/**
 * WCAG relative luminance from linear-light channels.
 *
 * Used only to answer "is this colour dark?", which is daisyUI's own test:
 * it compares the colour's contrast against black with its contrast against
 * white and takes the larger. Comparing the two ratios reduces to comparing
 * `Y` with `sqrt(1.05 * 0.05) - 0.05`, but the ratios are written out
 * because that is the form the rule is stated in.
 */
function isDark([r, g, b]: [number, number, number]): boolean {
  const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  const againstBlack = (luminance + 0.05) / 0.05;
  const againstWhite = 1.05 / (luminance + 0.05);
  return againstBlack < againstWhite;
}

/**
 * The theme every page renders under.
 *
 * **`@vaam-apps/ui`'s one registered theme (2026-09-12), not daisyUI's
 * `bumblebee`.** `app/globals.css` sets `themes: false`, which stops
 * `bumblebee` from compiling at all — a `:root[data-theme="bumblebee"]`
 * selector would then match nothing. `@vaam-apps/ui/styles/theme.css`
 * registers its custom theme under daisyUI's own built-in name `"dark"`
 * (`@plugin "daisyui/theme" { name: "dark"; … }`), confirmed by reading
 * that file directly, so this is the selector that now actually applies.
 */
export const THEME = "dark";

/**
 * The `<style>` body that applies an operator's colour, or `null`.
 *
 * Scoped to `:root[data-theme="dark"]` — the same specificity daisyUI's own
 * theme block has, and later in the document, so it wins without an
 * explicit override rule. Emitted by the server component that renders
 * `<head>`, which is what makes the colour arrive with the first byte
 * rather than after a flash of the default.
 *
 * Nothing in the returned string comes from a payer or from a merchant: the
 * only input is a `#rrggbb` this module re-validated, and every character it
 * emits besides the literal `color-mix(in oklch, … 20%, white|black)` is a
 * digit, a `#`, or punctuation. There is no path by which a mounted file
 * becomes markup.
 */
export function themeStyleSheet(primaryColor: string | null): string | null {
  if (primaryColor === null) {
    return null;
  }
  const rgb = hexToLinearRgb(primaryColor);
  if (rgb === null) {
    return null;
  }
  const towards = isDark(rgb) ? "white" : "black";
  return (
    `:root[data-theme="${THEME}"]{` +
    `--color-primary:${primaryColor};` +
    `--color-primary-content:color-mix(in oklch, ${primaryColor} 20%, ${towards});` +
    `}`
  );
}
