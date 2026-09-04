/**
 * `vpay_core::failure`'s closed vocabulary → a payer-facing message key.
 *
 * A `Record<FailureCode, MessageKey>` and not a template string: a code the
 * dictionaries have no sentence for must be a compile error, because the
 * alternative — `t(\`failure.${code}\`)` — renders the raw code on a payment
 * page the first time a rail reports something new.
 */
import type { FailureCode } from '@vpay/stripe-js';

import type { MessageKey } from '../i18n/index';

export const FAILURE_MESSAGES: Readonly<Record<FailureCode, MessageKey>> = Object.freeze({
  insufficient_funds: 'failure.insufficient_funds',
  payer_timeout: 'failure.payer_timeout',
  payer_declined: 'failure.payer_declined',
  invalid_payer: 'failure.invalid_payer',
  payer_limit_reached: 'failure.payer_limit_reached',
  payer_account_blocked: 'failure.payer_account_blocked',
  invalid_payee: 'failure.invalid_payee',
  payee_account_blocked: 'failure.payee_account_blocked',
  provider_account_blocked: 'failure.provider_account_blocked',
  provider_unavailable: 'failure.provider_unavailable',
  provider_error: 'failure.provider_error',
});

/** `failure.unknown` for a code no released dictionary names — never the raw code. */
export function failureMessage(code: FailureCode | null): MessageKey | null {
  if (code === null) {
    return null;
  }
  return FAILURE_MESSAGES[code] ?? 'failure.unknown';
}
