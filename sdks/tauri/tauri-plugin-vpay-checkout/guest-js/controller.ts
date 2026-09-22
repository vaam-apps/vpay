/**
 * The state machine (D1/D2/D4), ported from
 * `sdks/flutter/vpay_checkout_flutter/lib/src/checkout_controller.dart`.
 *
 * **Pure**: no HTTP of its own (every read goes through the injected
 * `@vaam-apps/vpay-stripe-js` client) and no ambient timer (every wait goes
 * through the injected {@link VpayClock}). Two independent properties this
 * file exists to hold, both parity rows on their own:
 *
 * - **D1: the outcome is never read off a URL.**
 *   {@link CheckoutController.resolveAfterStopUrlReached} does not accept or
 *   consult the URL that was reached — the signature itself is the proof.
 *   Only `retrievePaymentIntent` decides.
 * - **D4: dismissal polls before it reports.**
 *   {@link CheckoutController.resolveAfterDismissal} runs the *same* poll
 *   loop on a shorter budget. There is no separate "just say canceled" code
 *   path for a dismissal to fall into.
 */
import type {
  CheckoutSession,
  PaymentIntent,
  Stripe,
} from "@vaam-apps/vpay-stripe-js";

import {
  embeddedSessionNotSupportedError,
  unexpectedResponseError,
  type VpayError,
} from "./errors.js";
import { StopUrlSpec } from "./stop-url.js";
import type {
  CheckoutStopUrl,
  VpayCheckoutResult,
  VpayClock,
} from "./types.js";

/**
 * The budget {@link CheckoutController.resolveAfterStopUrlReached} polls on
 * — `sdks/stripe-js`'s `waitForPaymentIntent` default, reused rather than
 * invented: three minutes, at a two-second interval.
 */
export const DEFAULT_STOP_URL_POLL_TIMEOUT_MS = 180_000;
export const DEFAULT_STOP_URL_POLL_INTERVAL_MS = 2_000;

/** D4: "a **short** poll (a few seconds)" — before reporting anything for a dismissal. */
export const DEFAULT_DISMISSAL_POLL_TIMEOUT_MS = 5_000;
export const DEFAULT_DISMISSAL_POLL_INTERVAL_MS = 1_000;

/** What the pre-flight (D2) bought, once a session read succeeded and the session was not `embedded`. */
export interface CheckoutPreflightReady {
  sessionId: string;
  paymentIntentId: string;
  /**
   * D2 item 1: the polling credential, read while the session is `open` —
   * good for the rest of the intent's life thereafter.
   *
   * **`session.payment_intent.client_secret`, never
   * `session.client_secret`.** The latter is typed `string` by
   * `@vaam-apps/vpay-stripe-js` and the server never sends it (the Flutter
   * e2e suite found this on 2026-09-14), so reading it would put
   * `undefined` into a URL and produce a uniform 404 on every poll.
   */
  intentClientSecret: string;
  successStopUrl: StopUrlSpec | null;
  cancelStopUrl: StopUrlSpec | null;
  /** The two rules above, minus the ones the session did not configure. */
  stopUrls: CheckoutStopUrl[];
}

/**
 * The pre-flight's outcome: either a {@link CheckoutPreflightReady}, or a
 * typed {@link VpayError} — a bad key, an expired link, or an `embedded`
 * session (D2 item 3), all refused before any window opens.
 */
export type CheckoutPreflight =
  { ok: true; ready: CheckoutPreflightReady } | { ok: false; error: VpayError };

/** A poll budget. Both call sites pass one; both defaults are named above. */
export interface PollBudget {
  timeoutMs?: number | undefined;
  intervalMs?: number | undefined;
}

/**
 * True for the three states a payment intent stops moving in.
 *
 * There is no `failed` status in `vpay_core::state::IntentStatus`: a rail
 * failure returns the intent to `requires_payment_method` with
 * `last_payment_error` populated, which is why this predicate has to look
 * at both fields. A **bare** `requires_payment_method` is an intent nobody
 * has confirmed yet, so it keeps polling.
 */
function hasStoppedMoving(intent: PaymentIntent): boolean {
  if (intent.status === "succeeded" || intent.status === "canceled") {
    return true;
  }
  return (
    intent.status === "requires_payment_method" &&
    intent.last_payment_error !== null &&
    intent.last_payment_error !== undefined
  );
}

/** The pure state machine: pre-flight, and the one poll loop both outcomes resolve through. */
export class CheckoutController {
  readonly #stripe: Stripe;
  readonly #clock: VpayClock;

  constructor(deps: { stripe: Stripe; clock: VpayClock }) {
    this.#stripe = deps.stripe;
    this.#clock = deps.clock;
  }

