/**
 * The impure half of the page: turns network answers into
 * {@link CheckoutEvent}s and performs the two side effects that leave this
 * document — a top-level navigation, and a `postMessage` to the framer.
 *
 * It owns no rendering and no timers a component could not own; what it owns
 * is the ordering, which is where a payment page goes wrong. In particular:
 *
 * - `confirm` is called **once** per attempt and only from a state that has
 *   a rail; a second press while `confirming` is dropped by the reducer.
 * - the redirect for a redirect rail is performed **after** the machine has
 *   recorded it, so a payer who comes back to the tab mid-navigation sees
 *   "taking you to Orange Money" rather than the form again.
 * - `vpay:complete` is posted after a re-read of the session, so the status
 *   the parent receives is the session's own, not this page's guess from the
 *   intent.
 */
import type { Stripe, StripeError } from "@vaam-apps/vpay-stripe-js";

import type { MessageKey } from "../i18n/index";
import type { BrowserCheckoutApi, SessionCredentials } from "./api";
import type { FrameChannel } from "./frame";
import { normalizeCameroonMsisdn } from "./msisdn";
import {
  INITIAL_STATE,
  contextOf,
  reduce,
  type CheckoutEvent,
  type CheckoutState,
} from "./machine";
import type { SupportedRail } from "./rails";
import type { CheckoutError, PaymentIntent } from "./types";

/** Maps a `@vaam-apps/vpay-stripe-js` error onto a message this page can show in either language. */
export function messageForStripeError(error: StripeError): MessageKey {
  if (error.type === "api_connection_error") {
    return "error.network";
  }
  if (error.code === "resource_missing") {
    return "error.session_not_found";
  }
  return "error.unexpected";
}

/** The same, for this app's own API failures. `CheckoutErrorCode` is already a key. */
export function messageForCheckoutError(error: CheckoutError): MessageKey {
  return error.code;
}

export interface CheckoutControllerOptions {
  sessionId: string;
  credentials: SessionCredentials;
  api: BrowserCheckoutApi;
  stripe: Stripe;
  /** Performs a top-level navigation. `window.location.assign` in a browser. */
  navigate: (url: string) => void;
  /**
   * Closes this window. `window.close` in a browser, absent everywhere else.
   *
   * Only ever called for a **popup** (`channel.peer === 'opener'`), and only
   * after `vpay:complete` has been posted: a popup that stays open after the
   * payer has pressed "Back to {merchant}" is a window they now have to
   * find and close themselves, on a phone, on top of a merchant page that
   * has already moved on.
   */
  closeWindow?: (() => void) | undefined;
  /**
   * The peer to talk to, or `null`.
   *
   * Non-null in exactly two shapes: an embedded page framed by an origin the
   * merchant registered (`peer: 'parent'`), and a hosted page in a popup
   * whose opener resolved to one (`peer: 'opener'`). The peer is not
   * cosmetic — `forward` and `navigateTopLevel` both branch on it.
   */
  channel: FrameChannel | null;
  /**
   * The deployment's `checkout.allowed_methods` (`config.yaml`), or `null`
   * for "no opinion". Handed to `contextOf`, which puts it on the context
   * the pure reducer reads — see `CheckoutContext.allowedMethods`.
   */
  allowedMethods?: readonly string[] | null | undefined;
  /** Poll budget handed to `waitForPaymentIntent`. */
  pollTimeoutMs?: number | undefined;
  /** Poll interval handed to `waitForPaymentIntent`. */
  pollIntervalMs?: number | undefined;
  /**
   * Reads the window that opened this one. `() => window.opener` in a
   * browser; absent everywhere else.
   *
   * A function rather than the handle, because "is the merchant's tab still
   * open?" is a question about *now* — the payer may have closed it while
   * they were paying — and a captured value would answer about page load.
   */
  opener?: (() => Window | null) | undefined;
}

export type StateListener = (state: CheckoutState) => void;

export class CheckoutController {
  #state: CheckoutState = INITIAL_STATE;
  /** Whether `vpay:complete` has already gone out. See {@link CheckoutController.announceComplete}. */
  #announced = false;
  readonly #listeners = new Set<StateListener>();
  readonly #options: CheckoutControllerOptions;

  constructor(options: CheckoutControllerOptions) {
    this.#options = options;
  }

  get state(): CheckoutState {
    return this.#state;
  }

  subscribe(listener: StateListener): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  /** Refuses before any read: the framer is not an origin the merchant registered. */
  refuseEmbedding(): void {
    this.#dispatch({ type: "refuse", reason: "embed_not_allowed" });
  }

