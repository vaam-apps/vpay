/**
 * Invoices, driven by the **official `stripe` package** against a **real vpay
 * stack** (issue #91, decision 1: "Stripe-compat gains the cases the real
 * stripe package can drive").
 *
 * Every case asserts the **body** the server answered, not only that the call
 * resolved: a route that answered `200` with the wrong object would satisfy
 * stripe-node's typings exactly as well as the right one, because stripe-node
 * parses whatever JSON it is handed.
 *
 * # What the real package can drive, and what it cannot
 *
 * `stripe.invoices` reaches every invoice route vpay mounts, cast-free except
 * where noted: `create`, `retrieve`, `update`, `del`, `list` (and its
 * auto-pagination), `finalizeInvoice`, `voidInvoice`, `markUncollectible`
 * and `pay`.
 *
 * `stripe.invoiceItems` reaches **none** of vpay's line routes, and that is a
 * path, not a parameter: stripe-node addresses `/v1/invoiceitems` and vpay
 * mounts `/v1/invoice_items` (`docs/flows/invoices.md` § The surface). The
 * last case pins it — the sealing client refuses the unknown operation before
 * anything is sent. A Stripe merchant adds a line through the same client's
 * `rawRequest`, which is what {@link addLine} does: one more accommodation a
 * merchant has to write, written down here rather than hidden.
 * `stripe.invoices.listLineItems` (`GET /v1/invoices/{id}/lines`) is likewise
 * not mounted — a merchant reads the lines off the invoice, which carries them
 * expanded.
 *
 * If the stack is not there, `src/preflight.ts` fails the run before this file
 * is loaded.
 */
import Stripe from "stripe";
import { beforeAll, describe, expect, it } from "vitest";

import {
  CURRENCY,
  caught,
  invalidRequest,
  payInvoiceHosted,
  stripeClient,
} from "./client.js";

const stripe = stripeClient();

/**
 * `{prefix}-{000001}`: eight upper-case Crockford characters, a dash, six
 * digits (`docs/flows/invoices.md` § Numbering).
 */
const INVOICE_NUMBER = /^[0-9A-HJKMNP-TV-Z]{8}-\d{6}$/;

/**
 * A key off a stripe-node object that stripe-node's types do not declare.
 *
 * `payment_intent`, `amount_refunded`, `paid_out_of_band` and a line's
 * `unit_amount` are on vpay's invoice and not on Stripe's current one (Stripe
 * moved an invoice's intent onto `payments` in 2025). Read through `unknown`,
 * which is exactly what a merchant on stripe-node has to do.
 */
function vpayField(object: object, key: string): unknown {
  return (object as Record<string, unknown>)[key];
}

/**
 * One line on a draft, through the **same client's** `rawRequest`.
 *
 * `stripe.invoiceItems.create` cannot do this: it posts to
 * `/v1/invoiceitems`, which vpay does not mount (see this file's header).
 * `rawRequest` keeps the merchant on one configured client — the same
 * sealing `httpClient`, the same idempotency key handling — and costs one
 * untyped answer, which is narrowed here and nowhere else.
 */
async function addLine(
  invoice: string,
  description: string,
  quantity: number,
  unitAmount: number,
): Promise<Record<string, unknown>> {
  const line: unknown = await stripe.rawRequest("POST", "/v1/invoice_items", {
    invoice,
    description,
    quantity,
    unit_amount: unitAmount,
  });
  if (typeof line !== "object" || line === null) {
    throw new Error(`POST /v1/invoice_items answered ${String(line)}`);
  }
  return line as Record<string, unknown>;
}

let customer: Stripe.Customer;

beforeAll(async () => {
  customer = await stripe.customers.create({ phone: "+237670000002" });
});

/** A draft for this file's customer, with one line of `quantity × unitAmount`. */
async function draftWithLine(
  quantity = 2,
  unitAmount = 2_500,
): Promise<Stripe.Invoice> {
  const draft = await stripe.invoices.create({
    customer: customer.id,
    currency: CURRENCY,
  });
  await addLine(draft.id, "Stripe-compat line", quantity, unitAmount);
  return draft;
}

