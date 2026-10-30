/**
 * The things `dashboard-human-login.cy.ts` needs from Node, wrapped so the spec
 * reads as what it is doing rather than as `cy.task` plumbing, plus the one
 * accessibility pass it runs on each sign-in screen.
 */

import type axeCore from "axe-core";

/**
 * `window.axe`, set by `win.eval(axeSource)` in {@link expectNoWcagViolations}.
 * The same declaration `shop-hosted.cy.ts` makes, and for the same reason:
 * axe-core is evaluated into the page under test, never imported as a module,
 * so it is only typed here.
 */
declare global {
  interface Window {
    axe: typeof axeCore;
  }
}

/**
 * A current RFC 6238 code for `secret`.
 *
 * Computed in Node (`cypress/tasks/dashboardTasks.ts`) because it needs a
 * keyed hash, and from the secret the **enrolment screen displayed** because
 * that is the thing under test: a code derived from anything else would prove
 * only that this spec agrees with itself.
 */
export function totpDigits(secret: string): Cypress.Chainable<string> {
  return cy.task<string>("totpCode", { secret });
}

/**
 * Waits until the current 30-second TOTP step has passed.
 *
 * vpay's replay guard is a compare-and-swap on `staff_members.last_totp_step`
 * that admits only a **strictly greater** step, so a second sign-in inside
 * the same 30 seconds cannot present a code it would accept. Without this
 * wait the second sign-in is refused for exactly the right reason, with a
 * message — one sentence, by design — that looks identical to a wrong code.
 *
 * A real wait rather than a clock stub: the code has to be one the server's
 * own clock accepts, and `cy.clock` moves the browser's.
 */
export function waitForNextTotpStep(): void {
  cy.task<number>("secondsLeftInStep").then((seconds) => {
    // Plus a second, so the boundary is crossed rather than landed on.
    cy.wait((seconds + 1) * 1000);
  });
}

/**
 * Leaves a value for a LATER test in this spec, in Node.
 *
 * `Cypress.env(key, value)` is the obvious way to do this and it is the wrong
 * one here: it does not survive the primary origin moving, which this spec
 * does every run as soon as it buys something in the shop. The measurement,
 * and why Node is where the value has to live, are in
 * `cypress/tasks/dashboardTasks.ts`.
 */
export function carryForward(key: string, value: string): void {
  cy.task("carryForward", { key, value });
}

/**
 * Reads back what {@link carryForward} stored, failing with the key's name
 * if the test that was supposed to store it did not.
 */
export function carriedForward(key: string): Cypress.Chainable<string> {
  return cy.task<string | null>("carriedForward", key).then((value) => {
    expect(
      value,
      `'${key}', which an earlier test in this spec should have stored`,
    ).to.be.a("string");
    return String(value);
  });
}

/**
 * Runs axe-core on the page under test and fails on any WCAG 2.0/2.1 A or AA
 * violation **other than `color-contrast`**.
 *
 * # Why this is narrower than it looks
 *
 * * **Tagged rules only** (`wcag2a`, `wcag2aa`, `wcag21a`, `wcag21aa`), not
 *   axe's best-practice set. The best-practice rules (`page-has-heading-one`,
 *   `region`, `heading-order`) are advice about a page's shape and some of them
 *   depend on the shell around it; `src/a11y.test.tsx` in the dashboard already
 *   owns the shape, per screen, in a unit gate. What a real browser can add to
 *   that is a failure of an actual WCAG criterion, and that is what this asks
 *   for.
 * * **No `color-contrast`.** It cannot reach a verdict on a daisyUI 5 page
 *   (`shop-hosted.cy.ts`'s header measured it: every element is `incomplete`,
 *   never a violation, because of the `:root` scroll-lock `background-image`),
 *   so enabling it here would assert nothing while looking like it did. Contrast
 *   is measured by the dashboard's Storybook run
 *   (`pnpm --filter @vpay/dashboard test-storybook`) and
 *   `payment-status-contrast.test.ts`.
 *
 * It runs on the dashboard's own origin, so unlike the checkout's checks it
 * needs no `cy.origin()`; `cy.readFile` is allowed here.
 *
 * **Not yet seen to run.** This was written without a browser, and the first
 * red `e2e (compose)` run is the first time axe meets these pages in one. A
 * violation prints through the `logA11yViolations` task with the element and
 * the criterion.
 *
 * @param label Which screen, for the failure message.
 */
export function expectNoWcagViolations(label: string): void {
  cy.readFile("node_modules/axe-core/axe.min.js", { log: false }).then(
    (axeSource: string) => {
      // The promise from `axe.run` is RETURNED, and no `cy` command is issued
      // inside it: Cypress refuses a callback that does both. The assertions
      // are the next link in the chain, on the yielded result.
      cy.window({ log: false })
        .then((win) => {
          win.eval(axeSource);
          return win.axe.run(win.document, {
            runOnly: {
              type: "tag",
              values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"],
            },
            rules: { "color-contrast": { enabled: false } },
          });
        })
        .then((result) => {
          if (result.violations.length > 0) {
            cy.task("logA11yViolations", result.violations);
          }
          expect(
            result.violations.map((violation) => violation.id),
            `${label}: WCAG A/AA violations axe-core detected`,
          ).to.deep.equal([]);
        });
    },
  );
}
