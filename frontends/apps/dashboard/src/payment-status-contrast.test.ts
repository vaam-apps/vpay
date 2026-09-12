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
 * **Honest limit, stated once here rather than left implicit**: every
 * `InlineBanner` and every `PaymentStatusPill` this app renders sits
 * directly on the page background (`ScreenStack` → `<main>` → `<body>`,
 * none of which sets its own background class) or inside a plain
 * `<section>`/`<div role="…">` that does not either — confirmed by reading
 * every call site listed above. If one is ever placed inside a `Card` or
 * `bg-surface-1`/`bg-surface-2` container, this backdrop is the wrong one
 * and every ratio below would need recomputing against that surface
 * instead.
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

import { PAYMENT_STATUS_SYSTEM } from "./payment-status";
import { contrastRatio, hexToSrgb255, themeColours } from "./testing/contrast";

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
 * `--color-base-100`, read from `@vaam-apps/ui/dist/styles/theme.css`
 * directly rather than derived: it is the ground every `PaymentStatusPill`
 * glyph and every `InlineBanner`'s alpha `-bg` composites against on this
 * app's pages (see the header comment's "honest limit"), and nothing in
 * this app overrides it at runtime — unlike the checkout, the dashboard has
 * no `branding.yaml` colour override and no `config/theme.ts` to apply one.
 */
const PAGE_BACKGROUND_255 = hexToSrgb255("#0a0b0d") as [number, number, number];

const colours = themeColours(compiled.root, { over: PAGE_BACKGROUND_255 });

/**
 * Every hue `PAYMENT_STATUS_SYSTEM` actually assigns, deduplicated in
 * table order rather than hand-listed: `requires_payment_method` (neutral),
 * `requires_action` (warning), `processing` (parked), `succeeded`
 * (success), `canceled` (warning again, already counted). Deriving this
 * from the table itself — rather than typing `["neutral", "warning",
 * "parked", "success"]` — means a hue added or changed here is measured
 * automatically instead of by remembering to update a second list.
 */
const STATUS_PILL_HUES = [
  ...new Set(Object.values(PAYMENT_STATUS_SYSTEM).map((meta) => meta.hue)),
];

/**
 * The `InlineBanner` variants this app actually renders, per the header
 * comment's `git grep`. Hand-listed, not derived: unlike
 * `PAYMENT_STATUS_SYSTEM`, nothing in this app owns a typed table of "every
 * banner variant in use" the way `checkoutOutcomeVariant` does for the
 * checkout — these are six independent JSX literals across six files.
 */
const BANNER_VARIANTS = ["danger", "warning"];

describe("dashboard status colours, contrast measured from the compiled theme", () => {
  // (0) THE ANTI-VACUITY CASE. `themeColours` silently skips any
  // declaration whose value it cannot parse — that is how a themeColours
  // still shaped for `oklch(...)`-only input would report a clean run over
  // this hex-and-rgb() theme having resolved ZERO colours. A count, before
  // any ratio, on the real compiled sheet.
  it("parses a non-zero number of colours out of the compiled sheet", () => {
    expect(
      colours.size,
      "themeColours resolved nothing — the parser and the theme disagree",
    ).toBeGreaterThan(20);
  });

  // (1) Every token the status pill actually names resolves, plus the
  // ground every glyph is measured against.
  it("resolves a colour for every status-pill hue's glyph token, and for the page ground", () => {
    expect(colours.get("base-100"), "--color-base-100").toBeDefined();
    for (const hue of STATUS_PILL_HUES) {
      expect(colours.get(`state-${hue}-fg`), `--state-${hue}-fg`).toBeDefined();
    }
  });

  // (2) Every token the InlineBanner variants this app renders actually
  // name resolves.
  it("resolves a colour for every InlineBanner variant this app renders", () => {
    for (const variant of BANNER_VARIANTS) {
      expect(
        colours.get(`state-${variant}-fg`),
        `--state-${variant}-fg`,
      ).toBeDefined();
      expect(
        colours.get(`state-${variant}-bg`),
        `--state-${variant}-bg`,
      ).toBeDefined();
    }
  });

  // (3) A KNOWN NUMBER, so a parser that returns plausible garbage is
  // caught. #fdba74 text on rgb(251 146 60 / 0.1) over #0a0b0d, computed
  // independently (`<SP>/dashui-G-hand-calc.mjs`) from the three hex values
  // `@vaam-apps/ui/dist/styles/theme.css` declares for `--state-warning-fg`,
  // `--state-warning-bg`'s own channel triplet, and `--color-base-100` —
  // not from this file's own machinery.
  it("agrees with a hand-computed ratio on the InlineBanner warning pair", () => {
    const fg = colours.get("state-warning-fg");
    const bg = colours.get("state-warning-bg");
    expect(fg && bg).toBeTruthy();
    const ratio = contrastRatio(
      fg as [number, number, number],
      bg as [number, number, number],
    );
    expect(ratio).toBeCloseTo(10.28, 1);
  });

  // (4) The pair `StatusPill` actually paints for a quiet pill (every pill
  // this app renders): the glyph's `--state-<hue>-fg` directly against the
  // page ground. WCAG 1.4.11 (non-text contrast), 3:1 — the glyph is a
  // graphical object conveying state, not text — is the applicable
  // success criterion, not 1.4.3's 4.5:1 for text.
  for (const hue of STATUS_PILL_HUES) {
    it(`status hue "${hue}": the glyph clears WCAG non-text contrast (3:1) against the page ground`, () => {
      const fg = colours.get(`state-${hue}-fg`);
      const ground = colours.get("base-100");
      expect(
        fg && ground,
        `--state-${hue}-fg and --color-base-100`,
      ).toBeTruthy();
      const ratio = contrastRatio(
        fg as [number, number, number],
        ground as [number, number, number],
      );
      expect(
        ratio,
        `status hue "${hue}" glyph vs page ground: ${ratio.toFixed(2)}:1`,
      ).toBeGreaterThanOrEqual(3);
    });
  }

  // (5) The pair `InlineBanner` actually paints for the variants this app
  // renders: `--state-<hue>-fg` text on `--state-<hue>-bg`, composited over
  // the page ground because the bg is `rgb(… / 0.1)`. This is real body
  // text (`FormAlert`'s error sentence, `EnrolmentPanel`'s warning), so
  // WCAG 1.4.3 AA (4.5:1) applies — the same bar `outcome-contrast.test.ts`
  // holds the checkout's own `InlineBanner` text to.
  for (const variant of BANNER_VARIANTS) {
    it(`InlineBanner variant="${variant}": clears WCAG AA (4.5:1) as actually rendered`, () => {
      const fg = colours.get(`state-${variant}-fg`);
      const bg = colours.get(`state-${variant}-bg`);
      expect(
        fg && bg,
        `--state-${variant}-fg and --state-${variant}-bg (composited over --color-base-100)`,
      ).toBeTruthy();
      const ratio = contrastRatio(
        fg as [number, number, number],
        bg as [number, number, number],
      );
      expect(
        ratio,
        `InlineBanner variant="${variant}": ${ratio.toFixed(2)}:1`,
      ).toBeGreaterThanOrEqual(4.5);
    });
  }
});
