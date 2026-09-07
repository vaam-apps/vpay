/**
 * `themeStyleSheet`, tested for the two properties anyone actually cares
 * about: it never emits anything but a validated colour, and the
 * foreground it asks `color-mix()` to compute stays readable.
 *
 * Under daisyUI 5, `theme.ts` no longer converts sRGB to OKLCh — it emits
 * `--color-primary` as written and derives `--color-primary-content` with
 * the CSS `color-mix()` the browser itself will run, so there is no longer
 * a daisyUI-shaped output string to compare against daisyUI's own converter
 * (that was the daisyUI 4 test). What replaced it: an **independent**
 * re-implementation of `color-mix(in oklch, colour 20%, white|black)`,
 * written here once using Björn Ottosson's published OKLab matrices, that
 * computes the contrast ratio `theme.ts`'s rule produces and asserts it
 * clears WCAG AA (4.5:1) for six colours, including the three the module's
 * own doc comment uses as worked examples. A drift between `theme.ts`'s
 * rule ("80% of the way to white/black") and this independent computation
 * would be a bug in the rule itself — the property this file existed to
 * guard even under daisyUI 4, now asserted directly rather than by
 * comparison to daisyUI's own converter.
 *
 * One case per colour and one case per refused value, not a `for` loop
 * inside a single `it` — so a single bad hex or a single failed contrast
 * ratio is reported by name rather than folded into one assertion.
 */
import { describe, expect, it } from 'vitest';

import { THEME, themeStyleSheet } from './theme';

// --- An independent OKLCh round trip, used only to verify contrast. -------
// Not imported from theme.ts: theme.ts no longer does this arithmetic at
// all (daisyUI 5's --color-primary takes any CSS colour), so this exists
// solely to check the CONTRAST PROPERTY the browser's own `color-mix()`
// will produce from theme.ts's formula.

function linearize(channel: number): number {
  return channel <= 0.04045 ? channel / 12.92 : Math.pow((channel + 0.055) / 1.055, 2.4);
}

function hexToLinearRgb(hex: string): [number, number, number] {
  const channel = (start: number): number => Number.parseInt(hex.slice(start, start + 2), 16) / 255;
  return [linearize(channel(1)), linearize(channel(3)), linearize(channel(5))];
}

interface Oklab {
  l: number;
  a: number;
  b: number;
}

function linearRgbToOklab([r, g, b]: [number, number, number]): Oklab {
  const long = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
  const medium = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
  const short = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
  const l_ = Math.cbrt(long);
  const m_ = Math.cbrt(medium);
  const s_ = Math.cbrt(short);
  return {
    l: 0.2104542553 * l_ + 0.793617785 * m_ - 0.0040720468 * s_,
    a: 1.9779984951 * l_ - 2.428592205 * m_ + 0.4505937099 * s_,
    b: 0.0259040371 * l_ + 0.782771766 * m_ - 0.808675766 * s_,
  };
}

function oklabToLinearRgb({ l, a, b }: Oklab): [number, number, number] {
  const l_ = l + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = l - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = l - 0.0894841775 * a - 1.291485548 * b;
  const long = l_ ** 3;
  const medium = m_ ** 3;
  const short = s_ ** 3;
  return [
    +4.0767416621 * long - 3.3077115913 * medium + 0.2309699292 * short,
    -1.2684380046 * long + 2.6097574011 * medium - 0.3413193965 * short,
    -0.0041960863 * long - 0.7034186147 * medium + 1.707614701 * short,
  ];
}

/** WCAG relative luminance from linear-light channels, clamped as a real display would. */
function relativeLuminance([r, g, b]: [number, number, number]): number {
  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  return 0.2126 * clamp(r) + 0.7152 * clamp(g) + 0.0722 * clamp(b);
}

/** WCAG contrast ratio between two linear-light colours. */
function contrastRatio(a: [number, number, number], b: [number, number, number]): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  const lighter = Math.max(la, lb);
  const darker = Math.min(la, lb);
  return (lighter + 0.05) / (darker + 0.05);
}

function isDark(hex: string): boolean {
  const luminance = relativeLuminance(hexToLinearRgb(hex));
  const againstBlack = (luminance + 0.05) / 0.05;
  const againstWhite = 1.05 / (luminance + 0.05);
  return againstBlack < againstWhite;
}

