/**
 * Node-side (Cypress `setupNodeEvents`) task for the `color-contrast` checks
 * in `shop-hosted.cy.ts` — issue #73, plan §7 row 6.
 *
 * `cy.checkA11y`'s own failure message is just a count
 * ("1 accessibility violation was detected"); axe-core's per-node `data`
 * carries the measured ratio, the required one, and the two colours it
 * compared, and `Cypress.log`'s `consoleProps` only reaches the Cypress
 * runner's UI, not a headless `cypress run`'s terminal output. This task
 * prints that data with `console.log` in the Node process instead, so a CI
 * log names the exact ratio and element pair a violation was measured
 * against, without recolouring anything to find out.
 */
import type { Result as AxeResult } from "axe-core";

/** The subset of axe-core's `color-contrast` check data this task reads. */
interface ContrastCheckData {
  fgColor?: string;
  bgColor?: string;
  contrastRatio?: number;
  expectedContrastRatio?: string;
  fontSize?: string;
  fontWeight?: string;
}

export function logA11yViolations(violations: AxeResult[]): null {
  for (const violation of violations) {
    console.log(
      `\na11y violation: ${violation.id} (impact: ${violation.impact ?? "unknown"}) — ${violation.help}`,
    );
    for (const node of violation.nodes) {
      console.log(`  element: ${node.target.join(" ")}`);
      const checks = [...node.any, ...node.all, ...node.none];
      for (const check of checks) {
        const data = check.data as ContrastCheckData | null;
        if (data && typeof data.contrastRatio === "number") {
          console.log(
            `    measured ${data.contrastRatio}:1, needs ${data.expectedContrastRatio ?? "?"} ` +
              `— foreground ${data.fgColor ?? "?"} on background ${data.bgColor ?? "?"} ` +
              `(${data.fontSize ?? "?"}, ${data.fontWeight ?? "?"})`,
          );
        }
      }
      if (node.failureSummary) {
        console.log(`    ${node.failureSummary}`);
      }
    }
  }
  return null;
}
