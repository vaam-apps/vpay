/**
 * `examples/shop` → vpay's HOSTED checkout page → back to the shop, in a real
 * browser, against the real compose stack (Step 9, lane 6).
 *
 * Nothing in this file stubs a rail. The MTN and Orange stubs are WireMock
 * hosts in `compose.yml` (ADR-0006), `vpay-worker` is the process that polls
 * them, and the only thing that ever marks an order `paid` is the shop's own
 * webhook handler writing its own database after verifying vpay's signature.
 * Every assertion below about money having moved is an assertion about a page
 * that reads that database (`OrderPoller` → `orders.get`) and nothing else —
 * which is why a spec here cannot pass while `vpay-worker` is down.
 *
 * The MSISDNs are the DIGITS-ONLY steering numbers added in Step 9 lane 2b
 * (`backends/tests/conformance/wiremock/mtn/mappings/`): vpay's page validates
 * Cameroon E.164 and refuses the older hex-suffixed twins (`237600000ce0` and
 * friends), which is correct — a form that accepted letters as a phone number
 * would accept them for every payer. See `docs/plans/step9-notes/lane-3.md` §4c.
 *
 * Cross-origin: the shop, vpay's page and the Orange stub are three different
 * ports, and Cypress treats a differing port as a differing origin
 * (`getSuperDomainOrigin` = protocol + superdomain + port), so each leg on a
 * foreign origin is inside its own `cy.origin()` with everything it needs
 * passed through `args`.
 */

import {
  MTN,
  PRODUCT,
  checkoutOrigin,
  orangeOrigin,
  orderIdFromUrl,
  readOrder,
  shopUrl,
  waitForOrderStatus,
} from "../support/shop";

/** Adds one product and submits the checkout form, leaving the browser on vpay's page. */
function buyOnVpaysPage(): void {
  cy.visit(shopUrl());
  cy.get(`[data-testid="add-${PRODUCT.tote}"]`).click();

  cy.visit(`${shopUrl()}/cart`);
  cy.get('[data-testid="cart-table"]').should("be.visible");

  cy.get('[data-testid="to-checkout"]').click();
  // Still typed, though the field became OPTIONAL on 2026-09-06 (exp22): a
  // payer who gives one is the case worth driving here, and `checkout.cy.ts`
  // is not the place to cover the empty one — `orders.test.ts` does, at the
  // level where the stored value can actually be asserted.
  cy.get('[data-testid="email"]').type("payer@example.test");
  // The surface is chosen explicitly rather than relied on. `hosted` is the
  // default (`SHOP_CHECKOUT_MODE`, unset in the demo stack) and this radio
  // starts selected; checking it anyway means this spec keeps driving the
  // redirect the day that default changes, instead of quietly testing
  // whatever the deployment happens to prefer.
  cy.get('[data-testid="mode-hosted"]').check();
  // The shop's server now creates the PaymentIntent and the hosted Checkout
  // Session through `@vaam-apps/vpay-sdk` and answers with `session.url`; the browser
  // performs a top-level navigation to vpay's origin.
  cy.get('[data-testid="pay"]').click();
}

