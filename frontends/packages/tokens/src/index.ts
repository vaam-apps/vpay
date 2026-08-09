/**
 * Design tokens.
 *
 * Payment status is the one place colour carries meaning in this product, so
 * the mapping lives here rather than being re-invented per component.
 */

export const PAYMENT_STATUS = [
  'requires_payment_method',
  'requires_action',
  'processing',
  'succeeded',
  'canceled',
] as const;

export type PaymentStatus = (typeof PAYMENT_STATUS)[number];

/** daisyUI semantic colour per status. */
export const statusTone: Record<PaymentStatus, 'neutral' | 'info' | 'warning' | 'success' | 'error'> = {
  requires_payment_method: 'neutral',
  requires_action: 'warning',
  processing: 'info',
  succeeded: 'success',
  canceled: 'error',
};

/**
 * Copy shown to an operator.
 *
 * `processing` deliberately does not say "pending" — the whole point of that
 * state is that nothing has been decided, and the dashboard must not imply
 * a payment is nearly done. See docs/flows/payment-lifecycle.md.
 */
export const statusLabel: Record<PaymentStatus, string> = {
  requires_payment_method: 'Awaiting payment method',
  requires_action: 'Awaiting payer on the rail',
  processing: 'In flight — not yet decided',
  succeeded: 'Succeeded',
  canceled: 'Canceled',
};
