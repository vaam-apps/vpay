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
  /**
   * `pi_…_secret_…` — the payer credential `/v1/browser` accepts to confirm
   * this intent from a browser (hand it to `@vaam-apps/vpay-stripe-js`).
   *
   * Present **only** on `create()` and `retrieve()` responses (Step 5c's
   * D2, `vpay_api::model::PaymentIntentWithSecret`); absent — this property
   * is missing entirely, not `null` — on every `list()` item and on
   * `event.data.object`, so it never reaches a merchant's listing view or a
   * webhook body that's stored and forwarded at-least-once. Never log this
   * value.
   */
  client_secret?: string;
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
 *
 * `checkout.session.expired` is emitted when vpay's hourly sweep moves a
 * session past its 24-hour horizon (Step 9's D10); its `data.object` is a
 * {@link CheckoutSession}, which makes it the only member here whose payload
 * is neither a {@link PaymentIntent} nor a {@link Refund}. Narrow it with
 * {@link isCheckoutSessionEvent}.
 */
export type KnownEventType =
  | "payment_intent.created"
  | "payment_intent.processing"
  | "payment_intent.succeeded"
  | "payment_intent.payment_failed"
  | "payment_intent.canceled"
  | "charge.refunded"
  | "charge.refund.updated"
  | "checkout.session.expired";

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

/**
 * Narrows a {@link CheckoutSession}-shaped value out of `event.data.object`.
 *
 * Matched on the `checkout.session.` prefix rather than on the one literal
 * type vpay emits today, exactly as the two guards above are: a later
 * `checkout.session.*` type carries the same object, and a guard that had to
 * be edited for each one is a guard that will be out of date before the
 * `switch` that calls it is.
 *
 * The session on an event **never** carries `client_secret`, and its `url` is
 * always `null`: both would put a live payer credential in a body that is
 * stored, delivered at-least-once and replayed
 * (`vpay_api::model::CheckoutSessionObject::expired_snapshot`). So
 * {@link CheckoutSession.url} being `null` here does not mean the session was
 * embedded — read {@link CheckoutSession.ui_mode} for that.
 */
export function isCheckoutSessionEvent(
  event: Event,
): event is Event & { data: { object: CheckoutSession } } {
  return event.type.startsWith("checkout.session.");
}

/**
 * A Checkout Session's lifecycle state (Step 9's D10) — vpay's own three
 * values, not Stripe's. There is no `failed`: a session whose intent
 * reached a terminal non-success state is `expired` with
 * {@link CheckoutSession.payment_status} `failed`.
 */
export type CheckoutSessionStatus = "open" | "complete" | "expired";

/** Whether the session's intent has been paid. D10's second axis. */
export type CheckoutPaymentStatus = "unpaid" | "paid" | "failed";

/**
 * Which surface the session is rendered on. `hosted` gets a
 * {@link CheckoutSession.url} to send the payer to; `embedded` gets a
 * `client_secret` to hand `@vaam-apps/vpay-stripe-js`'s `initEmbeddedCheckout`.
 */
export type CheckoutSessionUiMode = "hosted" | "embedded";

/**
 * A `/v1/checkout/sessions` object (Step 9's D1).
 *
 * The session **references** an existing PaymentIntent; it never creates
 * one. Amount, currency and the allowed rails stay on the intent, where
 * every existing invariant already guards them — which is why there is no
 * `line_items`, no `mode` and no `amount_total` here, however Stripe-shaped
 * the rest of the field names are.
 */
export interface CheckoutSession {
  /** `cs_…`. */
  id: string;
  object: "checkout.session";
  livemode: boolean;
  /**
   * The `pi_…` this session drives — an **id**, on every `/v1` route.
   *
   * `@vaam-apps/vpay-stripe-js`'s `CheckoutSession` types the same field as the whole
   * expanded {@link PaymentIntent}, because the *browser* session read
   * expands it (the checkout page confirms and polls the intent through the
   * browser routes and cannot fetch it separately). That is a deliberate
   * per-route difference, ruled on 2026-09-04, not a skew between the
   * SDKs: nothing on `/v1` ever expands it, so a merchant integration reads
   * an id here and calls `paymentIntents.retrieve` if it wants the object.
   */
  payment_intent: string;
  ui_mode: CheckoutSessionUiMode;
  status: CheckoutSessionStatus;
  payment_status: CheckoutPaymentStatus;
  /** Hosted mode only; `null` when embedded. May carry `{CHECKOUT_SESSION_ID}` (D5). */
  success_url: string | null;
  /** Hosted mode only; `null` when embedded. */
  cancel_url: string | null;
  /** Embedded mode only; `null` when hosted. */
  return_url: string | null;
  /** The page to send the payer to, hosted mode only; `null` when embedded. */
  url: string | null;
  /** Unix seconds. 24 h from create. */
  expires_at: number;
  /** Unix seconds. */
  created: number;
  /**
   * `cs_…_secret_…` — the payer credential the browser presents to read
   * this session, and what `initEmbeddedCheckout`'s `fetchClientSecret`
   * must return.
   *
   * Present **only** on `create()` and `retrieve()` responses; absent —
   * missing entirely, not `null` — on every `list()` item. It is a
   * different credential from the intent's `client_secret`: it authorises
   * reading this session, never confirming the intent.
   *
   * Never log this value. Unlike {@link PaymentIntent}, whose plain
   * interface prints its secret through `console.log`, every
   * `CheckoutSession` this SDK returns carries a custom
   * `util.inspect` representation that redacts this field — see
   * `src/resources/checkout-sessions.ts`. `JSON.stringify` is deliberately
   * left faithful: an embedded integration has to serialise the secret to
   * get it to the browser at all.
   */
  client_secret?: string;
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

/**
 * Who a mobile-money number is registered to, or the fact that the rail has
 * no record of it — `GET /v1/account_holders` (issue #47).
 *
 * `name: null` means **the rail answered and does not know this number**. It
 * does not mean "we could not ask": that throws. A caller matching a
 * nominated refund destination against a buyer's verified name must refuse
 * on both — but only one of them is the buyer's problem.
 *
 * A name and nothing else, by construction: the rail's body carries more (a
 * birthdate, a locale, a gender) and vpay projects it away before it can be
 * logged or stored. There is no `id`, because nothing is stored to address
 * later.
 */
export interface AccountHolder {
  object: "account_holder";
  /**
   * The rail the question was put to, echoed back.
   *
   * `string` and not {@link PaymentMethodType}, for the reason
   * `PaymentIntent.payment_method_types` is `string[]`: the request side is
   * closed and the response side must stay readable for a rail this SDK
   * version predates.
   */
  payment_method_type: string;
  /** The registered holder's name, or `null` when the rail has no record. */
  name: string | null;
  /**
   * `true` exactly when `name` is present.
   *
   * Not a claim that anything was cryptographically verified — it says the
   * rail named a holder.
   */
  verified: boolean;
}

/**
 * `GET /v1/account_holders` request fields.
 *
 * A `type` alias for {@link ListParams}' index-signature reason. Both fields
 * are required and neither is optional: a lookup with no number or no rail
 * is not a narrower query, it is not a query.
 *
 * **`payment_method_type`, snake_case, like every other params type in this
 * package** — the wire spelling, which is what the encoder walks and what
 * `sdks/rust/tests/resources.rs` pins byte for byte. A camelCase
 * `paymentMethodType` would be the only camelCase request field in the SDK
 * and would need a translation step the others do not have.
 */
export type RetrieveAccountHolderParams = {
  /**
   * The mobile-money number, in any of the three shapes vpay accepts:
   * `+2376XXXXXXXX`, `2376XXXXXXXX` or the national `6XXXXXXXX`. Validated
   * by the server, not here — see `AccountHoldersResource.retrieve`.
   */
  msisdn: string;
  /** Which rail to ask. Closed on the request side, as a create's list is. */
  payment_method_type: PaymentMethodType;
};

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

/**
 * `POST /v1/checkout/sessions` request fields.
 *
 * The URL rules are the server's and are not duplicated here: `success_url`
 * and `cancel_url` are required for `hosted` and refused for `embedded`,
 * `return_url` the other way round, all http(s) and at most 2048
 * characters, `https` only under livemode. This SDK sends what it is given
 * and lets the server say no — a second copy of those rules here would be a
 * second thing to keep in step with them, and would refuse a combination a
 * later server version allows.
 */
export interface CreateCheckoutSessionParams {
  /** The `pi_…` to drive. Required; the session never creates one. */
  payment_intent: string;
  /** Defaults to `hosted` server-side when omitted. */
  ui_mode?: CheckoutSessionUiMode | undefined;
  /** Where to send a payer who paid. Hosted mode. May contain `{CHECKOUT_SESSION_ID}`. */
  success_url?: string | undefined;
  /** Where to send a payer who gave up. Hosted mode. */
  cancel_url?: string | undefined;
  /** Where vpay's framed page forwards the payer at the end. Embedded mode. */
  return_url?: string | undefined;
}

/**
 * `GET /v1/checkout/sessions` query parameters. A `type` alias for the same
 * index-signature reason as {@link ListParams}.
 */
export type ListCheckoutSessionsParams = {
  limit?: number | undefined;
  starting_after?: string | undefined;
  ending_before?: string | undefined;
  /** Only sessions for this `pi_…`. */
  payment_intent?: string | undefined;
};

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