describe("invoices through stripe.invoices", () => {
  /**
   * create → line → retrieve → update → list, all on a draft.
   *
   * Pins the draft's Stripe-shaped body as stripe-node hands it back: no
   * number, zero amounts until a line exists, the lines expanded as a Stripe
   * `list`, and `update` merging `metadata` key-wise rather than replacing it
   * — the semantics a Stripe merchant's code already assumes.
   */
  it("creates a draft, bills a line, reads it back, updates it and lists it", async () => {
    const draft = await stripe.invoices.create({
      customer: customer.id,
      currency: CURRENCY,
      description: "Stripe-compat draft",
      metadata: { order_id: "compat-1", channel: "stripe-node" },
    });
    expect(draft.object).toBe("invoice");
    expect(draft.id).toMatch(/^in_/);
    expect(draft.status).toBe("draft");
    expect(draft.number).toBeNull();
    expect(draft.customer).toBe(customer.id);
    expect(draft.currency).toBe(CURRENCY);
    expect(draft.amount_due).toBe(0);
    expect(draft.amount_paid).toBe(0);
    expect(draft.amount_remaining).toBe(0);
    expect(draft.description).toBe("Stripe-compat draft");
    expect(draft.metadata).toEqual({
      order_id: "compat-1",
      channel: "stripe-node",
    });
    expect(draft.hosted_invoice_url).toBeNull();
    expect(draft.livemode).toBe(false);
    expect(draft.lines.object).toBe("list");
    expect(draft.lines.data).toEqual([]);
    expect(draft.lines.has_more).toBe(false);
    expect(draft.status_transitions.finalized_at).toBeNull();
    expect(vpayField(draft, "payment_intent")).toBeNull();

    const line = await addLine(draft.id, "Consulting", 2, 2_500);
    expect(line["object"]).toBe("line_item");
    expect(line["id"]).toEqual(expect.any(String));
    expect(line["description"]).toBe("Consulting");
    expect(line["quantity"]).toBe(2);
    expect(line["unit_amount"]).toBe(2_500);
    // Computed by the database, never sent.
    expect(line["amount"]).toBe(5_000);
    expect(line["currency"]).toBe(CURRENCY);

    const read = await stripe.invoices.retrieve(draft.id);
    expect(read.amount_due).toBe(5_000);
    expect(read.amount_remaining).toBe(5_000);
    expect(read.lines.data).toHaveLength(1);
    const [first] = read.lines.data;
    expect(first?.id).toBe(line["id"]);
    expect(first?.amount).toBe(5_000);
    expect(first?.description).toBe("Consulting");

    const updated = await stripe.invoices.update(draft.id, {
      description: "Stripe-compat draft, revised",
      metadata: { channel: "stripe-node-22" },
    });
    expect(updated.id).toBe(draft.id);
    expect(updated.description).toBe("Stripe-compat draft, revised");
    // Merged key-wise: `order_id` survives an update that did not name it.
    expect(updated.metadata).toEqual({
      order_id: "compat-1",
      channel: "stripe-node-22",
    });
    expect(updated.amount_due).toBe(5_000);

    const page = await stripe.invoices.list({
      customer: customer.id,
      status: "draft",
    });
    expect(page.object).toBe("list");
    const listed = page.data.find((invoice) => invoice.id === draft.id);
    expect(listed?.status).toBe("draft");
    expect(listed?.amount_due).toBe(5_000);
    // Every row carries the customer the filter named, and nothing else.
    expect(page.data.every((invoice) => invoice.customer === customer.id)).toBe(
      true,
    );

    const all = await stripe.invoices
      .list({ customer: customer.id, limit: 1 })
      .autoPagingToArray({ limit: 100 });
    expect(all.map((invoice) => invoice.id)).toContain(draft.id);
  });

  /**
   * `del` on a draft answers Stripe's `DeletedInvoice` shape, and the id is
   * gone afterwards — the `404` a Stripe merchant's code already handles.
   */
  it("deletes a draft with Stripe's deleted shape, and a read after is a 404", async () => {
    const draft = await draftWithLine();
    const deleted = await stripe.invoices.del(draft.id);
    expect(deleted).toMatchObject({
      id: draft.id,
      object: "invoice",
      deleted: true,
    });

    const error = await invalidRequest(() =>
      stripe.invoices.retrieve(draft.id),
    );
    expect(error.statusCode).toBe(404);
    expect(error.code).toBe("resource_missing");
  });

  /**
   * finalize → pay out of band, **cast-free**: `paid_out_of_band` is Stripe's
   * own `pay` parameter, and vpay records it with the method `other`
   * (RFC-0004 § 6).
   */
  it("finalizes, assigns a number, and pays out of band through Stripe's own parameter", async () => {
    const draft = await draftWithLine(1, 7_500);

    const open = await stripe.invoices.finalizeInvoice(draft.id);
    expect(open.id).toBe(draft.id);
    expect(open.status).toBe("open");
    expect(open.number).toMatch(INVOICE_NUMBER);
    expect(open.amount_due).toBe(7_500);
    expect(open.amount_remaining).toBe(7_500);
    expect(open.amount_paid).toBe(0);
    expect(open.status_transitions.finalized_at).toEqual(expect.any(Number));
    expect(vpayField(open, "amount_refunded")).toBe(0);
    expect(vpayField(open, "paid_out_of_band")).toBe(false);

    const paid = await stripe.invoices.pay(draft.id, {
      paid_out_of_band: true,
    });
    expect(paid.status).toBe("paid");
    expect(paid.number).toBe(open.number);
    expect(paid.amount_paid).toBe(7_500);
    expect(paid.amount_remaining).toBe(0);
    expect(paid.status_transitions.paid_at).toEqual(expect.any(Number));
    expect(paid.hosted_invoice_url).toBeNull();
    expect(vpayField(paid, "payment_intent")).toBeNull();
    expect(vpayField(paid, "paid_out_of_band")).toBe(true);
    expect(vpayField(paid, "out_of_band_payment")).toMatchObject({
      method: "other",
    });

    // A paid invoice is terminal: a second payment is the 409 every
    // transition out of `paid` gets.
    const error = await caught(() =>
      stripe.invoices.pay(draft.id, { paid_out_of_band: true }),
    );
    expect(error).toBeInstanceOf(Stripe.errors.StripeAPIError);
    expect((error as Stripe.errors.StripeAPIError).statusCode).toBe(409);
  });

  /**
   * finalize → **hosted** `pay` → void once the intent is cancelled.
   *
   * vpay's `pay` mints a payment intent and a checkout session rather than
   * charging a stored method (vpay has none on this market), so it needs
   * `success_url` and `cancel_url`, which Stripe's `InvoicePayParams` does
   * not declare — the one cast this file needs, confined to
   * `payInvoiceHosted` in `client.ts`. The invoice stays `open` until a
   * settlement moves it, and cannot be voided while its intent is live.
   */
  it("pays through a hosted checkout, and voids once the intent is cancelled", async () => {
    const draft = await draftWithLine(1, 5_000);
    const open = await stripe.invoices.finalizeInvoice(draft.id);

    const paying = await payInvoiceHosted(stripe, draft.id, {
      success_url: "https://merchant.example/invoice-paid",
      cancel_url: "https://merchant.example/invoice-cancelled",
    });
    expect(paying.status).toBe("open");
    expect(paying.amount_remaining).toBe(5_000);
    expect(paying.hosted_invoice_url).toMatch(/^https?:\/\/.+\/c\/cs_/);
    const intentId = vpayField(paying, "payment_intent");
    expect(intentId).toEqual(expect.stringMatching(/^pi_/));

    // The intent is an ordinary one: for what is left, in the invoice's
    // currency, described by the invoice's number.
    const intent = await stripe.paymentIntents.retrieve(String(intentId));
    expect(intent.amount).toBe(5_000);
    expect(intent.currency).toBe(CURRENCY);
    expect(intent.description).toBe(`Invoice ${String(open.number)}`);

    const refused = await caught(() => stripe.invoices.voidInvoice(draft.id));
    expect(refused).toBeInstanceOf(Stripe.errors.StripeAPIError);
    const refusal = refused as Stripe.errors.StripeAPIError;
    expect(refusal.statusCode).toBe(409);
    expect(refusal.message).toContain(String(intentId));

    await stripe.paymentIntents.cancel(String(intentId));
    const voided = await stripe.invoices.voidInvoice(draft.id);
    expect(voided.status).toBe("void");
    // A voided invoice keeps its number: a hole is what an accountant reads
    // as a destroyed document.
    expect(voided.number).toBe(open.number);
    expect(voided.status_transitions.voided_at).toEqual(expect.any(Number));
  });

  it("writes an open invoice off with markUncollectible", async () => {
    const draft = await draftWithLine();
    const open = await stripe.invoices.finalizeInvoice(draft.id);

    const written = await stripe.invoices.markUncollectible(draft.id);
    expect(written.status).toBe("uncollectible");
    expect(written.number).toBe(open.number);
    expect(written.amount_remaining).toBe(open.amount_due);
    expect(written.status_transitions.marked_uncollectible_at).toEqual(
      expect.any(Number),
    );
  });

  /**
   * The gap, pinned: `stripe.invoiceItems` addresses `/v1/invoiceitems`, a
   * path vpay does not mount, so the sealing client refuses the operation
   * before anything is sent. If vpay ever mounts Stripe's spelling, this case
   * fails and the header above — and `docs/flows/stripe-sdk-compat.md` — must
   * change with it.
   */
  it("cannot reach vpay's line routes through stripe.invoiceItems", async () => {
    const draft = await stripe.invoices.create({
      customer: customer.id,
      currency: CURRENCY,
    });
    const error = await invalidRequest(() =>
      stripe.invoiceItems.create({
        invoice: draft.id,
        customer: customer.id,
        amount: 5_000,
        currency: CURRENCY,
        description: "never sent",
      }),
    );
    expect(error.statusCode).toBe(404);
    expect(error.code).toBe("unknown_operation");

    // Nothing reached the server: the draft is still empty.
    const read = await stripe.invoices.retrieve(draft.id);
    expect(read.lines.data).toEqual([]);
    expect(read.amount_due).toBe(0);
  });
});
