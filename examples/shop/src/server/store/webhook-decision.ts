/**
 * The one place that decides what a webhook delivery does to an order.
 *
 * Both `ShopStore` implementations — Prisma and the in-memory one the tests
 * use — call this, so the rule the tests prove is the rule production runs.
 * Without it the two implementations would each carry their own copy of the
 * dedupe-then-settle logic and only one of them would ever be tested.
 */
import type { OrderStatus, WebhookOutcome } from "./types";

export interface WebhookDecisionInput {
  /** Whether vpay's event id is already in `webhook_events`. */
  alreadySeen: boolean;
  /** The order carrying the event's `pi_…`, or `null` if no order does. */
  orderStatus: OrderStatus | null;
  /** What the event type maps to. */
  nextStatus: OrderStatus;
}

export interface WebhookDecision {
  outcome: WebhookOutcome;
  /** Whether to insert the `webhook_events` row. */
  recordEvent: boolean;
  /** The status to write, or `null` to leave the order alone. */
  writeStatus: OrderStatus | null;
}

/**
 * - An event id already seen changes nothing at all. Delivery is
 *   at-least-once (docs/flows/webhooks.md), so this is the ordinary case,
 *   not an error.
 * - An intent no order claims changes nothing at all, and is still a 2xx:
 *   the delivery is not this shop's, and answering non-2xx would make vpay
 *   retry it forever.
 * - An order that has already settled records the delivery but keeps its
 *   status. `paid` is terminal here; a later `payment_intent.payment_failed`
 *   for the same intent must not un-pay a shipped order.
 */
export function decideWebhook(input: WebhookDecisionInput): WebhookDecision {
  if (input.alreadySeen) {
    return { outcome: "duplicate", recordEvent: false, writeStatus: null };
  }
  if (input.orderStatus === null) {
    return { outcome: "unknown_intent", recordEvent: false, writeStatus: null };
  }
  if (input.orderStatus !== "unpaid") {
    return { outcome: "already_settled", recordEvent: true, writeStatus: null };
  }
  return {
    outcome: "applied",
    recordEvent: true,
    writeStatus: input.nextStatus,
  };
}
