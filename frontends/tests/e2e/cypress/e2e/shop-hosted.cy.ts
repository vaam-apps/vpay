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
        // `{CHECKOUT_SESSION_ID}` substituted (D5). The click is the ONLY way
        // off this screen: the five-second auto-forward this comment used to
        // describe was removed on 2026-09-06 and nothing replaced it, so a
        // spec that merely waited here would wait forever.
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
      // `data-testid`, not `button.btn-primary`: exp26 (2026-09-07) made
      // `Button` a `@vpay/ui` component whose primary look is a `cva`
      // variant default rather than a class this spec should know the name
      // of — see docs/plans/2026-09-07-ui-revamp.md §4.1.
      cy.get('[data-testid="continue"]').click();
      // The confirm answers `next_action.redirect_to_url` and the page
      // navigates top-level to the rail. `cy.origin` for the rail follows.
    });

    // Orange's own hosted page — a WireMock mapping (D7), not a vpay page.
    // Its two links `302` to the `return_url` and `cancel_url` THAT submit
    // carried, which since Step 9 lane 2 is vpay's own return page for this
    // session, carrying the `t=` return token.
    //
    // **This click now decides the charge, and until 2026-09-10 it decided
    // nothing** (vpay issue #58). The stub used to answer the very first
    // `transactionstatus` `SUCCESS`, about 449 ms after the submit, so this
    // charge was already `paid` by the time the page below had rendered —
    // the spec passed, and it was passing for a reason that had nothing to
    // do with the payer. The stub now answers `PENDING` while a payer is on
    // the page, and the two controls point back at the stub itself so it can
    // tell a payer who paid from one who wandered off. See
    // `backends/tests/conformance/wiremock/orange/mappings/stub-hosted-page.json`.
    cy.origin(orangeOrigin(), () => {
      cy.get("#pay", { timeout: 60_000 }).should("be.visible");
      // Both controls go back through the rail's own container. A link
      // straight to the merchant is a payer the rail never hears from
      // again, which is the shape this had before and the reason its test
      // numbers did not work from a browser.
      cy.get("#pay")
        .invoke("attr", "href")
        .should("match", /^\/stub-hosted-page\/[^/]+\/pay\?/u);
      cy.get("#cancel")
        .invoke("attr", "href")
        .should("match", /^\/stub-hosted-page\/[^/]+\/cancel\?/u);
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

  it("Orange redirect: the payer cancels on the rail's own page and the order reaches `failed`", () => {
    // **The other half of vpay issue #58, and the one that could not be
    // written at all before 2026-09-10.**
    //
    // Until then the rail stub's `#cancel` link pointed straight at the
    // `cancel_url` the submit carried, so a payer who clicked it never
    // touched the stub again — and the stub, asked for a status it had not
    // been steered on, answered `SUCCESS`. A payer who cancelled was paid.
    // The comment on the case above says so, because it was true.
    //
    // The link now goes to `/stub-hosted-page/{token}/cancel` on the rail's
    // own container, which arms `EXPIRED` and *then* `302`s to the same URL.
    // `EXPIRED` and not a `CANCELLED` of this repository's invention: Orange
    // documents five statuses and that is not one of them
    // (`docs/flows/adapter-orange-money.md`), so a payer who abandons the
    // page is a page that ended unpaid, which is `payer_timeout`.
    //
    // Nothing here stubs anything. The order moves because `vpay-worker`
    // asked the rail, the rail said `EXPIRED`, vpay emitted
    // `payment_intent.payment_failed`, and this shop's webhook handler
    // verified the signature and wrote its own database.
    buyOnVpaysPage();

    cy.origin(checkoutOrigin(), () => {
      cy.get('[data-screen="select_rail"]', { timeout: 60_000 }).should(
        "be.visible",
      );
      cy.get('button[data-rail="orange_money"]').click();
      cy.get('[data-screen="ready_redirect"]').should("be.visible");
      cy.get('[data-testid="continue"]').click();
    });

    cy.origin(orangeOrigin(), () => {
      cy.get("#cancel", { timeout: 60_000 }).should("be.visible");
      cy.get("#cancel").click();
    });

    // `cancel_url` and `return_url` are the same value on this rail, so the
    // payer lands back on vpay's return page either way and the page polls
    // until the rail's status query settles. What differs from the case
    // above is the outcome it settles on.
    cy.origin(checkoutOrigin(), () => {
      cy.url({ timeout: 60_000 }).should("include", "/return");
      cy.get('[data-outcome="failed"]', { timeout: 120_000 }).should(
        "be.visible",
      );
      cy.get('[data-testid="outcome-body"]').should("not.be.empty");
      cy.get('[data-outcome="failed"] button').click();
    });

    // A hosted session sends every non-success to the merchant's
    // `cancel_url` (`forwardKindFor(session, paid)` with `paid === false`).
    cy.url({ timeout: 60_000 }).should("include", "/cancelled");
    cy.get('[data-testid="cancelled-message"]').should("be.visible");

    orderIdFromUrl().then((orderId) => {
      // `waitForOrderStatus` fails the moment it ever reads `paid` — which
      // is precisely what this spec would have read before the stub grew a
      // payer's window, and is the assertion that makes this case a gate on
      // the fix rather than a description of it.
      waitForOrderStatus(orderId, "failed");
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
    //  2. The Orange stub's "Cancel" link went straight to the `cancel_url`
    //     THAT SUBMIT carried, and `vpay_adapter_orange_money` sends the
    //     charge's single `return_url` as both — so the link went to vpay's
    //     return page, the stub never heard about the click, and its
    //     `transactionstatus` mapping answered SUCCESS for any order_id it
    //     was not steered on. A payer who "cancelled" on the stub was paid
    //     anyway.
    //
    //     **The second one stopped being true on 2026-09-10** (vpay issue
    //     #58): the stub's Cancel link now goes through the stub, which arms
    //     `EXPIRED` before forwarding, and the case below this one drives
    //     exactly that. This case stays as it is — it is the MTN half of the
    //     same property, and a decline at the rail and an abandonment at the
    //     rail's page are two different things arriving at one `cancel_url`.
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
