/**
 * Colour maths for `theme-contrast.test.ts`: oklch -> linear sRGB, WCAG
 * relative luminance, contrast ratio, and the compiled sheet's resolved
 * `--color-*` values.
 *
 * It lives beside `axe.ts` rather than in the test because a file in this
 * package may not exceed 200 lines (`just verify-ui`), and because the
 * numbers it produces are measurements, not assertions — the test is the
 * file that decides what a measurement has to clear.
 *
 * Published at `./testing/contrast` (2026-09-11, issue #73) so a consumer
 * app can measure the pairs it actually renders against the compiled
 * theme rather than duplicating this math — see `frontends/apps/checkout/
 * src/components/outcome-contrast.test.ts`, which is the only reason this
 * stopped being test-only.
 */
import type postcss from "postcss";

/** sRGB 0-1 (linear-light) from an `oklch(L% C H)` triple. */
export function oklchToLinearRgb(
  l: number,
  c: number,
  h: number,
): [number, number, number] {
  const hue = (h * Math.PI) / 180;
  const a = c * Math.cos(hue);
  const b = c * Math.sin(hue);
  const l_ = l + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = l - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = l - 0.0894841775 * a - 1.291485548 * b;
  const long = l_ ** 3;
  const medium = m_ ** 3;
  const short = s_ ** 3;
  return [
    4.0767416621 * long - 3.3077115913 * medium + 0.2309699292 * short,
    -1.2684380046 * long + 2.6097574011 * medium - 0.3413193965 * short,
    -0.0041960863 * long - 0.7034186147 * medium + 1.707614701 * short,
  ];
}

/** WCAG relative luminance, clamped the way a display clamps an out-of-gamut colour. */
export function relativeLuminance([r, g, b]: [number, number, number]): number {
  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  return 0.2126 * clamp(r) + 0.7152 * clamp(g) + 0.0722 * clamp(b);
}

export function contrastRatio(
  a: [number, number, number],
  b: [number, number, number],
): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

const OKLCH = /^oklch\(\s*([\d.]+)%\s+([\d.]+)\s+([\d.]+)\s*\)$/;

/**
 * Every `--color-*: oklch(...)` custom property the compiled sheet defines,
 * resolved the way the cascade resolves them.
 *
 * The rule that matters here is cascade LAYERS, not source order: a normal
 * declaration outside any `@layer` beats one inside a layer whatever their
 * order and whatever their specificity. daisyUI emits its theme block inside
 * `@layer base`, and `styles.css`'s correction sits outside every layer and
 * therefore wins — the same mechanism
 * `frontends/apps/checkout/src/config/theme.ts` uses at runtime to retint
 * `--color-primary`, which `docs/plans/exp26-notes/lane-b/branded.png` shows
 * working in a real browser.
 *
 * Reading this from the AST rather than with a regex over the text is the
 * point: a text scan sees only order, and would have reported daisyUI's own
 * unreadable pair as the winner. Within each group, later wins.
 */
export function themeColours(
  root: postcss.Root,
): Map<string, [number, number, number]> {
  const layered = new Map<string, [number, number, number]>();
  const unlayered = new Map<string, [number, number, number]>();
  root.walkDecls(/^--color-/, (decl) => {
    const match = OKLCH.exec(decl.value.trim());
    if (match === null) return;
    const name = decl.prop.slice("--color-".length);
    const rgb = oklchToLinearRgb(
      Number(match[1]) / 100,
      Number(match[2]),
      Number(match[3]),
    );
    let inLayer = false;
    let node: postcss.Node | undefined = decl.parent;
    while (node !== undefined) {
      if (node.type === "atrule" && (node as postcss.AtRule).name === "layer") {
        inLayer = true;
        break;
      }
      node = node.parent;
    }
    (inLayer ? layered : unlayered).set(name, rgb);
  });
  return new Map([...layered, ...unlayered]);
}
