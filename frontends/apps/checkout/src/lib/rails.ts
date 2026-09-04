/**
 * Which rails this page can drive, and how.
 *
 * **The list of rails on offer is never written here.** It comes from the
 * intent's own `payment_method_types`, which the merchant chose when it
 * created the intent and the server validated against its configured
 * providers. What this module owns is a different question: for a rail the
 * intent offers, *what does the page have to do* — collect a mobile-money
 * number and push, or hand the payer to the rail's own web page?
 *
 * That is D9. A rail the map below does not name is not silently dropped
 * and not rendered as a button that fails on click: it is listed as
 * unsupported, with its code shown, and if it is the *only* rail the intent
 * offers the page refuses outright. A payer who cannot pay must be told so
 * before they try, not after the redirect goes nowhere.
 */
import type { MessageKey } from '../i18n/index';
import type { PaymentIntent, PublicPaymentIntent } from './types';

/** What the page must do for a rail. */
export type RailFlow = 'mobile_money_push' | 'redirect';

/**
 * Rail code → the page flow it needs, and the dictionary key naming it.
 *
 * The `label` is a `MessageKey` rather than a string so that a rail's name
 * is translated like everything else. Adding a rail to this map without
 * adding its key to both dictionaries does not compile.
 */
export const RAIL_PAGE_FLOWS: Readonly<Record<string, { flow: RailFlow; label: MessageKey }>> =
  Object.freeze({
    mtn_momo: { flow: 'mobile_money_push', label: 'rail.mtn_momo' },
    orange_money: { flow: 'redirect', label: 'rail.orange_money' },
  });

export interface SupportedRail {
  code: string;
  flow: RailFlow;
  label: MessageKey;
}

export interface RailChoices {
  /** Rails the intent offers that this page knows how to drive, in the intent's order. */
  supported: SupportedRail[];
  /** Rail codes the intent offers that this page cannot drive (D9). */
  unsupported: string[];
}

/** Splits an intent's `payment_method_types` into what this page can and cannot do. */
export function railChoices(intent: PaymentIntent | PublicPaymentIntent): RailChoices {
  const supported: SupportedRail[] = [];
  const unsupported: string[] = [];
  for (const code of intent.payment_method_types) {
    const entry = RAIL_PAGE_FLOWS[code];
    if (entry === undefined) {
      unsupported.push(code);
      continue;
    }
    supported.push({ code, flow: entry.flow, label: entry.label });
  }
  return { supported, unsupported };
}
