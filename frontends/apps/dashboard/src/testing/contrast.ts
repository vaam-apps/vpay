import type postcss from "postcss";

/**
 * Colour maths for `payment-status-contrast.test.ts`: parse the custom
 * properties a real Tailwind + daisyUI compile actually produces, resolve
 * them the way the cascade resolves them, and turn them into a WCAG
 * contrast ratio.
 *
 * **Ported from `frontends/apps/checkout/src/testing/contrast.ts`
 * (2026-09-12), not imported from it.** The two apps' PostCSS pipelines are
 * separate processes over separate `globals.css` entries with no shared
 * runtime dependency between them (`AGENTS.md`'s "no cross-app import"
 * shape), and the deleted `@vpay/ui/testing/contrast` is exactly the
 * dependency this file must not resurrect. The checkout copy also imports
 * its transfer function (`hexToLinearRgb`) from its own `config/theme.ts`,
 * which exists there to re-tint `branding.yaml`'s merchant colour at
 * runtime — the dashboard has no runtime colour override and no such
 * module, so `linearize`/`hexToLinearRgb` are inlined below instead of
 * manufacturing a `config/theme.ts` this app has no other use for.
 *
 * Same two measured facts as the checkout's copy, both true of this app's
 * compiled sheet too (`<SP>/dashui-orch-dash.css`, `dashui-measured-facts.md`
 * §4): **the theme is authored in hex/`rgb()`, not `oklch()`**, so a helper
 * that only recognised `oklch(L% C H)` would parse zero colours and report
 * a clean run; and **the status tokens are alpha over a surface**
 * (`--state-warning-bg: rgb(251 146 60 / 0.1)`), not opaque pairs, so an
 * alpha-bearing colour is composited over an explicit backdrop the caller
 * names rather than measured as if it were opaque.
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

/** sRGB 0-1 -> linear-light 0-1. The IEC 61966-2-1 transfer function. */
function linearize(channel: number): number {
  return channel <= 0.04045
    ? channel / 12.92
    : Math.pow((channel + 0.055) / 1.055, 2.4);
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

/** 0-255 sRGB → linear-light, via this file's own IEC 61966-2-1 transfer function. */
function srgb255ToLinear(
  rgb: [number, number, number],
): [number, number, number] {
  return [
    linearize(rgb[0] / 255),
    linearize(rgb[1] / 255),
    linearize(rgb[2] / 255),
  ];
}

/**
 * Every custom property the compiled sheet defines that parses as a real
 * colour, as linear RGB, resolved the way the cascade resolves them.
 *
 * The rule that matters here is cascade LAYERS, not source order: a normal
 * declaration outside any `@layer` beats one inside a layer whatever their
 * order and whatever their specificity — the same mechanism
 * `@vaam-apps/ui/dist/styles/theme.css` uses to declare its `[data-theme=
 * "dark"]` block unlayered while `@theme inline`'s aliases land inside
 * `@layer theme`. Within each group (layered / unlayered), later wins.
 *
 * **Keys are normalised**: a leading `--` is stripped, then a leading
 * `color-` is stripped, so `--color-base-100` and `--state-warning-fg` both
 * resolve under the names this test suite actually asks for (`"base-100"`,
 * `"state-warning-fg"`).
 *
 * `-border` tokens are skipped outright rather than composited: WCAG's
 * non-text-contrast rule (3:1) applies to them, a border sits over an
 * already-composited fill rather than over `opts.over` directly, and
 * `payment-status-contrast.test.ts` does not need them. Reporting a value
 * that looks like a measurement but compounds two unstated assumptions is
 * worse than reporting nothing.
 *
 * @param opts.over — the 0-255 sRGB backdrop to composite an alpha-bearing
 * colour over. Every `--state-<hue>-bg` token this theme defines that is
 * not the literal `transparent` is `rgb(r g b / 0.1)`; without `over`, a
 * translucent colour is skipped rather than measured as if it were opaque.
 * `transparent` itself (`neutral`/`success`) composites to exactly `over`,
 * which is correct: nothing is painted, so the ground shows through whole.
 */
/**
 * Which theme's declarations to read.
 *
 * **Required since `@vaam-apps/ui@0.1.2`, which registers two themes.** Every
 * `--state-*` token is now declared twice — `--state-danger-fg` is `#fca5a5`
 * under `dark` and `#a80f35` under `light` — and this function keyed a flat
 * map by variable name, so the later block silently won. `outcome-contrast.
 * test.ts` then compared the LIGHT foreground against a background
 * composited over the DARK `--color-base-100`, a pair no screen paints, and
 * reported the payment-outcome banners at 2.55-2.76:1 against AA's 4.5:1.
 * Nothing was wrong with the theme; the harness could not represent two of
 * them. The browser suite (`just test-storybook`), which measures painted
 * pixels rather than tokens, passed those same screens throughout — which is
 * how the two were told apart.
 *
 * A declaration is skipped when an enclosing selector pins a DIFFERENT
 * theme. Unpinned declarations (`:where(:root)`) are kept for either, which
 * is what the cascade does.
 */
export type ThemeName = "dark" | "light";

function pinnedToOtherTheme(
  decl: postcss.Declaration,
  theme: ThemeName,
): boolean {
  let node: postcss.Node | undefined = decl.parent;
  while (node !== undefined) {
    if (node.type === "rule") {
      const selector = (node as postcss.Rule).selector;
      const pinned = [
        ...selector.matchAll(/\[data-theme=["']?([\w-]+)["']?\]/g),
      ].map((m) => m[1]);
      if (pinned.length > 0 && !pinned.includes(theme)) {
        return true;
      }
    }
    node = node.parent;
  }
  return false;
}

export function themeColours(
  root: postcss.Root,
  opts: { over?: [number, number, number]; theme?: ThemeName } = {},
): Map<string, [number, number, number]> {
  const theme: ThemeName = opts.theme ?? "dark";
  const layered = new Map<string, [number, number, number]>();
  const unlayered = new Map<string, [number, number, number]>();
  root.walkDecls(/^--/, (decl) => {
    const key = decl.prop.replace(/^--/, "").replace(/^color-/, "");
    if (key.endsWith("-border")) {
      return;
    }
    if (pinnedToOtherTheme(decl, theme)) {
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