  /**
   * D2: one pre-flight session read; stop URLs derived, not configured.
   *
   * `sessionClientSecret` is the `cs_…_secret_…` read out of the session
   * URL's fragment (D6) before this is called.
   */
  async preflight(sessionClientSecret: string): Promise<CheckoutPreflight> {
    const result =
      await this.#stripe.retrieveCheckoutSession(sessionClientSecret);
    if (result.error !== undefined) {
      return { ok: false, error: result.error };
    }
    const session: CheckoutSession = result.checkoutSession;
    if (session.ui_mode === "embedded") {
      // D2 item 3, and it happens **here** — before any window exists.
      return { ok: false, error: embeddedSessionNotSupportedError(session.id) };
    }
    const successStopUrl = StopUrlSpec.fromConfigured(
      session.success_url,
      session.id,
    );
    const cancelStopUrl = StopUrlSpec.fromConfigured(
      session.cancel_url,
      session.id,
    );
    return {
      ok: true,
      ready: {
        sessionId: session.id,
        paymentIntentId: session.payment_intent.id,
        intentClientSecret: session.payment_intent.client_secret,
        successStopUrl,
        cancelStopUrl,
        stopUrls: [successStopUrl, cancelStopUrl]
          .filter((spec): spec is StopUrlSpec => spec !== null)
          .map((spec) => spec.toJSON()),
      },
    };
  }

  /**
   * D1: called once a host reports a navigation matched a stop URL.
   *
   * Deliberately takes **no URL parameter** — see this file's header — and
   * resolves purely from polling `retrievePaymentIntent`.
   */
  resolveAfterStopUrlReached(
    ready: CheckoutPreflightReady,
    budget: PollBudget = {},
  ): Promise<VpayCheckoutResult> {
    return this.#resolve(
      ready,
      budget.timeoutMs ?? DEFAULT_STOP_URL_POLL_TIMEOUT_MS,
      budget.intervalMs ?? DEFAULT_STOP_URL_POLL_INTERVAL_MS,
    );
  }

  /**
   * D4: called once a host reports the payer dismissed the window. Runs the
   * identical poll loop {@link resolveAfterStopUrlReached} does, only
   * shorter — there is no separate "report canceled" branch here.
   */
  resolveAfterDismissal(
    ready: CheckoutPreflightReady,
    budget: PollBudget = {},
  ): Promise<VpayCheckoutResult> {
    return this.#resolve(
      ready,
      budget.timeoutMs ?? DEFAULT_DISMISSAL_POLL_TIMEOUT_MS,
      budget.intervalMs ?? DEFAULT_DISMISSAL_POLL_INTERVAL_MS,
    );
  }

  /** The one poll loop. Both public entry points are this, with a budget. */
  async #resolve(
    ready: CheckoutPreflightReady,
    timeoutMs: number,
    intervalMs: number,
  ): Promise<VpayCheckoutResult> {
    const deadline = this.#clock.now() + timeoutMs;
    for (;;) {
      const result = await this.#stripe.retrievePaymentIntent(
        ready.intentClientSecret,
      );
      if (result.error !== undefined) {
        return {
          kind: "unresolved",
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
          error: result.error,
        };
      }
      const intent: PaymentIntent = result.paymentIntent;
      if (intent.id !== ready.paymentIntentId) {
        // The request was addressed by `ready.intentClientSecret`, whose
        // own prefix *is* `ready.paymentIntentId`, so these can only
        // disagree if the server answered about a different object.
        // Reporting that object's `succeeded` as this session's outcome is
        // precisely the "claims success it did not observe" failure D1
        // exists to refuse, so it is an unresolved, not a result. Neither
        // id is interpolated: one of them came off the wire.
        return {
          kind: "unresolved",
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
          error: unexpectedResponseError(200),
        };
      }
      if (hasStoppedMoving(intent)) {
        return resultFor(ready, intent);
      }
      const remaining = deadline - this.#clock.now();
      if (remaining <= 0) {
        return {
          kind: "pending",
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
        };
      }
      await this.#clock.delay(Math.min(remaining, intervalMs));
    }
  }
}

/** The terminal-status mapping. Reached only for an intent {@link hasStoppedMoving} accepted. */
function resultFor(
  ready: CheckoutPreflightReady,
  intent: PaymentIntent,
): VpayCheckoutResult {
  const { sessionId, paymentIntentId } = ready;
  switch (intent.status) {
    case "succeeded":
      return { kind: "succeeded", sessionId, paymentIntentId };
    case "canceled":
      return { kind: "canceled", sessionId, paymentIntentId };
    case "requires_payment_method":
      return {
        kind: "failed",
        sessionId,
        paymentIntentId,
        code: intent.last_payment_error?.code ?? null,
        providerMessage: intent.last_payment_error?.message ?? null,
      };
    default:
      // `hasStoppedMoving` is false for `requires_action` and `processing`,
      // so the loop never reaches here with one of them. Kept as a total
      // `default` that answers `pending` — the honest answer for a status
      // this version does not know — rather than as a throw, because a
      // sixth status added server-side must not turn a settled payment
      // into an exception on the payer's screen.
      return { kind: "pending", sessionId, paymentIntentId };
  }
}
