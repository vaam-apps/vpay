import {
  CHECKOUT_STATUS,
  DELIVERY_STATE,
  REFUND_STATUS,
  checkoutStatusLabel,
  deliveryStateLabel,
  refundStatusLabel,
  type CheckoutStatus,
  type DeliveryState,
  type RefundStatus,
} from "@vpay/tokens";
import {
  createStatusPill,
  defineStatusSystem,
  type StatusSystem,
} from "@vaam-apps/ui";

import { PAYMENT_STATUS_SYSTEM } from "./payment-status";

/**
 * Refunds, webhook deliveries and checkout sessions, as status systems.
 *
 * Until the Material 3 Expressive revamp these three lists printed their
 * state column as the raw wire string (`succeeded`, `exhausted`), so the
 * one column an operator scans for "is anything wrong" was the one column
 * that looked like every other. They render through `@vaam-apps/ui`'s
 * status components now, with the words from `@vpay/tokens` — the same
 * rule `payment-status.ts` follows for a payment.
 *
 * `attention: "loud"` is reserved for the two states a person has to act
 * on: a refund that failed and a delivery whose retries ran out. Everything
 * else is quiet, so a list with two loud rows reads instantly.
 */
export const REFUND_STATUS_SYSTEM = defineStatusSystem<RefundStatus>({
  pending: {
    family: "in-flight",
    silhouette: "circle",
    mark: "pie-1",
    hue: "progress",
    filled: false,
    attention: "quiet",
    label: refundStatusLabel.pending,
    tooltip: "Sent to the rail, which has not answered yet.",
  },
  succeeded: {
    family: "terminal",
    silhouette: "circle",
    mark: "check",
    hue: "success",
    filled: true,
    attention: "quiet",
    label: refundStatusLabel.succeeded,
    tooltip: "The rail confirmed the money went back. Terminal.",
  },
  failed: {
    family: "terminal",
    silhouette: "circle",
    mark: "cross",
    hue: "danger",
    filled: true,
    attention: "loud",
    label: refundStatusLabel.failed,
    tooltip: "The rail refused the refund. Nothing was moved back. Terminal.",
  },
  canceled: {
    family: "terminal",
    silhouette: "square",
    mark: "slash",
    hue: "warning",
    filled: true,
    attention: "quiet",
    label: refundStatusLabel.canceled,
    tooltip: "Cancelled before any rail saw it. Terminal.",
  },
});

export const DELIVERY_STATE_SYSTEM = defineStatusSystem<DeliveryState>({
  pending: {
    family: "in-flight",
    silhouette: "circle",
    mark: "clock",
    hue: "progress",
    filled: false,
    attention: "quiet",
    label: deliveryStateLabel.pending,
    tooltip:
      "Owed an attempt — the first one, or a retry after an endpoint answered with something other than a 2xx.",
  },
  succeeded: {
    family: "terminal",
    silhouette: "circle",
    mark: "check",
    hue: "success",
    filled: true,
    attention: "quiet",
    label: deliveryStateLabel.succeeded,
    tooltip: "The merchant's endpoint answered 2xx. Terminal.",
  },
  failed: {
    family: "terminal",
    silhouette: "circle",
    mark: "cross",
    hue: "danger",
    filled: true,
    attention: "loud",
    label: deliveryStateLabel.failed,
    tooltip:
      "In the vocabulary, written by nothing today: a failure that is not exhausted stays owed an attempt.",
  },
  exhausted: {
    family: "terminal",
    silhouette: "diamond",
    mark: "cross",
    hue: "danger",
    filled: true,
    attention: "loud",
    label: deliveryStateLabel.exhausted,
    tooltip:
      "The retry ladder ran out. The merchant never acknowledged this event. Terminal.",
  },
});

export const CHECKOUT_STATUS_SYSTEM = defineStatusSystem<CheckoutStatus>({
  open: {
    family: "in-flight",
    silhouette: "circle",
    mark: "ring",
    hue: "parked",
    filled: false,
    attention: "quiet",
    label: checkoutStatusLabel.open,
    tooltip: "Waiting for the payer to finish on the checkout page.",
  },
  complete: {
    family: "terminal",
    silhouette: "circle",
    mark: "check",
    hue: "success",
    filled: true,
    attention: "quiet",
    label: checkoutStatusLabel.complete,
    tooltip: "The payer finished the checkout. Terminal.",
  },
  expired: {
    family: "terminal",
    silhouette: "square",
    mark: "clock",
    hue: "expired",
    filled: true,
    attention: "quiet",
    label: checkoutStatusLabel.expired,
    tooltip: "Nobody finished it before it expired. Terminal.",
  },
});

export const RefundStatusPill = createStatusPill(REFUND_STATUS_SYSTEM);
export const DeliveryStatePill = createStatusPill(DELIVERY_STATE_SYSTEM);
export const CheckoutStatusPill = createStatusPill(CHECKOUT_STATUS_SYSTEM);

/** Every status system this app renders — for the contrast gate. */
export const STATUS_SYSTEMS: readonly StatusSystem<string>[] = [
  PAYMENT_STATUS_SYSTEM,
  REFUND_STATUS_SYSTEM,
  DELIVERY_STATE_SYSTEM,
  CHECKOUT_STATUS_SYSTEM,
];

const isIn =
  <S extends string>(list: readonly S[]) =>
  (value: string): value is S =>
    (list as readonly string[]).includes(value);

/** `value` if this build can name it, else `null` — render `null` as text. */
export const asRefundStatus = (v: string): RefundStatus | null =>
  isIn(REFUND_STATUS)(v) ? v : null;
export const asDeliveryState = (v: string): DeliveryState | null =>
  isIn(DELIVERY_STATE)(v) ? v : null;
export const asCheckoutStatus = (v: string): CheckoutStatus | null =>
  isIn(CHECKOUT_STATUS)(v) ? v : null;
