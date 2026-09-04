/**
 * The wire shapes this page reads, from
 * `docs/plans/2026-09-04-step9-hosted-checkout.md`'s "The wire contract".
 *
 * The PaymentIntent types are **imported** from `@vpay/stripe-js` rather than
 * restated: that package's `PaymentIntent` is already pinned against
 * `vpay_api::model::PaymentIntentWithSecret`'s twelve keys plus the secret,
 * and a second copy here would be a second thing to keep in step with the
 * server.
 *
 * `PublicPaymentIntent` is the one shape that package does not have: the
 * return route (D6) renders the intent **without** its `client_secret`,
 * because the `return_token` in the URL authorises reading and polling and
 * nothing else. Modelling that as an `Omit` rather than as an optional field
 * is what stops the return page from ever calling `confirm`: there is no
 * secret on the object to pass.
 */
import type { PaymentIntent } from '@vpay/stripe-js';

export type { PaymentIntent };

/** The intent as the return route renders it: every public key, no secret. */
export type PublicPaymentIntent = Omit<PaymentIntent, 'client_secret'>;

/** D10: `open`, `complete` or `expired`. Nothing else is a session status. */
export type CheckoutSessionStatus = 'open' | 'complete' | 'expired';

/** D10: `unpaid`, `paid` or `failed`. */
export type CheckoutSessionPaymentStatus = 'unpaid' | 'paid' | 'failed';

export type CheckoutUiMode = 'hosted' | 'embedded';

/**
 * `checkout.session`'s own fields — everything except the intent.
 *
 * The `payment_intent` member is deliberately **not** here. On the merchant
 * surface it is the intent's id (`"pi_…"`); on the two browser reads it is
 * the **expanded intent object**, Stripe's `expand` convention. Splitting
 * the member out of the base is what lets the two browser views below type
 * their own expansion without either of them describing a shape the server
 * does not send.
 *
 * `client_secret` is optional and only ever populated on the session read
 * (`GET /v1/browser/checkout/sessions/{id}`); the return route omits it.
 * `url` is `null` for an embedded session.
 */
export interface CheckoutSession {
  id: string;
  object: 'checkout.session';
  livemode: boolean;
  ui_mode: CheckoutUiMode;
  status: CheckoutSessionStatus;
  payment_status: CheckoutSessionPaymentStatus;
  success_url: string | null;
  cancel_url: string | null;
  return_url: string | null;
  url: string | null;
  expires_at: number;
  created: number;
  client_secret?: string | undefined;
}

/** The merchant's display name — the only thing about the merchant this page shows. */
export interface CheckoutMerchant {
  name: string;
}

/**
 * `GET /v1/browser/checkout/sessions/{cs_id}?key&client_secret`.
 *
 * The session object itself, with `payment_intent` **expanded** and its own
 * `client_secret` inside it, plus the merchant's display name — the one
 * thing about the merchant this page shows. Lane 1 owns the server side of
 * this shape; see `docs/plans/step9-notes/lane-3.md`.
 *
 * `merchant` is **optional**, and that is deliberate rather than lax. The
 * name is a nicety on a payment page; the amount, the reference and the
 * rail are not. A server that renders the member as `merchant_name`, omits
 * it for a merchant that has set no display name, or ships a version where
 * it has not landed yet must leave the payer able to pay — so the shape
 * this page *requires* is the shape it cannot work without, and the name is
 * read defensively (`merchantOf` in `machine.ts`) rather than asserted
 * here. Where it is present it is `{ name: string }` and nothing else.
 */
export interface CheckoutSessionView extends CheckoutSession {
  payment_intent: PaymentIntent;
  merchant?: CheckoutMerchant | undefined;
}

/**
 * `GET /v1/browser/checkout/sessions/{cs_id}/return?key&t=…`.
 *
 * The same object, with the same expansion, and **no secret on either
 * half** — neither the session's nor the intent's. That is the whole reason
 * the return page cannot confirm anything: the credential it would need is
 * not in the response.
 */
export interface CheckoutReturnView extends CheckoutSession {
  payment_intent: PublicPaymentIntent;
  /** Optional, for the same reason as {@link CheckoutSessionView.merchant}. */
  merchant?: CheckoutMerchant | undefined;
}

/** `GET /v1/browser/checkout/origins?key`. */
export interface CheckoutOriginsView {
  origins: string[];
}

/**
 * Why this page cannot go on.
 *
 * A closed vocabulary, each member of which is a `MessageKey` in both
 * dictionaries, so a failure is always rendered in the payer's language
 * rather than by echoing a server message written in English (and possibly
 * carrying detail no payer should read).
 */
export type CheckoutErrorCode =
  | 'error.session_not_found'
  | 'error.network'
  | 'error.unexpected'
  | 'error.missing_key'
  | 'error.missing_secret'
  | 'error.missing_return_token';

export interface CheckoutError {
  code: CheckoutErrorCode;
  /**
   * The server's own `error.code` (`resource_missing`, …) when there was
   * one. Rendered into a `data-` attribute for support, never into prose —
   * and never the server's `message`, which this page does not control.
   */
  serverCode?: string | undefined;
}

export type Result<T> = { ok: true; value: T } | { ok: false; error: CheckoutError };
