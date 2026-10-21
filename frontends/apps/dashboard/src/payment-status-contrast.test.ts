/**
 * The colour pairs the dashboard actually renders for a PaymentIntent's
 * status, measured from the compiled theme rather than walked in a browser
 * — the checkout app's `outcome-contrast.test.ts` (issue #73) has no
 * dashboard counterpart, and the dashboard just flipped from daisyUI's
 * light `bumblebee` to `@vaam-apps/ui`'s dark theme (2026-09-12) — exactly
 * where a contrast regression hides unseen, because nothing was looking.
 *
 * # Two different things get painted from the SAME hue tokens, and both are measured
 *
 * 1. **`PaymentStatusPill`** (`payment-status.ts`, `createStatusPill`).
 *    Every row in `PAYMENT_STATUS_SYSTEM` sets `attention: "quiet"`, and
 *    `StatusPill` (`@vaam-apps/ui/dist/components/status/status-pill.js`)
 *    only applies `hue.bg`/`hue.border` when `resolvedVariant === "loud"` —
 *    read directly, not assumed. Neither call site
 *    (`payments-table.tsx`, `payment-detail.tsx`) passes `variant`, so
 *    neither ever renders loud. What IS hue-coloured on a quiet pill is
 *    only the glyph (`StateMark`, painted `className={hue.fg}`, `fill`/
 *    `stroke="currentColor"`) — the label text renders
 *    `text-muted-foreground` instead, deliberately hue-independent. So the
 *    pair that matters for "can an operator tell this glyph's hue apart" is
 *    `--state-<hue>-fg` directly against the page ground (`--color-base-100`)
 *    — there is no pill background to composite over, because none is ever
 *    painted here.
 * 2. **`InlineBanner`** (`form-alert.tsx`, `read-failure.tsx`,
 *    `payment-detail.tsx` — `variant="danger"`; `enrolment-panel.tsx`,
 *    `app/login/page.tsx`, `app/login/password/page.tsx` —
 *    `variant="warning"`; no call site in this app uses `"success"` or
 *    `"uncertain"`, verified by `git grep -n 'InlineBanner variant='` over
 *    `frontends/apps/dashboard`). `InlineBanner` (`inline-banner.js`) paints
 *    `text-state-<hue>-fg` UNCONDITIONALLY on `bg-state-<hue>-bg` — there is
 *    no quiet/loud switch here, only the `variant` prop — and
 *    `--state-<hue>-bg` is `rgb(r g b / 0.1)`: an ALPHA colour over whatever
 *    surface sits behind it. Composited over `--color-base-100`, per the
 *    "honest limit" below.
 *
 * **Grounds.** Since the Material 3 Expressive revamp (2026-10-21) pills
 * and banners sit either on the page (`--color-base-100`) or inside a
 * `Card` (`--color-base-300`), so every pair is measured over both, in both
 * themes. The `InstrumentPanel` ground is the library's own measurement
 * (its contrast gate) and `instrument-register.test.ts` holds this app to
 * the one rule it derives.
 *
 * `src/testing/contrast.ts`'s own header explains why this is a port of
 * the checkout's helper rather than a shared import.
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import tailwind from "@tailwindcss/postcss";
import postcss from "postcss";
import { describe, expect, it } from "vitest";

import { STATUS_SYSTEMS } from "./status-systems";
import { contrastRatio, themeColours } from "./testing/contrast";

// A relative path to this app's OWN compiled stylesheet — the same file
// `styling-gate.test.ts` compiles, and for the same reason `just ci` never
// builds this app (`dashui-measured-facts.md` §5), so a vitest test is the
// only place this can be gated.
//
// Built with `fileURLToPath`/`join` rather than a bare `new URL(...)` passed
// to `readFileSync` (the checkout's `outcome-contrast.test.ts` pattern,
// which runs under `environment: "node"`): this app's vitest config runs
// under `environment: "jsdom"`, whose polyfilled `URL` is not the class
// Node's `fs` module recognises, and `readFileSync(url, ...)` throws "The
// URL must be of scheme file" as a result — measured directly, not assumed.
// `styling-gate.test.ts` already takes this same path-building approach for
// the same reason.
const APP_DIR = join(dirname(fileURLToPath(import.meta.url)), "..");
const GLOBALS_CSS_PATH = join(APP_DIR, "app", "globals.css");

const compiled = await postcss([tailwind()]).process(
  readFileSync(GLOBALS_CSS_PATH, "utf8"),
  { from: GLOBALS_CSS_PATH, to: GLOBALS_CSS_PATH },
);

/**
 * The grounds a pill glyph or a banner actually sits on in this app, read
 * from the compiled theme rather than typed in. Two of them since the
 * Material 3 Expressive revamp (2026-10-21): the page (`--color-base-100`,
 * M3's `surface`) and the card every list and detail section now sits in
 * (`--color-base-300`, `Card`'s `surface-container`). This said one ground,
 * hard-coded as `#0a0b0d`, until `@vaam-apps/ui@0.5.0` replaced the palette
 * — at which point the literal was simply wrong, and a ground nobody
 * paints any more is the thing this suite exists to refuse.
 */
