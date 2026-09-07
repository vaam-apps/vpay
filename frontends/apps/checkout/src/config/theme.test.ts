/**
 * The colour conversion, against daisyUI's own converter.
 *
 * The six expected pairs below were **produced by daisyUI**, not by reading
 * this module's output back:
 *
 * ```
 * node -e "const f=require('daisyui/src/theming/functions.js');
 *          console.log(f.convertColorFormat({primary:'#f3c623'}))"
 * ```
 *
 * That is what makes them a test rather than a snapshot. daisyUI computes
 * them with culori; this module computes them with forty lines of arithmetic
 * and no dependency, and the two agree to the last digit — so a drift is a
 * failure here instead of a page that is quietly the wrong colour.
 */
import { describe, expect, it } from 'vitest';

import { THEME, primaryOverride, themeStyleSheet } from './theme';

/** hex → `[--p, --pc]`, exactly as `daisyui/src/theming/functions.js` renders them. */
const DAISYUI: [string, string, string][] = [
  // bumblebee's own amber, near enough to be the colour an operator most
  // likely writes down first.
  ['#f3c623', '84.2251% 0.165456 91.330667', '16.845% 0.033091 91.330667'],
  ['#ff0000', '62.7955% 0.257683 29.233885', '12.5591% 0.051537 29.233885'],
  // Achromatic, both ends: the case where the hue is meaningless and both
  // implementations still have to spell it the same way.
  ['#ffffff', '100% 0 0', '20% 0 0'],
  ['#000000', '0% 0 0', '80% 0 0'],
  // Dark enough that the foreground goes toward WHITE rather than black —
  // the branch `isDark` exists for.
  ['#1d4ed8', '48.8198% 0.217165 264.376304', '89.764% 0.043433 264.376304'],
  ['#00a651', '63.4564% 0.170019 151.181276', '12.6913% 0.034004 151.181276'],
];

describe('primaryOverride', () => {
  for (const [hex, primary, content] of DAISYUI) {
    it(`converts ${hex} exactly as daisyUI does`, () => {
      expect(primaryOverride(hex)).toEqual({ '--p': primary, '--pc': content });
    });
  }

  it('sends a light colour’s foreground toward black and a dark one’s toward white', () => {
    // Not a restatement of the table: this is the property the table's rows
    // happen to cover, asserted as a comparison rather than as two strings.
    const light = primaryOverride('#f3c623');
    const dark = primaryOverride('#1d4ed8');
    const lightness = (value: string) => Number.parseFloat(value);
    expect(lightness(light!['--pc'])).toBeLessThan(lightness(light!['--p']));
    expect(lightness(dark!['--pc'])).toBeGreaterThan(lightness(dark!['--p']));
  });

  it('is case-insensitive about the hex digits', () => {
    expect(primaryOverride('#F3C623')).toEqual(primaryOverride('#f3c623'));
  });

  for (const bad of ['', '#fff', 'f3c623', '#f3c62', '#f3c6233', 'red', '#gggggg', '#f3c62 3']) {
    it(`refuses ${JSON.stringify(bad)} rather than guessing a colour`, () => {
      expect(primaryOverride(bad)).toBeNull();
    });
  }
});

describe('themeStyleSheet', () => {
  it('scopes the override to the theme the app actually renders under', () => {
    expect(themeStyleSheet('#f3c623')).toBe(
      ':root[data-theme="bumblebee"]{--p:84.2251% 0.165456 91.330667;--pc:16.845% 0.033091 91.330667;}',
    );
    expect(THEME).toBe('bumblebee');
  });

  it('emits nothing at all when no colour is configured', () => {
    expect(themeStyleSheet(null)).toBeNull();
  });

  it('emits nothing for a value that is not a colour, rather than a broken rule', () => {
    // The alternative — emitting `--p:;` — would be a CSS parse error in the
    // page's own `<head>`, and daisyUI's variable would be left at whatever
    // the browser did with the malformed declaration.
    expect(themeStyleSheet('javascript:alert(1)')).toBeNull();
    expect(themeStyleSheet('</style><script>')).toBeNull();
  });

  it('contains nothing but digits and punctuation, so a mounted file cannot become markup', () => {
    for (const [hex] of DAISYUI) {
      const sheet = themeStyleSheet(hex);
      expect(sheet).not.toBeNull();
      // Everything after the selector: the values this module composed.
      const values = sheet!.slice(sheet!.indexOf('{'));
      expect(values).toMatch(/^[0-9a-z%.\s{}:;-]+$/);
      expect(values).not.toContain('<');
    }
  });
});
