import type postcss from "postcss";

import { hexToLinearRgb } from "../config/theme";

/**
 * Colour maths for `outcome-contrast.test.ts`: parse the custom properties
 * a real Tailwind + daisyUI compile actually produces, resolve them the way
 * the cascade resolves them, and turn them into a WCAG contrast ratio.
 *
 * **Ported and rewritten, not copied, from `@vpay/ui/testing/contrast`
 * (2026-09-12, the `@vaam-apps/ui` cutover).** That helper's `themeColours`
 * parsed `oklch(L% C H)` and nothing else — `@vaam-apps/ui`'s theme is
 * authored in hex (`--color-base-100: #0a0b0d`, `dist/styles/theme.css`),
 * so a byte-for-byte port would have compiled, run, and reported a clean
 * suite having resolved a colour count of ZERO. `outcome-contrast.test.ts`
 * asserts a non-zero count before trusting any ratio for exactly this
 * reason.
 *
 * A second, independent reason the old helper cannot be ported unchanged:
 * `InlineBanner` (the component `OutcomePanel` renders its outcome text in)
 * does not paint a `--color-<tone>` / `--color-<tone>-content` pair the way
 * `@vpay/ui`'s `Alert` did. It paints `--state-<hue>-fg` on
 * `--state-<hue>-bg`, and `--state-<hue>-bg` is `rgb(r g b / 0.1)` — **an
 * alpha colour over whatever surface sits behind it**, not an opaque daisyUI
 * semantic colour. `themeColours` below therefore composites any
 * alpha-bearing colour it finds over an explicit backdrop the caller names
 * (`opts.over`), rather than reporting a translucent colour's own channel
 * values as if they were what a payer's eye actually receives.
 */

/** WCAG relative luminance from linear-light channels, clamped as a real display would. */
export function relativeLuminance([r, g, b]: [number, number, number]): number {
  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  return 0.2126 * clamp(r) + 0.7152 * clamp(g) + 0.0722 * clamp(b);
}

/** WCAG contrast ratio between two linear-light colours. */
export function contrastRatio(
  a: [number, number, number],
  b: [number, number, number],
): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** `#rrggbb` → three 0-255 sRGB (gamma-encoded, NOT linear) channels, or `null`. */
export function hexToSrgb255(hex: string): [number, number, number] | null {
  const match = /^#([0-9a-fA-F]{6})$/.exec(hex.trim());
  if (match === null) {
    return null;
  }
  const digits = match[1] as string;
  return [
    Number.parseInt(digits.slice(0, 2), 16),
    Number.parseInt(digits.slice(2, 4), 16),
    Number.parseInt(digits.slice(4, 6), 16),
  ];
}

const RGB_FUNCTION =
  /^rgba?\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*(?:\/\s*([\d.]+)(%)?\s*)?\)$/;

/**
 * One CSS colour value, as this theme actually writes it: `#rrggbb`, the
 * modern `rgb(r g b / a)` / `rgb(r g b)` space syntax daisyUI's own
 * `--state-*-bg`/`-border` tokens use, or the literal `transparent`.
 *
 * Returns 0-255 sRGB channels (gamma-encoded — the space alpha compositing
 * actually happens in, matching what a browser paints) and an alpha in
 * `[0, 1]`. `null` for anything else, including `oklch(...)` — this theme
 * never emits that function, so recognising it would be dead code, not
 * defensiveness.
 */
function parseCssColor(
  raw: string,
): { rgb: [number, number, number]; alpha: number } | null {
  const value = raw.trim();
  if (value === "transparent") {
    return { rgb: [0, 0, 0], alpha: 0 };
  }
  const hex = hexToSrgb255(value);
  if (hex !== null) {
    return { rgb: hex, alpha: 1 };
  }
  const match = RGB_FUNCTION.exec(value);
  if (match === null) {
    return null;
  }
  const [, r, g, b, alphaValue, alphaPercent] = match;
  let alpha = 1;
  if (alphaValue !== undefined) {
    alpha = Number(alphaValue) / (alphaPercent === "%" ? 100 : 1);
  }
  return {
    rgb: [Number(r), Number(g), Number(b)],
    alpha,
  };
}