const GROUNDS = ["base-100", "base-300"] as const;
const THEMES = ["dark", "light"] as const;

const colourSets = THEMES.flatMap((theme) =>
  GROUNDS.map((ground) => ({
    theme,
    ground,
    colours: themeColours(compiled.root, { over: ground, theme }),
  })),
);

/**
 * Every hue the app's status systems actually assign, deduplicated rather
 * than hand-listed, so a hue added or changed in a table is measured
 * automatically instead of by remembering to update a second list.
 */
const STATUS_PILL_HUES = [
  ...new Set(
    STATUS_SYSTEMS.flatMap((system) =>
      Object.values(system).map((meta) => meta.hue),
    ),
  ),
];

/**
 * The `InlineBanner` variants this app renders. Hand-listed: nothing owns a
 * typed table of "every banner variant in use". `uncertain` is the
 * Overview's "a count could not be read" notice.
 */
const BANNER_VARIANTS = ["danger", "warning", "uncertain"];

describe("dashboard status colours, contrast measured from the compiled theme", () => {
  // (0) THE ANTI-VACUITY CASE. `themeColours` silently skips any
  // declaration whose value it cannot resolve — which is how the 0.5.0 bump
  // first surfaced: every token a `var()`, zero colours resolved. A count,
  // per theme and ground, before any ratio.
  for (const { theme, ground, colours } of colourSets) {
    it(`parses a non-zero number of colours (${theme}, over ${ground})`, () => {
      expect(
        colours.size,
        "themeColours resolved nothing — the parser and the theme disagree",
      ).toBeGreaterThan(20);
      expect(colours.get(ground), `--color-${ground}`).toBeDefined();
      for (const hue of STATUS_PILL_HUES) {
        expect(
          colours.get(`state-${hue}-fg`),
          `--state-${hue}-fg`,
        ).toBeDefined();
      }
      for (const variant of BANNER_VARIANTS) {
        expect(colours.get(`state-${variant}-fg`)).toBeDefined();
        expect(colours.get(`state-${variant}-bg`)).toBeDefined();
      }
    });
  }

  // (1) A KNOWN NUMBER, so a parser that returns plausible garbage is
  // caught. `--state-warning-fg` text on `--state-warning-bg` composited
  // over `--md-sys-color-surface`, computed independently in Python from
  // the three literals `@vaam-apps/ui@0.5.0`'s theme.css declares — dark:
  // #ffb599 on rgb(255 181 153 / 0.12) over #0f0c1b = 9.131; light:
  // #963e17 on rgb(150 62 23 / 0.08) over #fdf7ff = 5.889 — not from this
  // file's own machinery.
  it("agrees with a hand-computed ratio on the InlineBanner warning pair", () => {
    const expected = { dark: 9.131, light: 5.889 } as const;
    for (const { theme, ground, colours } of colourSets) {
      if (ground !== "base-100") continue;
      const fg = colours.get("state-warning-fg");
      const bg = colours.get("state-warning-bg");
      expect(fg && bg).toBeTruthy();
      const ratio = contrastRatio(
        fg as [number, number, number],
        bg as [number, number, number],
      );
      expect(ratio, theme).toBeCloseTo(expected[theme], 2);
    }
  });

  // (2) The pair a status pill paints: the glyph's `--state-<hue>-fg`
  // against the ground it sits on. WCAG 1.4.11 (non-text contrast), 3:1 —
  // the glyph is a graphical object conveying state. Both themes, both
  // grounds: light became selectable in 0.1.2 and nothing measured it here
  // until this revamp.
  for (const { theme, ground, colours } of colourSets) {
    for (const hue of STATUS_PILL_HUES) {
      it(`status hue "${hue}" (${theme}, on ${ground}): the glyph clears 3:1`, () => {
        const fg = colours.get(`state-${hue}-fg`);
        const bg = colours.get(ground);
        expect(fg && bg).toBeTruthy();
        const ratio = contrastRatio(
          fg as [number, number, number],
          bg as [number, number, number],
        );
        expect(ratio, `${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(3);
      });
    }
  }

  // (3) The pair `InlineBanner` paints: `--state-<hue>-fg` text on
  // `--state-<hue>-bg` composited over the ground. Real body text, so WCAG
  // 1.4.3 AA (4.5:1).
  for (const { theme, ground, colours } of colourSets) {
    for (const variant of BANNER_VARIANTS) {
      it(`InlineBanner variant="${variant}" (${theme}, on ${ground}) clears 4.5:1`, () => {
        const fg = colours.get(`state-${variant}-fg`);
        const bg = colours.get(`state-${variant}-bg`);
        expect(fg && bg).toBeTruthy();
        const ratio = contrastRatio(
          fg as [number, number, number],
          bg as [number, number, number],
        );
        expect(ratio, `${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(4.5);
      });
    }
  }
});
