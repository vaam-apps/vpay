/**
 * The narrow port the shop's business logic talks to, and the only thing it
 * knows about persistence.
 *
 * It exists for one reason: the tRPC procedures and the webhook handler have
 * real rules (server-side totals, an idempotency key derived from the order
 * id, dedupe by vpay's event id) and those rules deserve tests that run in
 * `pnpm -r test` — a job with no Postgres and no Docker. A port with an
 * in-memory implementation under `src/testing/` gives them that.
 *
 * What it costs is stated plainly in the README and in
 * `docs/plans/step9-notes/lane-7.md`: the *Prisma* implementation of this
 * port has **no automated test at all**. It was verified by hand on
 * 2026-09-04 against the built image and a real Postgres — the run the
 * README records, step by step. `just demo` does not cover it either: that
 * walkthrough brings the shop container up, waits for its healthcheck and
 * prints its URL, and never places an order. Lane 6's Cypress specs have
 * merged and do click an order through in a real browser, but they assert on
 * the shop's pages and never on `PrismaShopStore` itself — so it still has no
 * unit or integration test of its own.
 *
 * `src/testing/` is never imported by anything under `src/app` or
 * `src/server` — `src/testing/no-runtime-imports.test.ts` fails if it is.
 */

/** Mirrors the `OrderStatus` enum in `schema.zmodel`. */
export type OrderStatus = "unpaid" | "paid" | "failed" | "cancelled";

export interface Product {
  id: string;
  name: string;
  description: string;
  /** Integer minor units (docs/flows/money.md). */
  priceMinor: number;
  /** Lowercase ISO 4217. */
  currency: string;
}

export interface OrderLine {
  productId: string;
  /** The product's name as it was when the order was placed. */
  name: string;
  quantity: number;
  /** The unit price as it was when the order was placed, in minor units. */
  unitMinor: number;
}

export interface Order {
  id: string;
  /**
   * `null` when the buyer did not give one. The shop asks for an e-mail so
   * it can send a receipt, and a receipt is not a condition of paying —
   * the identity on a mobile-money payment is the **phone number**, which
   * the payer gives the rail and never gives the shop (the customers
   * decision of 2026-09-05: phone-only customers are allowed).
   */
  email: string | null;
  status: OrderStatus;
  /** Integer minor units, computed server-side from the catalogue. */
  totalMinor: number;
  currency: string;
  paymentIntentId: string | null;
  checkoutSessionId: string | null;
  /**
   * `last_payment_error.code` from the event that failed this order — one of
   * `vpay_core::failure::FailureCode`'s eleven, and `null` on every other
   * status. Stored rather than derived because the event is the only place
   * it ever appears: the shop never calls vpay to read an intent.
   */
  failureCode: string | null;
  /** The rail-facing sentence that came with it. Operator-facing, not buyer-facing. */
  failureMessage: string | null;
  items: OrderLine[];
  createdAt: Date;
}

export interface NewOrder {
  email: string | null;
  currency: string;
  totalMinor: number;
  items: OrderLine[];
}

/** What `applyWebhookEvent` did, and what the route handler answers with. */
export type WebhookOutcome =
  /** The event was new, the order existed, and its status was written. */
  | "applied"
  /** vpay's event id was already recorded. Nothing was written. */
  | "duplicate"
  /** No order carries that `pi_…`. Nothing was written. */
  | "unknown_intent"
  /**
   * The order had already left `unpaid`. The delivery was recorded, the
   * status was not touched — `paid` is terminal for this shop.
   */
  | "already_settled";

export interface WebhookApplication {
  /** vpay's `evt_…`. The dedupe key. */
  eventId: string;
  /** vpay's event type, stored for the record. */
  type: string;
  /** The `pi_…` the event's object names. */
  paymentIntentId: string;
  /** What the order's status becomes when the event is applied. */
  nextStatus: OrderStatus;
  /** `last_payment_error.code`, when the event carried one. */
  failureCode: string | null;
  /** `last_payment_error.message`, when the event carried one. */
  failureMessage: string | null;
}

/** Thrown when an order already carries a different `pi_…`. */
export class PaymentIntentConflictError extends Error {
  constructor(orderId: string) {
    super(`order ${orderId} already has a payment intent`);
    this.name = "PaymentIntentConflictError";
  }
}

export interface ShopStore {
  /** The catalogue, in a stable display order (price ascending, then id). */
  listProducts(): Promise<Product[]>;
  /** Only the products whose ids were asked for. Missing ids are simply absent. */
  findProducts(ids: readonly string[]): Promise<Product[]>;
  createOrder(input: NewOrder): Promise<Order>;
  getOrder(id: string): Promise<Order | null>;
  /**
   * Records the intent id **before** the checkout session is created, so a
   * crash between the two calls leaves a row naming the transaction that
   * exists at vpay (AGENTS.md: "never let a payer act on a transaction you
   * cannot name").
   */
  setPaymentIntentId(orderId: string, paymentIntentId: string): Promise<Order>;
  setCheckoutSessionId(
    orderId: string,
    checkoutSessionId: string,
  ): Promise<Order>;
  /**
   * Dedupe, look up, and write — as one operation, because the 2xx this
   * shop answers with must mean the write happened.
   */
  applyWebhookEvent(application: WebhookApplication): Promise<WebhookOutcome>;
}
