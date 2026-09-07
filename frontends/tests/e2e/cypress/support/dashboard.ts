/**
 * The two things `dashboard.cy.ts` needs from Node, wrapped so the spec reads
 * as what it is doing rather than as `cy.task` plumbing.
 */

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
