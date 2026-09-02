/**
 * Wire types for vpay's `/v1` resource contract
 * (docs/api/README.md, docs/flows/merchant-auth.md's "Objects" table).
 */

/**
 * The five-value state machine `vpay_core::state::IntentStatus` defines
 * (docs/flows/payment-lifecycle.md). There is deliberately no `failed`
 * status — a rail failure returns the intent to
 * `requires_payment_method` with {@link LastPaymentError} populated.
 */
export type PaymentIntentStatus =
  | "requires_payment_method"
  | "requires_action"
  | "processing"
  | "succeeded"
  | "canceled";

/** Rail codes this SDK's `payment_method_types` values may hold. */
export type PaymentMethodType = "mtn_momo" | "orange_money";

/** Redirect-rail next step, Stripe's own `next_action.redirect_to_url` shape. */
export interface NextAction {
  type: "redirect_to_url";
  redirect_to_url: {
    url: string;
    return_url: string | null;
  };
}

/**
 * The closed failure-code vocabulary `vpay_core::failure` owns
 * (docs/flows/failures.md). Adapters map rail-specific error strings into
 * this list; merchants integrate against it once.
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

export interface PaymentIntent {
  id: string;
  object: "payment_intent";
  /** Integer minor units. XAF is zero-decimal: 5000 means 5,000 FCFA. */
  amount: number;
  /** Lowercase ISO 4217 code, e.g. `xaf`. */
  currency: string;
  status: PaymentIntentStatus;
  payment_method_types: string[];
  next_action: NextAction | null;
  last_payment_error: LastPaymentError | null;
  metadata: Record<string, string>;
  description: string | null;
  /** Unix seconds. */
  created: number;
  livemode: boolean;
}

export type RefundStatus = "pending" | "succeeded" | "failed" | "canceled";

export interface Refund {
  id: string;
  object: "refund";
  amount: number;
  currency: string;
  payment_intent: string;
  status: RefundStatus;
  reason: string | null;
  metadata: Record<string, string>;
  created: number;
}

/**
 * The real Stripe event types docs/flows/webhooks.md commits to. A custom
 * type would be silently dropped by a merchant's exhaustive `switch` over
 * Stripe's own typed union — this SDK types `type` as `string` rather than
 * this literal union so an event carrying a type it does not yet know about
 * is still deliverable, and narrows with the exported guards below instead.
 */
export type KnownEventType =
  | "payment_intent.created"
  | "payment_intent.processing"
  | "payment_intent.succeeded"
  | "payment_intent.payment_failed"
  | "payment_intent.canceled"
  | "charge.refunded"
  | "charge.refund.updated";

export interface Event {
  id: string;
  object: "event";
  type: string;
  /** Unix seconds. */
  created: number;
  livemode: boolean;
  /**
   * Kept as raw JSON with typed accessors below, rather than a typed union,
   * so an event carrying an object shape this SDK does not model is still
   * deliverable (docs/flows/merchant-auth.md, "Objects").
   */
  data: { object: unknown };
}

/** Narrows a {@link PaymentIntent}-shaped value out of `event.data.object`. */
export function isPaymentIntentEvent(
  event: Event,
): event is Event & { data: { object: PaymentIntent } } {
  return event.type.startsWith("payment_intent.");
}

/** Narrows a {@link Refund}-shaped value out of `event.data.object`. */
export function isRefundEvent(
  event: Event,
): event is Event & { data: { object: Refund } } {
  return event.type.startsWith("charge.refund");
}

export interface BalanceAmount {
  amount: number;
  currency: string;
}

export interface Balance {
  object: "balance";
  available: BalanceAmount[];
  pending: BalanceAmount[];
}

export interface List<T> {
  object: "list";
  data: T[];
  has_more: boolean;
  url: string;
}

/**
 * A `type` alias, not an `interface`, on purpose: TypeScript gives an
 * anonymous object type an *implicit index signature* and an interface none,
 * so only this form is assignable to the encoder's
 * `Record<string, FormValue>` without an `as unknown as` cast.
 *
 * Every optional property is written `?: T | undefined` rather than `?: T`,
 * so that a consumer compiling with `exactOptionalPropertyTypes` (as this
 * repo does) can pass a variable that is legitimately `T | undefined`.
 */
export type ListParams = {
  limit?: number | undefined;
  starting_after?: string | undefined;
  ending_before?: string | undefined;
};

export interface CreatePaymentIntentParams {
  /**
   * Integer minor units. A non-integer, a negative, or anything past
   * `Number.MAX_SAFE_INTEGER` throws `TypeError` before any request.
   */
  amount: number;
  currency: string;
  payment_method_types: PaymentMethodType[];
  metadata?: Record<string, string> | undefined;
  description?: string | undefined;
}

/** A `type` alias for the same index-signature reason as {@link ListParams}. */
export type MtnMomoPaymentMethodData = {
  type: "mtn_momo";
  mtn_momo: { msisdn: string };
};

/** A `type` alias for the same index-signature reason as {@link ListParams}. */
export type OrangeMoneyPaymentMethodData = {
  type: "orange_money";
};

export type ConfirmPaymentIntentParams =
  | { payment_method_data: MtnMomoPaymentMethodData }
  | { payment_method_data: OrangeMoneyPaymentMethodData; return_url: string };

export interface CreateRefundParams {
  payment_intent: string;
  /** Integer minor units. Omit for a full refund. */
  amount?: number | undefined;
  reason?: string | undefined;
  metadata?: Record<string, string> | undefined;
}

/**
 * Written flat rather than as `ListParams & { … }` for the same
 * index-signature reason as {@link ListParams}: only an anonymous object type
 * gets an implicit index signature, and an intersection of aliases does not.
 */
export type ListEventsParams = {
  limit?: number | undefined;
  starting_after?: string | undefined;
  ending_before?: string | undefined;
  type?: string | undefined;
};

/** Per-call options accepted by every resource method that issues a `POST`. */
export interface RequestOptions {
  /** Caller-supplied idempotency key; a fresh UUIDv4 is generated when omitted. */
  idempotencyKey?: string | undefined;
}
