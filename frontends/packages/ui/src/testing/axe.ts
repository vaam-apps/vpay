import axe from 'axe-core';

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
  'label',
  'button-name',
  'link-name',
  'aria-required-attr',
  'aria-required-children',
  'aria-required-parent',
  'aria-roles',
  'aria-valid-attr',
  'aria-valid-attr-value',
  'aria-command-name',
  'region',
  'list',
  'listitem',
  'duplicate-id',
  'duplicate-id-aria',
];

/**
 * Runs axe-core's structural rules against a rendered subtree and returns
 * its violations. Callers assert `violations` is empty — asserting on the
 * array itself, not on a boolean, so a failure prints which rule and which
 * node, not just that something failed.
 */
export async function axeViolations(container: Element): Promise<axe.Result[]> {
  const results = await axe.run(container, { runOnly: { type: 'rule', values: STRUCTURAL_RULES } });
  return results.violations;
}
