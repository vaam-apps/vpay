/**
 * Node-side (Cypress `setupNodeEvents`) helpers for `hosted-invoice.cy.ts`:
 * mint a real invoice, finalize it, pay it into a hosted checkout session and
 * read it back — all through `@vaam-apps/vpay-sdk` against the demo stack,
 * with the `shop-merchant` credential `just gen-demo-keys` writes.
 *
 * They run in Node for `checkoutTasks.ts`'s own reason: an invoice is a
 * MERCHANT object, minted with the merchant's Ed25519 signing seed, and that
 * credential must never reach the page under test. The browser is given only
 * the `hosted_invoice_url` a merchant would e-mail a payer — the publishable
 * key and the session secret it carries, and nothing else (`docs/flows/
 * browser-checkout.md`).
 *
 * # Why a customer and a line, rather than a bare invoice
 *
 * `POST /v1/invoices` bills a required `customer`, and `finalize` refuses an
 * invoice with no line, so the shortest path to a real `hosted_invoice_url`
 * is the same four calls a merchant makes. Skipping either would test a state
 * the API does not let a merchant reach.
 *
 * # Why `success_url` and `cancel_url` are sent here
 *
 * The demo's `shop-merchant` registration configures no
 * `merchant_clients[].invoices` defaults, so `pay` must be given both or it
 * answers the `400` naming them (`docs/flows/invoices.md` § Paying). They
 * point at the shop, which is the merchant whose bill this is.
 */
import type { Invoice } from "@vaam-apps/vpay-sdk";

import { shopMerchantClient } from "./checkoutTasks.js";

/** What `hosted-invoice.cy.ts` needs to open the payer link. */
export interface MintedHostedInvoice {
  /** The `in_…`, so the spec can read the invoice back after paying. */
  id: string;
  /** The human number the checkout page is expected to render (issue #91, D6). */
  number: string;
  /** The link a payer follows — vpay's own hosted checkout page. */
  hostedInvoiceUrl: string;
}

/** The invoice fields `hosted-invoice.cy.ts` asserts after the charge settles. */
export interface ReadInvoice {
  status: Invoice["status"];
  amountPaid: number;
  amountRemaining: number;
  paymentIntent: string | null;
}

/**
 * A finalized 5,000 XAF invoice with a live hosted checkout session, minted
 * exactly as a merchant would.
 *
 * A random Cameroon mobile for the customer: `customers.create` needs one
 * identifier and nothing makes a phone unique per merchant, but a fixed one
 * would accumulate identical rows across a retried run and make a failure
 * harder to read in the shop's own database.
 */
export async function mintHostedInvoice(): Promise<MintedHostedInvoice> {
  const { vpay } = shopMerchantClient();
  const shopUrl = process.env["VPAY_SHOP_URL"] ?? "http://localhost:3001";
  const mobile = `+23767${Math.floor(1_000_000 + Math.random() * 8_999_999)}`;

  const customer = await vpay.customers.create({ phone: mobile });
  const draft = await vpay.invoices.create({
    customer: customer.id,
    currency: "XAF",
    description: "hosted-invoice.cy.ts",
  });
  await vpay.invoiceItems.create({
    invoice: draft.id,
    description: "A bill a payer follows a link to",
    quantity: 1,
    unit_amount: 5000,
  });
  await vpay.invoices.finalize(draft.id);

  const paying = await vpay.invoices.pay(draft.id, {
    success_url: `${shopUrl}/`,
    cancel_url: `${shopUrl}/cart`,
  });

  const { number, hosted_invoice_url: hostedInvoiceUrl } = paying;
  if (typeof number !== "string" || number.length === 0) {
    throw new Error(
      "hosted-invoice.cy.ts: a finalized invoice has no number; the D6 line " +
        "the spec asserts on could not be rendered.",
    );
  }
  if (typeof hostedInvoiceUrl !== "string" || hostedInvoiceUrl.length === 0) {
    throw new Error(
      "hosted-invoice.cy.ts: `pay` answered no `hosted_invoice_url`, so there " +
        "is no payer link to follow.",
    );
  }

  return { id: draft.id, number, hostedInvoiceUrl };
}

/**
 * The invoice as the merchant reads it, after the payer has paid — the only
 * authority on whether money moved. The checkout page's `succeeded` screen is
 * a read of the intent's status; this is the invoice the settlement
 * transaction was supposed to flip in the same commit.
 */
export async function readInvoice(id: string): Promise<ReadInvoice> {
  const { vpay } = shopMerchantClient();
  const invoice = await vpay.invoices.retrieve(id);
  return {
    status: invoice.status,
    amountPaid: invoice.amount_paid,
    amountRemaining: invoice.amount_remaining,
    paymentIntent: invoice.payment_intent,
  };
}
