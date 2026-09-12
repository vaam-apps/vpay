import axe from "axe-core";

/**
 * Structural accessibility checks only — plan §7 row 5, ported here
 * verbatim from `@vpay/ui/testing` (2026-09-12, the `@vaam-apps/ui`
 * cutover). `@vpay/ui` is being deleted in this same change, and
 * `screens.axe.test.tsx` was its one consumer outside the package's own
 * tests, so the helper moves to live beside the one file that uses it
 * rather than staying published from a package with no other reason to
 * exist.
 *
 * jsdom parses and applies declared CSS rules but computes no real layout or
 * paint, so a contrast check running here would either report nothing or
 * report against numbers that were never actually rendered — "a green jsdom
 * contrast run is not evidence" (plan §7 row 6). `color-contrast` and every
 * other visual-geometry rule (`target-size`, `scrollable-region-focusable`,
 * …) stay excluded here on purpose; contrast is measured for real in
 * `outcome-contrast.test.ts`, from the compiled theme, not from a browser.
 *
 * The rules below are exactly the ones plan §7 row 5 names, copied
 * verbatim from `frontends/packages/ui/src/testing/axe.ts` — every one is
 * a DOM-structure or ARIA-attribute check, not a rendered-pixel check.
 * Dropping even one of these would silently narrow the guarantee, which is
 * why the list is a single array both suites (the deleted package's own and
 * this one) import from the same place — except there is now only one
 * suite left, this one, since `@vpay/ui`'s own `axe.test.tsx` dies with the
 * package.
 */
const STRUCTURAL_RULES = [
  "label",
  "button-name",
  "link-name",
  "aria-required-attr",
  "aria-required-children",
  "aria-required-parent",
  "aria-roles",
  "aria-valid-attr",
  "aria-valid-attr-value",
  "aria-command-name",
  "region",
  "list",
  "listitem",
  "duplicate-id",
  "duplicate-id-aria",
];

/**
 * Runs axe-core's structural rules against a rendered subtree and returns
 * its violations. Callers assert `violations` is empty — asserting on the
 * array itself, not on a boolean, so a failure prints which rule and which
 * node, not just that something failed.
 */
export async function axeViolations(container: Element): Promise<axe.Result[]> {
  const results = await axe.run(container, {
    runOnly: { type: "rule", values: STRUCTURAL_RULES },
  });
  return results.violations;
}

/**
 * How many rules `axeViolations` actually evaluated against `container` —
 * passes, violations and incomplete results together.
 *
 * **Why this exists.** `axe.run` over a container that rendered nothing
 * (a `render()` that threw inside a `try`, a state fixture that silently
 * became `null`, a screen map that emptied) reports ZERO violations — which
 * looks identical to a clean screen. `screens.axe.test.tsx` asserts this is
 * nonzero *before* trusting an empty violation list, so the anti-vacuity
 * guarantee lives in an assertion the test file can see, not buried inside
 * this helper. Exported so that assertion can be written once and reused,
 * rather than recomputing `axe.run` a second time per test.
 */
export async function axeEvaluatedRuleCount(
  container: Element,
): Promise<number> {
  const results = await axe.run(container, {
    runOnly: { type: "rule", values: STRUCTURAL_RULES },
  });
  return (
    results.passes.length +
    results.violations.length +
    results.incomplete.length
  );
}

// `axeContrastViolations` (a jsdom `color-contrast` run) lived in the
// deleted package until issue #73 was answered for real. Removed
// 2026-09-11 (b1e review): jsdom parses no CSS and computes no layout, so
// the function could only ever return zero violations — a check that
// cannot fail is worse than none, because a reader trusts it. Contrast is
// `outcome-contrast.test.ts` now, against the real compiled theme, and
// `cy.checkA11y(…, { runOnly: ['color-contrast'] })` in
// `frontends/tests/e2e/cypress/e2e/shop-hosted.cy.ts` in a real browser.
