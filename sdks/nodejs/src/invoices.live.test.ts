/**
 * The invoice surface against a **real** `vpay-server`, over a socket.
 *
 * Every other case in this package answers itself: the server is
 * `src/testing/test-server.ts`, which returns whatever the case told it to,
 * so "the stub answers the way this SDK expects" is the whole of the
 * evidence. That was recorded as a dated ⛔/⛔ row in
 * `docs/sdks/parity.md` on 2026-09-07 — *invoices exercised against a running
 * vpay* — precisely so that building thirteen methods against stubs would not
 * close it by accident. This file is what closes it, with
 * `sdks/rust/tests/live_invoices.rs` as its twin.
 *
 * # It never skips
 *
 * `pnpm test` does not collect this file — `vitest.config.ts` excludes the
 * glob and `vitest.live.config.ts` is the only thing that includes it — and
 * `pnpm test:live` fails, rather than skipping, when there is no stack: the
 * `globalSetup` in `src/testing/live-preflight.ts` throws on a missing
 * variable, an unanswered `/healthz` or a handshake the stack refuses. There
 * is no `it.skip` and no `if (!process.env…) return` anywhere below, because
 * a green run must never overstate coverage (AGENTS.md rule 2).
 *
 * # Running it
 *
 * ```text
 * just sdk-live
 * ```
 */
import { beforeAll, describe, expect, it } from "vitest";

import { VpayClient } from "./client.js";
import { readLiveEnv } from "./testing/live-env.js";
import type { Customer } from "./types.js";

let client: VpayClient;
let customer: Customer;

beforeAll(async () => {
  const env = readLiveEnv();
  client = new VpayClient({
    baseUrl: env.baseUrl,
    clientId: env.clientId,
    privateKey: env.privateKey,
  });
  customer = await client.customers.create({ phone: "+237670000001" });
});

describe("invoices against a running vpay", () => {
  /**
   * **create → two lines → finalize → retrieve → void.**
   *
   * One case for the sequence rather than five, because the states are
   * sequential: an invoice must be a draft to take a line and open to be
   * voided, so five independent cases would each rebuild the ones before.
   *
   * What it pins that the stub could not: a draft really has no number and
   * zero amounts; the lines the server stores add up to the `amount_due` it
   * computes (`quantity * unit_amount`, summed — a database expression this
   * SDK never sees); `retrieve` really carries the lines expanded; and a
   * voided invoice **keeps** its number.
   */
  it("creates a draft, bills two lines, finalizes, reads back and voids", async () => {
    const draft = await client.invoices.create({
      customer: customer.id,
      currency: "XAF",
      description: "exp33 review — live run",
      metadata: { order_id: "exp33-node" },
    });
    expect(draft.status).toBe("draft");
    expect(draft.number).toBeNull();
    expect(draft.amount_due).toBe(0);
    // Upper-case in, lower-case on the wire.
    expect(draft.currency).toBe("xaf");

    for (const [description, quantity, unitAmount] of [
      ["Consulting", 2, 15_000],
      ["Delivery", 1, 2_500],
    ] as const) {
      const line = await client.invoiceItems.create({
        invoice: draft.id,
        description,
        quantity,
        unit_amount: unitAmount,
      });
      // `amount` is computed by the database and never sent.
      expect(line.amount).toBe(quantity * unitAmount);
      expect(line.currency).toBe(draft.currency);
      expect(line.object).toBe("line_item");
    }

    const open = await client.invoices.finalize(draft.id);
    expect(open.status).toBe("open");
    expect(open.number).toEqual(expect.any(String));
    expect(open.amount_due).toBe(32_500);
    expect(open.amount_remaining).toBe(32_500);
    expect(open.amount_paid).toBe(0);

    const read = await client.invoices.retrieve(draft.id);
    expect(read.lines.data).toHaveLength(2);
    const summed = read.lines.data.reduce((total, line) => total + line.amount, 0);
    expect(summed).toBe(read.amount_due);
    expect(read.number).toBe(open.number);

    const page = await client.invoices.list({
      customer: customer.id,
      status: "open",
    });
    expect(page.data.map((invoice) => invoice.id)).toContain(read.id);

    const voided = await client.invoices.void(draft.id);
    expect(voided.status).toBe("void");
    // A voided invoice keeps its number: a hole is what an accountant reads
    // as a destroyed document.
    expect(voided.number).toBe(open.number);
  });

  /**
   * **create → finalize → pay**, and `pay` charges nobody.
   *
   * Stripe's `pay` charges a stored payment method; vpay has none on this
   * market, so it mints a payment intent and a hosted page and leaves the
   * invoice `open` until a settlement moves it.
   */
  it("finalizes a second invoice and mints a payment intent and a hosted url", async () => {
    const draft = await client.invoices.create({
      customer: customer.id,
      currency: "xaf",
    });
    await client.invoiceItems.create({
      invoice: draft.id,
      description: "One thing",
      unit_amount: 5_000,
    });
    const open = await client.invoices.finalize(draft.id);
    expect(open.number).toEqual(expect.any(String));

    const paying = await client.invoices.pay(draft.id, {
      success_url: "https://merchant.example/ok",
      cancel_url: "https://merchant.example/no",
    });
    expect(paying.status).toBe("open");
    expect(paying.payment_intent).toEqual(expect.any(String));
    expect(paying.hosted_invoice_url).toMatch(/^https?:\/\//);
  });

  /**
   * **`currency` is required**, which is the claim both SDKs documented the
   * other way round until 2026-09-08 — "omitted from the body entirely when
   * absent, so the server applies this deployment's own default". There is no
   * such default, and no stub answering `201` to anything could have said so.
   *
   * `CreateInvoiceParams` cannot express the omission any more, which is the
   * fix; the empty string is the shape the type still permits, and the server
   * refuses it by the same name.
   */
  it("refuses an invoice create with no usable currency, naming the parameter", async () => {
    await expect(
      client.invoices.create({ customer: customer.id, currency: "" }),
    ).rejects.toThrow(/currency/);
  });
});