/**
 * `color-mix(in oklch, hex 20%, white|black)`, computed independently.
 *
 * White and black are both achromatic (chroma 0, hue undefined), so mixing
 * toward either in OKLCh reduces to a linear interpolation of lightness and
 * chroma while the hue is carried through unchanged from `hex` — CSS Color
 * 4's rule for mixing with a hue-less endpoint. That is exactly `theme.ts`'s
 * own stated rule ("80% of the way … lightness and chroma only"), so this
 * function is the independent check of that rule rather than a restatement
 * of it. Returns linear-light sRGB, ready for `relativeLuminance`.
 */
function mixTowardAchromatic(hex: string, towards: 'white' | 'black'): [number, number, number] {
  const oklab = linearRgbToOklab(hexToLinearRgb(hex));
  const targetL = towards === 'white' ? 1 : 0;
  const mixed: Oklab = {
    l: oklab.l + 0.8 * (targetL - oklab.l),
    a: oklab.a * 0.2,
    b: oklab.b * 0.2,
  };
  return oklabToLinearRgb(mixed);
}

/** The colours `theme.ts`'s doc comment and this suite both use as worked examples. */
const COLOURS = ['#f3c623', '#ff0000', '#ffffff', '#000000', '#1d4ed8', '#00a651'];

/** Values `settings.ts` would already have refused before this module sees them, checked here too. */
const NOT_A_COLOUR = [
  '',
  '#fff',
  'f3c623',
  '#f3c62',
  '#f3c6233',
  'red',
  '#gggggg',
  '#f3c62 3',
  'javascript:alert(1)',
  '</style><script>',
];

describe('themeStyleSheet', () => {
  it('emits nothing at all when no colour is configured', () => {
    expect(themeStyleSheet(null)).toBeNull();
  });

  for (const bad of NOT_A_COLOUR) {
    it(`refuses ${JSON.stringify(bad)} rather than a broken rule`, () => {
      expect(themeStyleSheet(bad)).toBeNull();
    });
  }

  it('is case-insensitive about the hex digits', () => {
    expect(themeStyleSheet('#F3C623')).toBe(
      ':root[data-theme="bumblebee"]{--color-primary:#F3C623;' +
        '--color-primary-content:color-mix(in oklch, #F3C623 20%, black);}',
    );
  });

  it('scopes the override to the theme the app actually renders under', () => {
    expect(THEME).toBe('bumblebee');
    expect(themeStyleSheet('#1d4ed8')).toContain(':root[data-theme="bumblebee"]{');
  });

  for (const hex of COLOURS) {
    it(`emits ${hex} as --color-primary, verbatim, and mixes toward ${isDark(hex) ? 'white' : 'black'}`, () => {
      const towards = isDark(hex) ? 'white' : 'black';
      expect(themeStyleSheet(hex)).toBe(
        `:root[data-theme="bumblebee"]{--color-primary:${hex};` +
          `--color-primary-content:color-mix(in oklch, ${hex} 20%, ${towards});}`,
      );
    });
  }

  for (const hex of COLOURS) {
    it(`contains nothing but ${hex}, digits and punctuation, so a mounted file cannot become markup`, () => {
      const sheet = themeStyleSheet(hex);
      expect(sheet).not.toBeNull();
      // Everything after the selector: the values this module composed.
      const values = sheet!.slice(sheet!.indexOf('{'));
      expect(values).toMatch(/^[0-9a-z#%.,()\s{}:;-]+$/i);
      expect(values).not.toContain('<');
      expect(values).not.toContain('script');
      // The colour must appear exactly as given — re-emitted, not rewritten.
      expect(values.match(new RegExp(hex, 'gi'))).toHaveLength(2);
    });
  }

  for (const hex of COLOURS) {
    it(`clears WCAG AA contrast (4.5:1) for the foreground it computes for ${hex}`, () => {
      const dark = isDark(hex);
      const original = hexToLinearRgb(hex);
      const foreground = mixTowardAchromatic(hex, dark ? 'white' : 'black');
      const ratio = contrastRatio(original, foreground);
      expect(ratio, `${hex}: contrast ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(4.5);
    });
  }
});