describe("the shop, paid on vpay's hosted page", () => {
  it("MTN push: the payer pays on vpay's page and the shop reaches `paid` through the webhook", () => {
    buyOnVpaysPage();

    cy.origin(
      checkoutOrigin(),
      { args: { msisdn: MTN.succeeds } },
      ({ msisdn }) => {
        // The rail selector, because the shop's intent offers both rails and
        // vpay's page can drive both (D9). Its presence is already a fact
        // about the intent: the page renders what `payment_method_types`
        // says, never a hard-coded list.
        cy.get('[data-screen="select_rail"]', { timeout: 60_000 }).should(
          "be.visible",
        );
        cy.get('[data-testid="amount"]').should("contain.text", "12");
        cy.get('button[data-rail="orange_money"]').should("exist");
        cy.get('button[data-rail="mtn_momo"]').click();

        cy.get('[data-screen="collect_msisdn"]').should("be.visible");
        cy.get("#vpay-msisdn").type(msisdn);
        cy.get('button[type="submit"]').click();

        // `waiting` is the page polling `/v1/browser/payment_intents/{id}`
        // through `@vaam-apps/vpay-stripe-js`. What moves the intent underneath it is
        // `vpay-worker` polling MTN's stub — nothing in this spec pushes the
        // status forward.
        cy.get('[data-screen="waiting"]', { timeout: 60_000 }).should("exist");
        cy.get('[data-outcome="succeeded"]', { timeout: 120_000 }).should(
          "be.visible",
        );
        // The forward is the merchant's `success_url` with
        // `{CHECKOUT_SESSION_ID}` substituted (D5). Clicking rather than
        // waiting out the five-second countdown, so the navigation is this
        // spec's and not a race with it.
        cy.get('[data-outcome="succeeded"] button').click();
      },
    );

    // Back on the shop. This page reads the shop's database; it takes no
    // decision from the `session_id` vpay put in the query string.
    cy.url({ timeout: 60_000 }).should("include", "/return");
    cy.get('[data-testid="return-session-id"]').should("contain.text", "cs_");
    cy.get('[data-testid="paid-message"]', { timeout: 120_000 }).should(
      "be.visible",
    );
    cy.get('[data-testid="order-status"]').should("have.text", "Paid");
    cy.get('[data-testid="confirming"]').should("not.exist");

    orderIdFromUrl().then((orderId) => {
      readOrder(orderId).then((order) => {
        expect(order.status, "the shop's own orders.get").to.equal("paid");
        expect(order.paymentIntentId).to.match(/^pi_/);
        expect(order.checkoutSessionId).to.match(/^cs_/);
      });
    });
  });

  it("Orange redirect: the payer pays on the rail's own page and the shop reaches `paid`", () => {
    buyOnVpaysPage();

    cy.origin(checkoutOrigin(), () => {
      cy.get('[data-screen="select_rail"]', { timeout: 60_000 }).should(
        "be.visible",
      );
      cy.get('button[data-rail="orange_money"]').click();
      // No form: a redirect rail collects nothing here.
      cy.get('[data-screen="ready_redirect"]').should("be.visible");
      cy.get("button.btn-primary").click();
      // The confirm answers `next_action.redirect_to_url` and the page
      // navigates top-level to the rail. `cy.origin` for the rail follows.
    });

    // Orange's own hosted page — a WireMock mapping (D7), not a vpay page.
    // Its two links are the `return_url` and `cancel_url` THAT submit
    // carried, which since Step 9 lane 2 is vpay's own return page for this
    // session, carrying the `t=` return token.
    cy.origin(orangeOrigin(), () => {
      cy.get("#pay", { timeout: 60_000 }).should("be.visible");
      cy.get("#cancel").should("exist");
      cy.get("#pay").click();
    });

    // vpay's return page: it holds the return token, not the intent's
    // secret, and it polls until the rail's status query settles.
    cy.origin(checkoutOrigin(), () => {
      cy.url({ timeout: 60_000 }).should("include", "/return");
      cy.get('[data-outcome="succeeded"]', { timeout: 120_000 }).should(
        "be.visible",
      );
      cy.get('[data-outcome="succeeded"] button').click();
    });

    cy.url({ timeout: 60_000 }).should("include", "/return");
    cy.get('[data-testid="paid-message"]', { timeout: 120_000 }).should(
      "be.visible",
    );
    cy.get('[data-testid="order-status"]').should("have.text", "Paid");

    orderIdFromUrl().then((orderId) => {
      readOrder(orderId).then((order) => {
        expect(order.status).to.equal("paid");
      });
    });
  });

  it("a payment that does not succeed lands on the shop's cancel_url, and the order never becomes `paid`", () => {
    // WHY A DECLINE AND NOT THE RAIL PAGE'S "Cancel" LINK. The plan's third
    // hosted case is spelled '"Cancel" → the shop's cancelled page with the
    // order still unpaid'. Two facts of the code as merged make the literal
    // reading unreachable, and neither is this spec's to change:
    //
    //  1. vpay's hosted page has no cancel control. Its only exits are the
    //     outcome screen's forward and closing the tab.
    //  2. The Orange stub's "Cancel" link is the `cancel_url` THAT SUBMIT
    //     carried, and `vpay_adapter_orange_money` sends the charge's single
    //     `return_url` as both `return_url` and `cancel_url` — so the link
    //     goes to vpay's return page, and the stub's `transactionstatus`
    //     mapping answers SUCCESS for any order_id it is not steered on. A
    //     payer who "cancels" on the stub is therefore paid anyway.
    //
    // What actually reaches `cancel_url` is `forwardKindFor(session, paid)`
    // with `paid === false`: a hosted session sends every non-success there.
    // That is the property worth proving, and this is how it is reachable.
    //
    // The order does not stay `unpaid` either: vpay emits
    // `payment_intent.payment_failed`, the shop's webhook handler maps it to
    // `failed`, and asserting `unpaid` would be asserting the shop's webhook
    // did nothing. What is asserted is what matters — it is never `paid`, and
    // the cancelled page itself writes nothing.
    buyOnVpaysPage();

    cy.origin(
      checkoutOrigin(),
      { args: { msisdn: MTN.insufficientFunds } },
      ({ msisdn }) => {
        cy.get('[data-screen="select_rail"]', { timeout: 60_000 }).should(
          "be.visible",
        );
        cy.get('button[data-rail="mtn_momo"]').click();
        cy.get("#vpay-msisdn").type(msisdn);
        cy.get('button[type="submit"]').click();

        cy.get('[data-outcome="failed"]', { timeout: 120_000 }).should(
          "be.visible",
        );
        cy.get('[data-testid="outcome-body"]').should("not.be.empty");
        cy.get('[data-outcome="failed"] button').click();
      },
    );

    cy.url({ timeout: 60_000 }).should("include", "/cancelled");
    cy.get('[data-testid="cancelled-message"]').should("be.visible");
    cy.get('[data-testid="order-status"]').should("not.have.text", "Paid");

    orderIdFromUrl().then((orderId) => {
      // The shop's own record. `failed` because the signed
      // `payment_intent.payment_failed` was delivered and verified — the
      // cancel URL is a navigation, not an authority. `waitForOrderStatus`
      // fails the moment it ever reads `paid`.
      waitForOrderStatus(orderId, "failed");
    });
  });
});
