/**
 * The shapes `/dash/v1` answers in, and the one shape a refusal becomes.
 *
 * # What this file is since VP-6a (ADR-0036)
 *
 * It used to be every call the app made to vpay, and most of it was the
 * sign-in: the password and TOTP routes, the authorization-code leg, the token
 * mint and the SSO exchange. All of those routes are deleted. What is left is
 * what a console that reads `/dash/v1` will still need whatever carries the
 * request: the object types, and {@link ApiFailure} with its builders. No call
 * is made from here, because the console now signs every request with a paired
 * key and **this app cannot do that yet** (VP-6b).
 *
 * # Why the request id is threaded through everything
 *
 * vpay emits `request-id` (Stripe's spelling) and `x-request-id` with one
 * value on every response, and `vpay-api`'s error envelope deliberately
 * carries no `request_id` field because the header already does. It is the
 * only thing that distinguishes one refusal from another for a person who has
 * to quote something to an operator, so it is carried on the failure and
 * rendered beside the message.
 */

/** The two spellings vpay emits, in the order this app prefers them. */
const REQUEST_ID_HEADERS = ["request-id", "x-request-id"] as const;

/** A call that did not succeed, in the words a screen can render. */
export interface ApiFailure {
  /** HTTP status. `401` is the one every caller branches on. */
  readonly status: number;
  /** `error.message` from the envelope, or a sentence written here. */
  readonly message: string;
  /** `request-id`, or `null` if the response carried none. */
  readonly requestId: string | null;
}

/** Success or {@link ApiFailure} — never a thrown error for a refusal. */
export type ApiResult<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly failure: ApiFailure };

/** One `payment_intent`, as `/dash/v1` renders it (no `client_secret`). */
export interface PaymentIntentObject {
  readonly id: string;
  readonly object: "payment_intent";
  readonly amount: number;
  readonly currency: string;
  readonly status: string;
  readonly payment_method_types: readonly string[];
  readonly last_payment_error: {
    readonly code?: string;
    readonly message?: string;
  } | null;
  readonly description: string | null;
  readonly customer: string | null;
  readonly created: number;
  readonly livemode: boolean;
}

/** `GET /dash/v1/payment_intents`' envelope. */
export interface PaymentIntentList {
  readonly object: string;
  readonly data: readonly PaymentIntentObject[];
  readonly has_more: boolean;
}

/** The charge, as `/dash/v1`'s detail read renders it. */
export interface ChargeSummary {
  readonly object: "charge";
  readonly id: string;
  readonly provider_code: string;
  readonly provider_reference_id: string;
  readonly provider_txn_id: string | null;
  readonly state: string;
  readonly amount: number;
  readonly currency: string;
  /**
   * `null` on every row this deployment has ever written — nothing writes
   * `charges.payer_ref_masked` (`docs/status.md`). Rendered from the column
   * and never derived; see `src/components/payment-detail.tsx`.
   */
  readonly payer_ref_masked: string | null;
  readonly failure_code: string | null;
  readonly failure_raw: string | null;
  readonly created: number;
  readonly updated: number;
}

/** One refund, as `/v1` renders it. */
export interface RefundObject {
  readonly id: string;
  readonly amount: number;
  readonly currency: string;
  readonly status: string;
  readonly reason: string | null;
  readonly created: number;
}

/** One line of the timeline. */
export interface TimelineEvent {
  readonly object: "event";
  readonly id: string;
  readonly type: string;
  readonly object_id: string;
  readonly created: number;
}

/** `GET /dash/v1/payment_intents/{id}`'s envelope. */
export interface PaymentDetail {
  readonly object: "dashboard.payment_detail";
  readonly payment_intent: PaymentIntentObject;
  readonly charge: ChargeSummary | null;
  readonly refunds: readonly RefundObject[];
  readonly events: readonly TimelineEvent[];
}

/** The request id from either spelling, or `null`. */
export function requestIdOf(response: Response): string | null {
  for (const header of REQUEST_ID_HEADERS) {
    const value = response.headers.get(header);
    if (value !== null && value.trim().length > 0) {
      return value.trim();
    }
  }
  return null;
}

/**
 * `error.message` out of an already-decoded vpay envelope, or the sentence
 * naming the status.
 *
 * A body this app cannot parse is **not** rendered raw: an upstream proxy's
 * HTML error page in an alert is both unreadable and a disclosure of what sits
 * in front of vpay.
 */
export function envelopeMessage(status: number, body: unknown): string {
  const envelope = (body as { error?: { message?: unknown } } | null)?.error;
  if (
    typeof envelope?.message === "string" &&
    envelope.message.trim().length > 0
  ) {
    return envelope.message.trim();
  }
  return `vpay answered ${status}.`;
}

/**
 * A network failure, as an {@link ApiFailure} rather than a thrown error.
 *
 * `status: 0` because there was no response. Every caller already branches on
 * the failure shape, and a `fetch` that rejected inside a server component
 * would otherwise render Next's own error page — which tells a staff member
 * nothing and an operator less.
 */
export function unreachable(error: unknown): ApiFailure {
  const detail = error instanceof Error ? error.message : String(error);
  return {
    status: 0,
    message: `vpay could not be reached (${detail}).`,
    requestId: null,
  };
}
