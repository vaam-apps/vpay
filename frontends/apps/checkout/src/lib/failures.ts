/**
 * `vpay_core::failure`'s closed vocabulary → a payer-facing message key.
 *
 * A `Record<FailureCode, MessageKey>` and not a template string: a code the
 * dictionaries have no sentence for must be a compile error, because the
 * alternative — `t(\`failure.${code}\`)` — renders the raw code on a payment
 * page the first time a rail reports something new.
 */
import type { FailureCode } from '@vaam-apps/vpay-stripe-js';

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

/**
 * The rail's own sentence, where the API gave one, bounded and cleaned.
 *
 * **Shown as data beside the translated message, never instead of it.**
 * `last_payment_error.message` is written by whatever wrote the rail's
 * response; it arrives in a language nobody chose, it is not part of the
 * closed vocabulary above, and this page does not control a word of it. The
 * reason it is rendered at all is that a payer standing in a shop with
 * "payment not completed" on the screen, when the rail actually said
 * something specific, is a payer this page is withholding the useful half
 * from.
 *
 * What this function guarantees to the layout: at most
 * {@link MAX_PROVIDER_REASON} characters, no control characters, whitespace
 * collapsed, and `null` rather than an empty string. React escapes the rest
 * — the value reaches the DOM as a text node and never as markup, an
 * attribute or a URL.
 */
export const MAX_PROVIDER_REASON = 300;

/** Every C0 and C1 control character, plus the Unicode line separators. */
// eslint-disable-next-line no-control-regex -- naming the control characters is the whole job: this is what strips them out of a rail's message before it is rendered.
const CONTROL = /[\u0000-\u001f\u007f-\u009f\u2028\u2029]+/g;

export function providerReason(message: string | null | undefined): string | null {
  if (typeof message !== 'string') {
    return null;
  }
  const cleaned = message.replace(CONTROL, ' ').replace(/\s+/g, ' ').trim();
  if (cleaned.length === 0) {
    return null;
  }
  return cleaned.length > MAX_PROVIDER_REASON
    ? `${cleaned.slice(0, MAX_PROVIDER_REASON - 1)}\u2026`
    : cleaned;
}