/** Alpha-composite `fg` (0-255 sRGB) over `bg` (0-255 sRGB), in sRGB space — the space a browser actually blends in. */
function compositeOver(
  fg: [number, number, number],
  alpha: number,
  bg: [number, number, number],
): [number, number, number] {
  return [0, 1, 2].map(
    (i) => alpha * (fg[i] as number) + (1 - alpha) * (bg[i] as number),
  ) as [number, number, number];
}

/** 0-255 sRGB → linear-light, via `theme.ts`'s own transfer function on a synthesised hex string. */
function srgb255ToLinear(
  rgb: [number, number, number],
): [number, number, number] {
  const toHex = (v: number) =>
    Math.round(Math.min(255, Math.max(0, v)))
      .toString(16)
      .padStart(2, "0");
  const hex = `#${toHex(rgb[0])}${toHex(rgb[1])}${toHex(rgb[2])}`;
  const linear = hexToLinearRgb(hex);
  // `toHex` always produces a well-formed #rrggbb, so this cannot be null.
  return linear as [number, number, number];
}

/**
 * Every custom property the compiled sheet defines that parses as a real
 * colour, as linear RGB, resolved the way the cascade resolves them.
 *
 * The rule that matters here is cascade LAYERS, not source order: a normal
 * declaration outside any `@layer` beats one inside a layer whatever their
 * order and whatever their specificity — the same mechanism
 * `frontends/apps/checkout/src/config/theme.ts` uses at runtime to retint
 * `--color-primary`. Within each group (layered / unlayered), later wins.
 *
 * **Keys are normalised**: a leading `--` is stripped, then a leading
 * `color-` is stripped, so `--color-primary` and `--state-danger-fg` both
 * resolve under the names the rest of this test suite actually asks for
 * (`"primary"`, `"state-danger-fg"`). This is more than cosmetic: measured
 * by compiling this app's own `app/globals.css` (`<SP>/dashui-D-probe-
 * compile3.mjs`), `@theme inline`'s `--color-state-danger-fg: var(--state-
 * danger-fg)` DOES compile to a real declaration, inside `@layer theme` —
 * but because it is layered and the direct `--state-danger-fg: #fca5a5`
 * declaration (in `[data-theme="dark"]`, unlayered) is not, the unlayered
 * one wins under the same key without this function ever needing to follow
 * the `var()` indirection by hand.
 *
 * `-border` tokens are skipped outright rather than composited: WCAG's
 * non-text-contrast rule (3:1) applies to them, a border sits over an
 * already-composited fill rather than over `opts.over` directly, and
 * `outcome-contrast.test.ts` does not need them. Reporting a value that
 * looks like a measurement but compounds two unstated assumptions is worse
 * than reporting nothing.
 *
 * @param opts.over — the 0-255 sRGB backdrop to composite an alpha-bearing
 * colour over. Every `--state-*-bg` token this theme defines is `rgb(r g b
 * / 0.1)`; without `over`, a translucent colour is skipped rather than
 * measured as if it were opaque.
 */
export function themeColours(
  root: postcss.Root,
  opts: { over?: [number, number, number] } = {},
): Map<string, [number, number, number]> {
  const layered = new Map<string, [number, number, number]>();
  const unlayered = new Map<string, [number, number, number]>();
  root.walkDecls(/^--/, (decl) => {
    const key = decl.prop.replace(/^--/, "").replace(/^color-/, "");
    if (key.endsWith("-border")) {
      return;
    }
    const parsed = parseCssColor(decl.value);
    if (parsed === null) {
      return;
    }
    let rgb255 = parsed.rgb;
    if (parsed.alpha < 1) {
      if (opts.over === undefined) {
        return;
      }
      rgb255 = compositeOver(parsed.rgb, parsed.alpha, opts.over);
    }
    let inLayer = false;
    let node: postcss.Node | undefined = decl.parent;
    while (node !== undefined) {
      if (node.type === "atrule" && (node as postcss.AtRule).name === "layer") {
        inLayer = true;
        break;
      }
      node = node.parent;
    }
    (inLayer ? layered : unlayered).set(key, srgb255ToLinear(rgb255));
  });
  return new Map([...layered, ...unlayered]);
}