  /** Reads the session and enters the state it implies. Resumes a poll when one is owed. */
  async start(): Promise<void> {
    const result = await this.#options.api.readSession(
      this.#options.sessionId,
      this.#options.credentials,
    );
    if (!result.ok) {
      this.#dispatch({ type: "load_failed", error: result.error });
      return;
    }
    this.#dispatch({
      type: "loaded",
      context: contextOf(result.value, this.#options.allowedMethods ?? null),
    });
    if (this.#state.name === "waiting") {
      await this.#poll();
    } else if (this.#state.name === "outcome") {
      await this.#announceOutcome();
    }
  }

  chooseRail(rail: SupportedRail): void {
    this.#dispatch({ type: "choose_rail", rail });
  }

  back(): void {
    this.#dispatch({ type: "back" });
  }

  /**
   * The MTN path: normalise, confirm, then poll.
   *
   * The raw input never leaves this method — what goes to the rail is the
   * canonical `2376XXXXXXXX` {@link normalizeCameroonMsisdn} produced, so
   * the rail can never receive whatever spacing a payer typed.
   */
  async submitMsisdn(raw: string): Promise<void> {
    const state = this.#state;
    if (state.name !== "collect_msisdn") {
      return;
    }
    const msisdn = normalizeCameroonMsisdn(raw);
    if (msisdn === null) {
      this.#dispatch({ type: "problem", problem: "msisdn.invalid" });
      return;
    }
    const rail = state.rail;
    const clientSecret = this.#intentSecret();
    if (clientSecret === null) {
      this.#dispatch({ type: "problem", problem: "error.missing_secret" });
      return;
    }
    this.#dispatch({ type: "confirm_started" });
    const result = await this.#options.stripe.confirmMobileMoneyPayment(
      clientSecret,
      {
        type: rail.code,
        msisdn,
      },
    );
    if (result.error !== undefined) {
      this.#dispatch({
        type: "problem",
        problem: messageForStripeError(result.error),
      });
      return;
    }
    this.#dispatch({ type: "intent_updated", intent: result.paymentIntent });
    if (this.#state.name === "waiting") {
      await this.#poll();
    } else if (this.#state.name === "outcome") {
      await this.#announceOutcome();
    }
  }

  /**
   * The Orange path: confirm with `redirect: 'if_required'`, then move the
   * payer ourselves.
   *
   * `if_required` rather than letting `@vaam-apps/vpay-stripe-js` navigate, because
   * *who* navigates depends on whether this page is framed, and that is a
   * fact only this page has. Framed: ask the parent
   * (`{type:'vpay:redirect', url}`) — an iframe navigating the top-level
   * context is the behaviour browsers are removing. Top-level:
   * `location.assign` ourselves.
   */
  async startRedirect(): Promise<void> {
    const state = this.#state;
    if (state.name !== "ready_redirect") {
      return;
    }
    const clientSecret = this.#intentSecret();
    if (clientSecret === null) {
      this.#dispatch({ type: "problem", problem: "error.missing_secret" });
      return;
    }
    this.#dispatch({ type: "confirm_started" });
    const result = await this.#options.stripe.confirmPayment({
      clientSecret,
      // The rail is named on the confirm, exactly as it is for a push:
      // an intent may offer more than one, and the payer chose this one.
      // Without it the server has no way to know which rail to charge.
      confirmParams: { payment_method_data: { type: state.rail.code } },
      redirect: "if_required",
    });
    if (result.error !== undefined) {
      this.#dispatch({
        type: "problem",
        problem: messageForStripeError(result.error),
      });
      return;
    }
    const intent = result.paymentIntent;
    const url = redirectUrlOf(intent);
    if (url === null) {
      // A redirect rail that answered without a redirect. Not an error —
      // poll and let the intent say what happened.
      this.#dispatch({ type: "intent_updated", intent });
      if (this.#state.name === "waiting") {
        await this.#poll();
      } else if (this.#state.name === "outcome") {
        await this.#announceOutcome();
      }
      return;
    }
    this.#dispatch({ type: "redirect_required", url });
    this.#navigateTopLevel(url);
  }

  /** Re-runs the poll after a `waitForPaymentIntent` that could not be answered. */
  async retryPoll(): Promise<void> {
    if (this.#state.name !== "waiting") {
      return;
    }
    await this.#poll();
  }

  /**
   * Sends the payer back to the merchant. No-op unless an outcome is on
   * screen.
   *
   * Three shapes, one per place this page can be running:
   *
   * - **Framed** — ask the parent to navigate (D8). An iframe navigating
   *   itself would render the merchant's confirmation page inside its own
   *   checkout widget.
   * - **A popup** — make sure the opener has heard `vpay:complete`, then
   *   close this window. The merchant's own page is already where the payer
   *   is going back to; navigating this window to `success_url` would leave
   *   the merchant's page untouched behind a popup showing a second copy of
   *   it. If the opener has gone (the payer closed the merchant's tab), the
   *   popup navigates itself instead, so the payer is never left on a dead
   *   window.
   * - **Top-level** — this document navigates itself, as before.
   */
  forward(url: string): void {
    this.#dispatch({ type: "forward", url });
    if (this.#state.name !== "forwarding") {
      return;
    }
    const channel = this.#options.channel;
    if (channel !== null && channel.peer === "opener") {
      this.#announceComplete();
      if (this.#openerIsGone()) {
        this.#options.navigate(url);
        return;
      }
      this.#options.closeWindow?.();
      return;
    }
    this.#navigateTopLevel(url);
  }

  /**
   * Whether the window that opened this popup is still there.
   *
   * `closed` is one of the three members readable on a cross-origin `Window`
   * handle, so this is a real check rather than an optimistic one. A `null`
   * opener means the browser severed the relationship (a `rel=noopener`
   * link, a navigation that discarded it).
   */
  #openerIsGone(): boolean {
    const opener = this.#options.opener?.() ?? undefined;
    return opener === null || opener === undefined || opener.closed;
  }

  /**
   * Who performs a top-level navigation, decided in exactly one place.
   *
   * Framed: the child asks and the parent goes (D8). A `location.assign`
   * inside an iframe moves the iframe, which for a rail's page means the
   * payer types their PIN in a 300-pixel box on a merchant's site, and for
   * a `success_url` means the merchant's confirmation page renders inside
   * its own checkout widget.
   *
   * Not framed: this document navigates itself.
   */
  #navigateTopLevel(url: string): void {
    const channel = this.#options.channel;
    // A POPUP navigates itself: it is a top-level browsing context, and
    // `vpay:redirect` to an opener would send the merchant's own page to the
    // rail out from under the payer. Only a frame delegates.
    if (channel !== null && channel.peer === "parent") {
      channel.post({ type: "vpay:redirect", url });
      return;
    }
    this.#options.navigate(url);
  }

  async #poll(): Promise<void> {
    const clientSecret = this.#intentSecret();
    if (clientSecret === null) {
      this.#dispatch({ type: "problem", problem: "error.missing_secret" });
      return;
    }
    const options: { timeoutMs?: number; intervalMs?: number } = {};
    if (this.#options.pollTimeoutMs !== undefined) {
      options.timeoutMs = this.#options.pollTimeoutMs;
    }
    if (this.#options.pollIntervalMs !== undefined) {
      options.intervalMs = this.#options.pollIntervalMs;
    }
    const result = await this.#options.stripe.waitForPaymentIntent(
      clientSecret,
      options,
    );
    if (result.error !== undefined) {
      // The payer stays on the waiting screen: the payment is in flight and
      // this page has learnt nothing that says otherwise.
      this.#dispatch({
        type: "problem",
        problem: messageForStripeError(result.error),
      });
      return;
    }
    this.#dispatch({ type: "intent_updated", intent: result.paymentIntent });
    if (this.#state.name === "outcome") {
      await this.#announceOutcome();
    }
  }

  /**
   * Re-reads the session, then tells the parent.
   *
   * The re-read is what makes `vpay:complete`'s `status` the session's own.
   * The worker flips the session in the same transaction that settles the
   * intent (lane 1), so by the time the browser has seen a terminal intent
   * the row is already written; if the re-read fails, the status already on
   * screen is sent rather than nothing.
   */
  async #announceOutcome(): Promise<void> {
    const refreshed = await this.#options.api.readSession(
      this.#options.sessionId,
      this.#options.credentials,
    );
    if (refreshed.ok) {
      this.#dispatch({
        type: "session_refreshed",
        session: contextOf(
          refreshed.value,
          this.#options.allowedMethods ?? null,
        ).session,
      });
    }
    this.#announceComplete();
  }

  /**
   * `vpay:complete`, **at most once per page**.
   *
   * Two moments can reach it: the outcome first appearing (the session
   * re-read above), and the payer pressing "Back to {merchant}" in a popup.
   * Whichever comes first wins. A second copy would fire a merchant's
   * `onComplete` twice for one payment, and a merchant that treated that
   * handler as a cue to create an order would create two.
   */
  #announceComplete(): void {
    if (this.#announced) {
      return;
    }
    const state = this.#state;
    if (!("context" in state) || state.context === null) {
      return;
    }
    if (state.name !== "outcome" && state.name !== "forwarding") {
      return;
    }
    this.#announced = true;
    this.#options.channel?.post({
      type: "vpay:complete",
      session: state.context.session.id,
      status: state.context.session.status,
    });
  }

  /** The intent's own `client_secret`, present only on the session read. */
  #intentSecret(): string | null {
    const state = this.#state;
    if (!("context" in state) || state.context === null) {
      return null;
    }
    const secret = (state.context.intent as Partial<PaymentIntent>)
      .client_secret;
    return typeof secret === "string" && secret.length > 0 ? secret : null;
  }

  #dispatch(event: CheckoutEvent): void {
    const next = reduce(this.#state, event);
    if (next === this.#state) {
      return;
    }
    this.#state = next;
    for (const listener of this.#listeners) {
      listener(next);
    }
  }
}

/** The absolute URL a `next_action.redirect_to_url` names, or `null`. */
export function redirectUrlOf(intent: PaymentIntent): string | null {
  const nextAction = intent.next_action;
  if (nextAction === null || nextAction.type !== "redirect_to_url") {
    return null;
  }
  const url = nextAction.redirect_to_url.url;
  if (typeof url !== "string" || url.length === 0) {
    return null;
  }
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  // The rail chose this string; vpay echoed it. `javascript:` here would be
  // script execution on vpay's own origin, and a relative one would resolve
  // against this page rather than the rail.
  return parsed.protocol === "http:" || parsed.protocol === "https:"
    ? url
    : null;
}
