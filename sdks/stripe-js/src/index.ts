/**
 * `@vpay/stripe-js` — a Stripe.js-shaped browser client for vpay's payer
 * surface.
 *
 * Drop-in for the *payment-intent* half of Stripe.js and nothing else. See
 * README.md for the compatible surface, the list of Stripe features that are
 * not compatible and never will be, and this package's own Status section
 * (as of this commit the server routes it speaks to are being built in the
 * same step and nothing here has run against them).
 */
export { loadStripe } from "./client.js";

export type { StripeError } from "./errors.js";

export type {
  Stripe,
  PaymentIntent,
  PaymentIntentResult,
  PaymentIntentStatus,
  NextAction,
  LastPaymentError,
  FailureCode,
  VpayStripeOptions,
  ConfirmPaymentOptions,
  WaitForPaymentIntentOptions,
  MobileMoneyData,
} from "./types.js";
