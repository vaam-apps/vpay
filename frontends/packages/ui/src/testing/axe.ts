import axe from "axe-core";

/**
 * Structural accessibility checks only — plan §7 row 5.
 *
 * jsdom parses and applies declared CSS rules but computes no real layout or
 * paint, so a contrast check running here would either report nothing or
 * report against numbers that were never actually rendered — "a green jsdom
 * contrast run is not evidence" (plan §7 row 6). `color-contrast` and every
 * other visual-geometry rule (`target-size`, `scrollable-region-focusable`,
 * …) stay excluded here on purpose; contrast is `cypress-axe` against a
 * real browser, plan §7 row 6, out of this package's own gate.
 *
 * The rules below are exactly the ones plan §7 row 5 names — every one is
 * a DOM-structure or ARIA-attribute check, not a rendered-pixel check.
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
 * Runs axe-core's `color-contrast` rule against a rendered subtree — issue #73.
 *
 * **CRITICAL LIMITATION: jsdom computes no styles from the built stylesheet.**
 * A violation that exists in the rendered page will NOT be reported here; a
 * violation reported here is impossible (jsdom would have had to compute the
 * contrast, which it cannot). "A green jsdom contrast run is not evidence"
 * (plan §7 row 6).
 *
 * This function documents the _requirement_ to check contrast on specific
 * component compositions (like the outcome screens under the bumblebee theme).
 * **The real verification is in a real browser** — `cypress-axe` running
 * against the `compose.e2e.yml` stack, plan §7 row 6. When that test is built,
 * this jsdom function can be deleted.
 */
export async function axeContrastViolations(
  container: Element,
): Promise<axe.Result[]> {
  const results = await axe.run(container, {
    runOnly: { type: "rule", values: ["color-contrast"] },
  });
  return results.violations;
}
