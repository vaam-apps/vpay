/**
 * What the shop tells a buyer when a payment did not work, and whether it
 * offers them a way to try again.
 *
 * The vocabulary is **vpay's**, not a rail's: `vpay_core::failure::FailureCode`
 * (docs/flows/failures.md) is a closed list every adapter maps its own error
 * strings into, which is the whole reason a merchant can write one message
 * per outcome instead of one per rail. This module is the merchant's half of
 * that bargain — the eleven codes, one sentence each, in a module that
 * imports nothing so a client component can render it.
 *
 * Two rules hold here and are tested in `failures.test.ts`:
 *
 * 1. **Every code has a message.** An unrecognised code renders a message
 *    that says the payment failed and says nothing else — never a blank, and
 *    never a guess at which of the eleven it might have been.
 * 2. **`retryable` is a superset of `FailureCode::payer_actionable`, and
 *    disjoint from `FailureCode::merchant_actionable`.** A shop that offered
 *    "try again" after `invalid_payee` — the shop's *own* collection account
 *    refused — would be inviting a buyer to fail identically a second time;
 *    one that withheld it after `insufficient_funds` would be turning a
 *    top-up away.
 *
 *    The superset, not equality, is a judgement this shop makes and the core
 *    deliberately does not. `payer_actionable` asks "could the **payer** do
 *    something differently", and the answer for `provider_unavailable` and
 *    `provider_error` is no — but a fresh order on a rail that has come back
 *    up succeeds, and the buyer is the only person present to press the
 *    button. Those two codes are the whole of the difference and they are
 *    named in `failures.test.ts`, so a third one cannot be added here
 *    quietly.
 */

/** `vpay_core::failure::FailureCode`, in the spelling `/v1` puts on the wire. */
export type FailureCode =
  | "insufficient_funds"
  | "payer_timeout"
  | "payer_declined"
  | "invalid_payer"
  | "payer_limit_reached"
  | "payer_account_blocked"
  | "invalid_payee"
  | "payee_account_blocked"
  | "provider_account_blocked"
  | "provider_unavailable"
  | "provider_error";

export interface FailureCopy {
  /** A heading the buyer reads first. */
  title: string;
  /** One sentence, written for a buyer and not for an operator. */
  detail: string;
  /**
   * Whether the shop offers "try again". Every `payer_actionable` code, plus
   * the two rail-side ones a later attempt can clear — see this module's
   * header, and the test that pins exactly which. The retry is a **new
   * order** and therefore a new PaymentIntent, because vpay allows one
   * charge per intent forever.
   */
  retryable: boolean;
}

/**
 * The table. Ordered as `FailureCode::ALL` is, so the two can be diffed by
 * eye against `backends/crates/vpay-core/src/failure.rs`.
 */
export const FAILURE_COPY: Readonly<Record<FailureCode, FailureCopy>> = {
  insufficient_funds: {
    title: "Not enough money in the wallet",
    detail:
      "The rail said the balance was too low. Top the wallet up and place the order again — nothing has been charged.",
    retryable: true,
  },
  payer_timeout: {
    title: "The payment prompt expired",
    detail:
      "Nobody answered the prompt in time, so the rail closed it. You can start again; nothing has been charged.",
    retryable: true,
  },
  payer_declined: {
    title: "The payment was refused on the handset",
    detail:
      "The rail reported that the prompt was declined. Start again if that was not what you meant to do.",
    retryable: true,
  },
  invalid_payer: {
    title: "The rail does not know that number",
    detail:
      "The mobile-money account you gave is not one this rail could find. Check the number and start again.",
    retryable: false,
  },
  payer_limit_reached: {
    title: "A wallet limit was reached",
    detail:
      "The rail refused because the wallet is over one of its own limits — per transaction, per day, or per month.",
    retryable: true,
  },
  payer_account_blocked: {
    title: "The wallet is not active",
    detail:
      "The rail reported that the paying account is blocked or not yet activated. Your mobile-money provider can say why.",
    retryable: false,
  },
  invalid_payee: {
    title: "We could not accept the payment",
    detail:
      "The shop's own collection account was refused by the rail. This one is on us — nothing you can do differently.",
    retryable: false,
  },
  payee_account_blocked: {
    title: "We could not accept the payment",
    detail:
      "The shop's collection account is blocked at the rail. This one is on us; please come back later.",
    retryable: false,
  },
  provider_account_blocked: {
    title: "We could not accept the payment",
    detail:
      "The shop's partner account with the rail is blocked. This one is on us and someone here has been paged.",
    retryable: false,
  },
  provider_unavailable: {
    title: "The mobile-money rail could not answer",
    detail:
      "The rail itself is unavailable. Nothing has been charged; trying again in a few minutes usually works.",
    retryable: true,
  },
  provider_error: {
    title: "The rail refused the payment",
    detail:
      "The rail refused and did not say why in a way we can pass on. Nothing has been charged.",
    retryable: true,
  },
};

/** What the shop shows for a `failed` order whose code it does not recognise. */
export const UNKNOWN_FAILURE: FailureCopy = {
  title: "The payment failed",
  detail:
    "The rail refused the payment and nothing has been charged. We do not have a more specific reason to give you.",
  // A code this shop's build predates is not something to declare final: a
  // fresh order is cheap, and refusing one on an outcome we cannot read
  // would be the shop guessing.
  retryable: true,
};

/**
 * The copy for a `last_payment_error.code`, never `undefined`.
 *
 * `Object.hasOwn`, not a bare index: the code arrives inside a webhook body,
 * and `FAILURE_COPY["constructor"]` is a truthy function rather than
 * `undefined` — which would then be rendered at a buyer.
 */
export function failureCopy(code: string | null): FailureCopy {
  if (code === null) {
    return UNKNOWN_FAILURE;
  }
  return Object.hasOwn(FAILURE_COPY, code)
    ? (FAILURE_COPY[code as FailureCode] ?? UNKNOWN_FAILURE)
    : UNKNOWN_FAILURE;
}
