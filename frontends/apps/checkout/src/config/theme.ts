/**
 * `branding.yaml`'s `primary_color` → a daisyUI theme override.
 *
 * daisyUI compiles a theme into CSS custom properties whose values are
 * **unwrapped OKLCh components** — `--p: 84.2251% 0.165456 91.330667` — and
 * builds every `btn-primary`, `checkbox-primary` and `text-primary` rule out
 * of them. So an operator's colour does not need a rebuild: one `:root`
 * block redefining `--p` and `--pc` retints the whole page, which is the
 * difference between a brand that is configuration and a brand that is a
 * `tailwind.config.ts` edit and a new image.
 *
 * That means converting sRGB to OKLCh here, in ~40 lines, rather than adding
 * a colour library to a payment page. The arithmetic is Björn Ottosson's
 * published OKLab matrices and daisyUI's own foreground rule, and the tests
 * assert this module's output against values produced by **daisyUI's own
 * converter** (`daisyui/src/theming/functions.js`, which uses culori) for
 * six colours — so a drift from what daisyUI would have compiled is a
 * failing test rather than a page that is subtly the wrong colour.
 *
 * There is no `#rgb` short form and no named colour: `settings.ts` admits
 * `#rrggbb` and nothing else, so this module never has to guess.
 */

/** The daisyUI variables one colour can set. */
export interface PrimaryOverride {
  /** `--p`, the primary colour, as `L% C H`. */
  '--p': string;
  /** `--pc`, what daisyUI paints *on* the primary — text on a `btn-primary`. */
  '--pc': string;
}

interface Oklch {
  /** 0–1, not a percentage. */
  l: number;
  c: number;
  /** Degrees, 0–360. */
  h: number;
}

/** sRGB 0–1 → linear-light 0–1. The IEC 61966-2-1 transfer function. */
function linearize(channel: number): number {
  return channel <= 0.04045 ? channel / 12.92 : Math.pow((channel + 0.055) / 1.055, 2.4);
}

/** `#rrggbb` → three linear-light channels, or `null` if it is not that. */
function linearRgb(hex: string): [number, number, number] | null {
  if (!/^#[0-9a-fA-F]{6}$/.test(hex)) {
    return null;
  }
  const channel = (start: number): number => Number.parseInt(hex.slice(start, start + 2), 16) / 255;
  return [linearize(channel(1)), linearize(channel(3)), linearize(channel(5))];
}

/** Linear-light sRGB → OKLCh. Ottosson's matrices, unchanged. */
function toOklch([r, g, b]: [number, number, number]): Oklch {
  const long = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
  const medium = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
  const short = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;

  const l_ = Math.cbrt(long);
  const m_ = Math.cbrt(medium);
  const s_ = Math.cbrt(short);

  const lightness = 0.2104542553 * l_ + 0.793617785 * m_ - 0.0040720468 * s_;
  const a = 1.9779984951 * l_ - 2.428592205 * m_ + 0.4505937099 * s_;
  const bb = 0.0259040371 * l_ + 0.782771766 * m_ - 0.808675766 * s_;

  const chroma = Math.hypot(a, bb);
  // A grey has no hue. daisyUI reports 0 for one (culori leaves the hue
  // undefined and `cutNumber` turns that into 0), and every consumer
  // multiplies it by a chroma of 0, so the number is arbitrary — but it has
  // to be the *same* arbitrary number, or two spellings of grey would not
  // compare equal.
  //
  // The test is on the ROUNDED chroma, not on `chroma === 0`. `#ffffff`
  // comes out of the matrices with a chroma around 1e-8 rather than exactly
  // zero, which is a real hue of 89.87° in a colour that has none — and
  // that was measured, not assumed: the first version of this line used
  // `chroma === 0` and emitted `100% 0 89.874892` where daisyUI emits
  // `100% 0 0`.
  const hue = cut(chroma) === 0 ? 0 : ((Math.atan2(bb, a) * 180) / Math.PI + 360) % 360;
  return { l: lightness, c: chroma, h: hue };
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
 * daisyUI's `generateForegroundColorFrom`: 80% of the way from the colour to
 * white if it is dark, to black if it is light.
 *
 * Both endpoints are achromatic, so chroma falls to 20% of the original and
 * the hue is carried through unchanged — which is what culori does when one
 * end of an interpolation has no hue, and why a yellow's foreground is a
 * very dark yellow-brown rather than a neutral grey.
 */
function foreground(colour: Oklch, dark: boolean): Oklch {
  const targetLightness = dark ? 1 : 0;
  return {
    l: colour.l + 0.8 * (targetLightness - colour.l),
    c: colour.c * 0.2,
    h: colour.h,
  };
}

/** daisyUI's `cutNumber`: six decimal places, and a falsy value is a hard 0. */
function cut(value: number): number {
  return value ? Number(value.toFixed(6)) : 0;
}

/** daisyUI's `colorObjToString`, byte for byte. */
function format(colour: Oklch): string {
  const lightness = Number.parseFloat((cut(colour.l) * 100).toFixed(6));
  return `${lightness}% ${cut(colour.c)} ${cut(colour.h)}`;
}

/**
 * `#rrggbb` → the two daisyUI variables it implies, or `null` for a value
 * that is not a six-digit hex colour.
 *
 * `null` rather than a fallback colour: a page that silently painted itself
 * a colour nobody configured would be indistinguishable from one whose
 * configuration was read, and the caller already logs the problem.
 */
export function primaryOverride(hex: string): PrimaryOverride | null {
  const rgb = linearRgb(hex);
  if (rgb === null) {
    return null;
  }
  const colour = toOklch(rgb);
  return {
    '--p': format(colour),
    '--pc': format(foreground(colour, isDark(rgb))),
  };
}

/** The theme every page renders under. daisyUI's own `bumblebee`. */
export const THEME = 'bumblebee';

/**
 * The `<style>` body that applies an operator's colour, or `null`.
 *
 * Scoped to `:root[data-theme="bumblebee"]` — the same specificity daisyUI's
 * own theme block has, and later in the document, so it wins without an
 * `!important`. Emitted by the server component that renders `<head>`, which
 * is what makes the colour arrive with the first byte rather than after a
 * flash of the default.
 *
 * Nothing in the returned string comes from a payer or from a merchant: the
 * only input is a `#rrggbb` this module re-validated, and every character it
 * emits is a digit, a dot, a percent or a space. There is no path by which a
 * mounted file becomes markup.
 */
export function themeStyleSheet(primaryColor: string | null): string | null {
  if (primaryColor === null) {
    return null;
  }
  const override = primaryOverride(primaryColor);
  if (override === null) {
    return null;
  }
  return `:root[data-theme="${THEME}"]{--p:${override['--p']};--pc:${override['--pc']};}`;
}
