/**
 * Wire and API types for vpay's browser surface
 * (`docs/plans/2026-09-03-step5c-stripejs.md` §1, `docs/api/README.md`).
 *
 * The object types mirror `vpay_api::model` exactly. Where a name also
 * exists in `@stripe/stripe-js` the shape is Stripe's, so that a merchant
 * who has integrated against Stripe reads the same field names here — see
 * `src/compat.test.ts` for which of those shapes are actually assignable
 * across the two packages and which are not.
 */
import type { StripeError } from "./errors.js";

/**
 * The five-value state machine `vpay_core::state::IntentStatus` defines
 * (`docs/flows/payment-lifecycle.md`). There is deliberately no `failed`
 * status — a rail failure returns the intent to `requires_payment_method`
 * with {@link LastPaymentError} populated, which is why
 * {@link Stripe.waitForPaymentIntent} must inspect both fields to decide it
 * is finished.
 */
export type PaymentIntentStatus =
  | "requires_payment_method"
  | "requires_action"
  | "processing"
  | "succeeded"
  | "canceled";

/** Redirect-rail next step — Stripe's own `next_action.redirect_to_url` shape. */
export interface NextAction {
  type: "redirect_to_url";
  redirect_to_url: {
    url: string;
    /**
     * The merchant's `return_url`, echoed back. **A label, not a
     * destination**: as of Step 5c nothing in vpay redirects a payer to it
     * (D4, and §0 S2 of the design). See the package README.
     */
    return_url: string | null;
  };
}

/**
 * The closed failure-code vocabulary `vpay_core::failure` owns
 * (`docs/flows/failures.md`). Adapters map rail-specific error strings into
 * this list, so a payer-facing message is written once per code rather than
 * once per rail.
 */
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

export interface LastPaymentError {
  code: FailureCode;
  message: string;
}

/**
 * `vpay_api::model::PaymentIntentWithSecret`: the twelve keys of
 * `PaymentIntentObject` — pinned server-side by
 * `every_documented_key_is_present_including_the_null_ones` — plus the
 * `client_secret` the browser surface flattens in alongside them (D2).
 *
 * `client_secret` is `string`, not Stripe's `string | null`: the browser
 * routes are reachable *only* by presenting one, so a response that omitted
 * it would be a server bug rather than a case to model.
 */
export interface PaymentIntent {
  id: string;
  object: "payment_intent";
  /** Integer minor units. XAF is zero-decimal: `5000` means 5,000 FCFA. */
  amount: number;
  /** Lowercase ISO 4217 code, e.g. `xaf`. */
  currency: string;
  status: PaymentIntentStatus;
  payment_method_types: string[];
  /** `null` on a push rail, and on an intent nobody has confirmed. */
  next_action: NextAction | null;
  /** Present *with* `requires_payment_method` — there is no `failed` status. */
  last_payment_error: LastPaymentError | null;
  metadata: Record<string, string>;
  description: string | null;
  /** Unix **seconds**, not milliseconds. */
  created: number;
  livemode: boolean;
  /** `pi_…_secret_…`. Never log this. */
  client_secret: string;
}

/**
 * Stripe.js's `PaymentIntentResult`, member for member.
 *
 * The `error?: undefined` / `paymentIntent?: undefined` members are what
 * make `if (result.error)` narrow `result.paymentIntent` to non-`undefined`
 * — without them a merchant's Stripe-shaped code needs a non-null assertion.
 */
export type PaymentIntentResult =
  | { paymentIntent: PaymentIntent; error?: undefined }
  | { paymentIntent?: undefined; error: StripeError };

/** Options for {@link loadStripe}. */
export interface VpayStripeOptions {
  /**
   * The origin vpay is served from, e.g. `https://api.vpay.example`. The
   * `/v1/browser/...` path is appended by this package; a trailing slash is
   * tolerated and stripped.
   */
  baseUrl: string;
  /**
   * Injected `fetch`, for tests and for a host that wants its own
   * instrumentation. Defaults to `globalThis.fetch`.
   */
  fetch?: typeof fetch | undefined;
}

/** `payment_method_data` for {@link Stripe.confirmMobileMoneyPayment}. */
export interface MobileMoneyData {
  /** A rail code, e.g. `mtn_momo`. Encoded as `payment_method_data[type]`. */
  type: string;
  /** Encoded as `payment_method_data[<type>][msisdn]`. */
  msisdn: string;
}

/** Options for {@link Stripe.confirmPayment}. */
export interface ConfirmPaymentOptions {
  clientSecret: string;
  confirmParams?:
    | {
        return_url?: string | undefined;
        payment_method_data?: Record<string, unknown> | undefined;
      }
    | undefined;
  /**
   * `always` (the default) navigates the browser when the rail answers with
   * `next_action.redirect_to_url`, and the returned promise then never
   * settles — Stripe.js's own behaviour, and the only way a caller's
   * `finally` cannot fire a "payment failed" state during the unload.
   * `if_required` suppresses the navigation and resolves with the intent so
   * the caller can render the redirect itself.
   */
  redirect?: "always" | "if_required" | undefined;
}

/** Options for {@link Stripe.waitForPaymentIntent}. */
export interface WaitForPaymentIntentOptions {
  /** Total budget in milliseconds. Default 180000 (three minutes). */
  timeoutMs?: number | undefined;
  /** Base delay between polls in milliseconds, jittered ±25%. Default 2000. */
  intervalMs?: number | undefined;
}

/**
 * The object {@link loadStripe} resolves to — a deliberate subset of
 * Stripe.js's `Stripe`.
 *
 * Everything Stripe.js exposes that depends on card data, an iframe served
 * from `js.stripe.com`, or a Stripe-hosted page is **absent by
 * construction**, not stubbed: Elements, `confirmCardPayment`,
 * `createPaymentMethod`, Checkout, Link, Payment Request. The package README
 * carries the full list. A method that is missing is a compile error at the
 * merchant; a method that returned a plausible-looking failure would be a
 * runtime surprise on a payment page.
 */
export interface Stripe {
  /** `GET /v1/browser/payment_intents/{id}`. The polling endpoint. */
  retrievePaymentIntent(clientSecret: string): Promise<PaymentIntentResult>;
  /** `POST /v1/browser/payment_intents/{id}/confirm`, then the redirect rule above. */
  confirmPayment(options: ConfirmPaymentOptions): Promise<PaymentIntentResult>;
  /** Retrieves, then performs the navigation `next_action` calls for, if any. */
  handleNextAction(options: {
    clientSecret: string;
  }): Promise<PaymentIntentResult>;
  /** {@link confirmPayment} for a push rail: the two fields a payer supplies. */
  confirmMobileMoneyPayment(
    clientSecret: string,
    data: MobileMoneyData,
  ): Promise<PaymentIntentResult>;
  /** Polls {@link retrievePaymentIntent} until the intent stops moving. */
  waitForPaymentIntent(
    clientSecret: string,
    options?: WaitForPaymentIntentOptions,
  ): Promise<PaymentIntentResult>;
}
