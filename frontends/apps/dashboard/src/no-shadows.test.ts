/**
 * No shadows in this app's own source — depth is a surface step, not a blur.
 *
 * The owner's rulings on the sibling product: a floating surface is flat and
 * parted from the page by its fill alone, and the landing site "has no shadows
 * of any kind". This dashboard follows both, and the rule is stated as narrowly
 * as it can be enforced: **this app never asks for one.** `@vaam-apps/ui`'s own
 * components draw an elevation of their own (`DrawerContent`'s
 * `shadow-[var(--md-sys-elevation-level1)]`, `Button`'s `elevated` variant) and
 * this app does not fight them; what it can do is never write the utility, the
 * variant or the CSS property itself, so a shadow in this app is always a
 * decision in `@vaam-apps/ui` and never one made here by accident.
 *
 * Scanned, in every `.ts`, `.tsx` and `.css` file under `app/` and `src/`
 * (comments stripped, so a sentence about the rule does not trip it):
 *
 * * a Tailwind `shadow`, `shadow-*`, `drop-shadow*`, `text-shadow*` or
 *   `inset-shadow*` utility, with or without a variant prefix;
 * * `variant="elevated"` — `Button`'s and `Card`'s shadowed fill;
 * * the CSS properties `box-shadow`, `text-shadow` and `filter: drop-shadow`,
 *   the `--shadow*` and `--md-sys-elevation*` custom properties, and the
 *   React style keys `boxShadow` / `textShadow`.
 */
import { describe, expect, it } from "vitest";

import { appSources, stripComments } from "./testing/sources";

const PATTERNS: readonly { readonly name: string; readonly re: RegExp }[] = [
  {
    name: "a Tailwind shadow utility",
    re: /(?<![\w-])(?:[a-z0-9-]+:)*(?:drop-|text-|inset-)?shadow(?:-(?:\[[^\]]*\]|[a-z0-9./]+))?(?![\w-])(?=[\s"'`}]|$)/,
  },
  { name: 'variant="elevated"', re: /variant\s*=\s*\{?\s*["'`]elevated["'`]/ },
  {
    name: "a box-shadow or text-shadow property",
    re: /\b(?:box|text)-shadow\s*:/,
  },
  { name: "drop-shadow()", re: /drop-shadow\s*\(/ },
  {
    name: "a shadow or elevation custom property",
    re: /--(?:shadow|md-sys-elevation)[\w-]*/,
  },
  { name: "a boxShadow style key", re: /\b(?:boxShadow|textShadow)\b/ },
];

/** The first pattern a text trips, or `null`. */
export function violation(text: string): string | null {
  const stripped = stripComments(text);
  return PATTERNS.find((pattern) => pattern.re.test(stripped))?.name ?? null;
}

describe("this app asks for no shadow", () => {
  const sources = appSources(/\.(ts|tsx|css)$/).filter(
    (source) => !/src\/no-(dropdowns|shadows)\.test\.ts$/.test(source.path),
  );

  it("scans the whole source tree, stylesheet included — not an empty glob", () => {
    expect(sources.length).toBeGreaterThan(40);
    expect(sources.some((source) => source.path === "app/globals.css")).toBe(
      true,
    );
    expect(sources.some((source) => source.path === "app/login/page.tsx")).toBe(
      true,
    );
  });

  it("finds none", () => {
    const found = sources.flatMap((source) => {
      const name = violation(source.text);
      return name === null ? [] : [`${source.path}: ${name}`];
    });
    expect(found).toEqual([]);
  });
});

describe("the scan itself", () => {
  it.each<[string, string]>([
    ["shadow", '<div className="shadow" />'],
    ["shadow-md", '<div className="rounded shadow-md" />'],
    ["a variant-prefixed shadow", '<div className="hover:shadow-lg" />'],
    ["an arbitrary shadow", '<div className="shadow-[0_1px_2px_#000]" />'],
    ["drop-shadow-lg", '<img className="drop-shadow-lg" />'],
    ["text-shadow-sm", '<p className="text-shadow-sm" />'],
    ["inset-shadow-sm", '<p className="inset-shadow-sm" />'],
    ["a shadow at the end of a string", 'const c = "flex shadow";'],
    ["an elevated variant", '<Button variant="elevated">x</Button>'],
    ["an elevated variant in braces", "<Card variant={'elevated'} />"],
    ["box-shadow", ".a { box-shadow: 0 0 1px red; }"],
    ["text-shadow", ".a { text-shadow: 0 0 1px red; }"],
    ["a drop-shadow filter", ".a { filter: drop-shadow(0 0 1px red); }"],
    ["a shadow custom property", ".a { color: var(--shadow-dialog); }"],
    [
      "an elevation custom property",
      ".a { x: var(--md-sys-elevation-level1); }",
    ],
    ["a boxShadow style key", "<div style={{ boxShadow: 'none' }} />"],
  ])("trips on %s", (_name, text) => {
    expect(violation(text)).not.toBeNull();
  });

  it.each<[string, string]>([
    ["a primary button", '<Button variant="primary">x</Button>'],
    ["a word in a comment", "// no shadow here\nconst x = 1;"],
    [
      "a word in a block comment",
      '/* variant="elevated" is banned */ const x = 1;',
    ],
    ["the word shadowed", 'const shadowed = "a shadowed name";'],
    [
      "a class that merely contains the letters",
      '<div className="text-shadowless" />',
    ],
    ["an identifier", "const hasShadowRoot = false;"],
  ])("does not trip on %s", (_name, text) => {
    expect(violation(text)).toBeNull();
  });
});
