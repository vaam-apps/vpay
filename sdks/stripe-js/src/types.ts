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
   * The origin vpay's **checkout app** is served from, e.g.
   * `https://checkout.vpay.example` — the deployment's
   * `checkout.public_base_url`. A different host from {@link baseUrl}: one
   * serves the API, the other serves the page that goes in the iframe.
   *
   * Optional, and **required for {@link Stripe.initEmbeddedCheckout}**,
   * which rejects without it rather than guessing. It is also the value the
   * `message` listener pins `event.origin` against, so a wrong one does not
   * fail open — it silently accepts nothing.
   *
   * Validated here when supplied: an absolute `http:`/`https:` URL, trailing
   * slashes stripped.
   */
  checkoutBaseUrl?: string | undefined;
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
  /**
   * `GET /v1/browser/checkout/sessions/{id}` — reads a Checkout Session by
   * its own `client_secret`. Never rejects.
   */
  retrieveCheckoutSession(clientSecret: string): Promise<CheckoutSessionResult>;
  /**
   * Frames vpay's checkout page on the merchant's own page (Step 9's D8).
   *
   * **This one rejects**, like {@link loadStripe}: a missing
   * `checkoutBaseUrl` or a `client_secret` that is not a session's is an
   * integration mistake visible on the first render, not a payer-facing
   * failure, and a rejection from `fetchClientSecret` is the merchant's own
   * server call failing.
   */
  initEmbeddedCheckout(
    options: InitEmbeddedCheckoutOptions,
  ): Promise<EmbeddedCheckout>;
}

/**
 * A Checkout Session's lifecycle state — Step 9's D10, and deliberately
 * vpay's own three values rather than Stripe's.
 *
 * There is no `failed`: a session whose intent reached a terminal
 * non-success state is reported `expired` with
 * {@link CheckoutSession.payment_status} `failed`, the same way a
 * PaymentIntent has no `failed` status either.
 */
export type CheckoutSessionStatus = "open" | "complete" | "expired";

/** Whether the session's intent has been paid. D10's second axis. */
export type CheckoutPaymentStatus = "unpaid" | "paid" | "failed";

/**
 * Which surface the session is rendered on: `hosted` (vpay serves a
 * top-level page and returns its `url`) or `embedded` (the merchant frames
 * vpay's page with {@link Stripe.initEmbeddedCheckout}).
 */
export type CheckoutSessionUiMode = "hosted" | "embedded";

/**
 * `checkout.session` as the **browser** surface renders it — the object in
 * §"The wire contract" of `docs/plans/2026-09-04-step9-hosted-checkout.md`,
 * with `payment_intent` expanded.
 *
 * The one place this differs from the merchant SDKs' `CheckoutSession`
 * (`@vpay/sdk`, `vpay_sdk`) is {@link payment_intent}, and the difference is
 * deliberate rather than a skew: on `/v1` the field is the `pi_…` **id**; on
 * `GET /v1/browser/checkout/sessions/{id}` it is the whole intent, because
 * vpay's checkout page has to confirm and poll it through the existing
 * browser routes and a second round trip to fetch it would need a credential
 * the page does not have yet. Stripe's own `expand` convention, applied at
 * exactly one route.
 *
 * **This means `retrieveCheckoutSession` hands its caller a live *confirm*
 * credential** (`session.payment_intent.client_secret`), not only a
 * session-read one. That is a wider exposure than the session's own secret
 * and is stated here rather than buried: a merchant's outer page that only
 * wants the session's status should read `status` and `payment_status` and
 * leave the intent alone.
 *
 * `client_secret` is `string`, not `string | null`, for the same reason
 * {@link PaymentIntent.client_secret} is: this route is reachable *only* by
 * presenting one.
 */
export interface CheckoutSession {
  /** `cs_…`. */
  id: string;
  object: "checkout.session";
  livemode: boolean;
  /**
   * The intent this session drives, **expanded** — every
   * {@link PaymentIntent} field, `client_secret` included. A session never
   * creates its intent; it references one that already exists.
   *
   * Typed as the object rather than `string | PaymentIntent` because this
   * package calls exactly one route and that route always expands it. The
   * *return* read (`…/{id}/return?t=…`) expands it too but omits the
   * intent's `client_secret` — nothing here calls that route, it is vpay's
   * own return page, and typing a union for a response this package never
   * receives would push a needless narrowing onto every caller.
   */
  payment_intent: PaymentIntent;
  ui_mode: CheckoutSessionUiMode;
  status: CheckoutSessionStatus;
  payment_status: CheckoutPaymentStatus;
  /** Hosted mode only; `null` on an embedded session. May carry `{CHECKOUT_SESSION_ID}` (D5). */
  success_url: string | null;
  /** Hosted mode only; `null` on an embedded session. */
  cancel_url: string | null;
  /** Embedded mode only; `null` on a hosted session. */
  return_url: string | null;
  /** The page vpay serves for a hosted session, secret in the fragment (D6); `null` when embedded. */
  url: string | null;
  /** Unix **seconds**. 24 h from create (D10). */
  expires_at: number;
  /** Unix **seconds**. */
  created: number;
  /** `cs_…_secret_…`. Never log this. */
  client_secret: string;
}

/**
 * {@link Stripe.retrieveCheckoutSession}'s answer, shaped like
 * {@link PaymentIntentResult} so the same `if (result.error)` narrowing
 * works. It never rejects — see the package README's "Errors".
 */
export type CheckoutSessionResult =
  | { checkoutSession: CheckoutSession; error?: undefined }
  | { checkoutSession?: undefined; error: StripeError };

/**
 * What vpay's page sends the parent when the payer is finished
 * (`{type:'vpay:complete', session, status}`, D8).
 *
 * Both members are strings on the wire and a message whose `session` or
 * `status` is anything else is **ignored**, not coerced: this crosses an
 * origin boundary, and a callback fired with a half-understood payload is
 * worse than one that did not fire.
 */
export interface EmbeddedCheckoutCompleteEvent {
  /** The `cs_…` that completed. Look the outcome up server-side; do not trust `status` alone. */
  session: string;
  /** The session's {@link CheckoutSessionStatus} as the page saw it. */
  status: string;
}

/** Options for {@link Stripe.initEmbeddedCheckout}. */
export interface InitEmbeddedCheckoutOptions {
  /**
   * Returns the session's `client_secret` — the merchant's own server call
   * to `POST /v1/checkout/sessions`, proxied through its page. Called once
   * per {@link Stripe.initEmbeddedCheckout}; a rejection propagates.
   */
  fetchClientSecret: () => Promise<string>;
  /**
   * Called when vpay's page reports the payer is finished. **Not** proof of
   * payment: it is a message from an iframe, so treat it as a cue to
   * re-read the session from the merchant's own server.
   */
  onComplete?: ((event: EmbeddedCheckoutCompleteEvent) => void) | undefined;
}

/**
 * The handle {@link Stripe.initEmbeddedCheckout} resolves to.
 *
 * Member for member `@stripe/stripe-js`'s own `StripeEmbeddedCheckout` —
 * pinned as a compile-time assertion in `src/compat.test.ts`, in both
 * directions, because this is the one part of Checkout where the two
 * packages' shapes genuinely do coincide.
 */
export interface EmbeddedCheckout {
  /** Attaches the frame to a CSS selector or an element. */
  mount(location: string | HTMLElement): void;
  /** Detaches the frame. The handle stays usable — {@link mount} may be called again. */
  unmount(): void;
  /** Detaches the frame, drops the `message` listener, and makes the handle unusable. */
  destroy(): void;
}
