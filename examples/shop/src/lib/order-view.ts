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
  email: string;
  status: OrderStatus;
  totalMinor: number;
  currency: string;
  /** `pi_…`. An identifier, not a credential. */
  paymentIntentId: string | null;
  /** `cs_…`. An identifier, not a credential. */
  checkoutSessionId: string | null;
  /** Unix milliseconds. */
  createdAt: number;
  items: OrderLineView[];
}

/** Whether the shop is still waiting for a webhook to settle this order. */
export function isPending(status: OrderStatus): boolean {
  return status === "unpaid";
}
