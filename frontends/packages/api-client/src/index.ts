/**
 * Typed client for the vpay dashboard API (`/dash/v1`).
 *
 * NOTE: this is deliberately NOT a client for `/v1` (the Stripe-shaped merchant
 * API). The dashboard authenticates with an OIDC session, never a merchant
 * secret key — see docs/adr/0008-dashboard-scope.md.
 *
 * STATUS: types only. No request is issued yet; see docs/status.md.
 */

import type { PaymentStatus } from '@vpay/tokens';

export interface PaymentIntentView {
  id: string;
  status: PaymentStatus;
  /** Integer minor units. XAF is zero-decimal: 5000 means 5,000 FCFA. */
  amount: number;
  currency: string;
  providerCode: string | null;
  createdAt: string;
}

export class NotImplementedError extends Error {
  constructor(what: string) {
    super(`${what} is not implemented — see docs/status.md`);
    this.name = 'NotImplementedError';
  }
}

/**
 * Format a minor-unit amount for display.
 *
 * Mirrors `Money::to_provider_string` in `vpay-core`; the two are covered by
 * the same table of cases in docs/flows/money.md.
 */
export function formatAmount(minor: number, currency: string): string {
  if (!Number.isInteger(minor)) {
    throw new TypeError(`amount must be an integer in minor units, got ${minor}`);
  }
  const exponent = currency.toUpperCase() === 'XAF' ? 0 : 2;
  if (exponent === 0) return `${minor} ${currency.toUpperCase()}`;
  const divisor = 10 ** exponent;
  const major = Math.trunc(minor / divisor);
  const frac = Math.abs(minor % divisor);
  return `${major}.${String(frac).padStart(exponent, '0')} ${currency.toUpperCase()}`;
}

/** @throws NotImplementedError always. */
// async is the shape `PaymentIntentView[]` callers await; there is nothing to
// await until this is implemented.
// eslint-disable-next-line @typescript-eslint/require-await
export async function listPayments(): Promise<PaymentIntentView[]> {
  throw new NotImplementedError('listPayments');
}
