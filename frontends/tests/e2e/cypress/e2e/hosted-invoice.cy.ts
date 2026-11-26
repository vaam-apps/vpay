/**
 * An invoice's `hosted_invoice_url`, end to end against the real compose
 * stack: mint a finalized invoice as the `shop-merchant`, follow the payer
 * link `POST /v1/invoices/{id}/pay` answered with, pay it on vpay's own
 * hosted checkout page, and read the invoice back as `paid` (issue #91,
 * item 5).
 *
 * Nothing in this file stubs a rail: `237670000900` keys WireMock scenario
 * `mtn-e2e-poll` (priority 5) to answer `PENDING` on the first status query
 * and `SUCCESSFUL` on the next, and `vpay-worker` — running in the compose
 * stack, not stubbed — is what drives that poll and settles the charge. The
 * page only reports the intent's status; the invoice moves because the
 * settlement transaction flips it in the same commit the intent succeeds in
 * (`docs/flows/invoices.md` § When the intent succeeds).
 *
 * # What only this spec covers
 *
 * `checkout.cy.ts` drives a PaymentIntent minted directly; `shop-hosted.cy.ts`
 * drives sessions the shop created. Neither follows a `hosted_invoice_url`,
 * which is the one payer link whose intent a merchant process did not create
 * — `pay` mints it, and vpay (not the merchant) decides which rails it offers.
 *
 * # The bug this case was written against
 *
 * Until 2026-11-20 `POST /v1/invoices/{id}/pay` minted that intent with an
 * **empty** `payment_method_types`. The browser session read renders one
 * `RailSpec` per code in the list, so the page was handed an empty `rails`
 * member and refused with `no_supported_rail` before rendering a rail: the
 * D6 invoice number (`docs/flows/invoices.md` § Paying) could not be read off
 * a payment page, because a payment page never rendered. The intent now
 * names every enabled rail that settles the invoice's currency, and the
 * `select_rail` assertion below is what fails if that regresses.
 *
 * # Why the visit is direct
 *
 * The payer link is a different origin from the dashboard `baseUrl`, and this
 * spec never navigates back, so `cy.visit()` may visit it without
 * `cy.origin()` — Cypress's cross-origin rules, the same arrangement
 * `checkout.cy.ts` uses for its fixture. The page still runs its own
 * frame-refusal check; the ordinary run's `modifyObstructiveCode` rewrite is
 * what lets it (see `docs/flows/browser-checkout.md`).
 */
import type {
  MintedHostedInvoice,
  ReadInvoice,
} from "../tasks/invoiceTasks.js";

/**
 * The MTN number `checkout.cy.ts` and `shop-hosted.cy.ts` also drive: the
 * digits-only steering MSISDN `mtn-e2e-poll` settles. See `shop.ts`'s own
 * note for why the older hex-suffixed twins are refused by the page's
 * Cameroon E.164 validation.
 */
const MTN_E2E_POLL_MSISDN = "237670000900";

describe("an invoice's hosted_invoice_url", () => {
  it("follows the payer link to a paid charge and the invoice ends up `paid`", () => {
    cy.task("mintHostedInvoice").then((minted) => {
      const { id, number, hostedInvoiceUrl } = minted as MintedHostedInvoice;
      cy.log(`minted invoice ${id} (${number})`);

      cy.visit(hostedInvoiceUrl);

      // The invoice number (issue #91, D6) proves the link opened THIS
      // invoice's session, not merely a checkout of the same amount.
      cy.get('[data-testid="invoice-number"]', { timeout: 60_000 }).should(
        "contain.text",
        number,
      );

      // The rails the invoice's intent names. Before the 2026-11-20 fix the
      // page refused here with `no_supported_rail`, and this is the line
      // that fails if `intent_for` stops naming them again.
      cy.get('[data-screen="select_rail"]', { timeout: 60_000 }).should(
        "be.visible",
      );
      cy.get('button[data-rail="mtn_momo"]').click();

      cy.get("#vpay-msisdn").type(MTN_E2E_POLL_MSISDN);
      cy.get('button[type="submit"]').click();

      // `waiting` is the page polling the intent; `vpay-worker` polling MTN's
      // stub is what moves it, never this spec.
      cy.get('[data-screen="waiting"]', { timeout: 60_000 }).should("exist");
      cy.get('[data-outcome="succeeded"]', { timeout: 120_000 }).should(
        "be.visible",
      );

      // The merchant's own read: the only authority on whether the bill is
      // settled. The page's `succeeded` screen is a read of the intent's
      // status; this is the invoice the settlement transaction was supposed
      // to flip in the same commit.
      cy.task("readInvoice", id).then((invoice) => {
        const { status, amountPaid, amountRemaining, paymentIntent } =
          invoice as ReadInvoice;
        expect(
          status,
          "the invoice's status after the charge settled",
        ).to.equal("paid");
        expect(amountPaid, "the invoice's amount_paid").to.equal(5000);
        expect(amountRemaining, "the invoice's amount_remaining").to.equal(0);
        expect(
          paymentIntent,
          "the invoice names the intent that paid it",
        ).to.match(/^pi_/u);
      });
    });
  });
});
