/**
 * What `orders.get` puts on the wire, in a module that imports nothing.
 *
 * Both the tRPC router (server) and the pages (some of them client
 * components) name this type, and a shared *type-only* module keeps the
 * client bundle from ever having a reason to resolve a server module.
 */

/** Mirrors `schema.zmodel`'s `OrderStatus`. */
export type OrderStatus = "unpaid" | "paid" | "failed" | "cancelled";

export interface OrderLineView {
  productId: string;
  name: string;
  quantity: number;
  unitMinor: number;
}

export interface OrderView {
  id: string;
  /** `null` when the buyer did not give one — it is optional at checkout. */
  email: string | null;
  status: OrderStatus;
  totalMinor: number;
  currency: string;
  /** `pi_…`. An identifier, not a credential. */
  paymentIntentId: string | null;
  /** `cs_…`. An identifier, not a credential. */
  checkoutSessionId: string | null;
  /** vpay's `last_payment_error.code` for a `failed` order; `null` otherwise. */
  failureCode: string | null;
  /**
   * The sentence the rail's own vocabulary produced, carried through
   * `last_payment_error.message`. Shown on the order page under "for the
   * runbook" — the buyer reads `failureCode`'s copy from
   * `src/lib/failures.ts`, not this.
   */
  failureMessage: string | null;
  /** Unix milliseconds. */
  createdAt: number;
  items: OrderLineView[];
}

/** Whether the shop is still waiting for a webhook to settle this order. */
export function isPending(status: OrderStatus): boolean {
  return status === "unpaid";
}
