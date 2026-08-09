/**
 * Dashboard end-to-end tests.
 *
 * These run against the real stack from `compose.e2e.yml`: real Next.js, real
 * vpay-server, real Postgres, with WireMock standing in for the payment rails.
 * Nothing is stubbed inside the browser.
 */
describe('dashboard', () => {
  it('loads and renders the design system smoke test', () => {
    cy.visit('/');
    cy.contains('h1', 'vpay dashboard').should('be.visible');
    cy.get('[data-status="succeeded"]').should('exist');
  });

  it('states plainly that it is a scaffold rather than showing empty tables', () => {
    cy.visit('/');
    cy.get('[role="status"]').should('contain.text', 'Scaffold');
  });

  it('renders every payment status exactly once', () => {
    cy.visit('/');
    for (const s of [
      'requires_payment_method',
      'requires_action',
      'processing',
      'succeeded',
      'canceled',
    ]) {
      cy.get(`[data-status="${s}"]`).should('have.length', 1);
    }
  });
});
