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
 * - a `requires_action` intent is re-read from the payment-intents route
 *   before the reducer sees it, because that is the only route that renders
 *   `next_action` — without it a payer who abandoned a redirect rail's page
 *   lands on a spinner nothing will ever resolve. `#withNextAction` has the
 *   mechanism.
 */
import type {
  PaymentIntentResult,
  Stripe,
  StripeError,
} from "@vaam-apps/vpay-stripe-js";

import type { MessageKey } from "../i18n/index";
import type { BrowserCheckoutApi, SessionCredentials } from "./api";
import type { FrameChannel } from "./frame";
import { normalizeCameroonMsisdn } from "./msisdn";
import {
  INITIAL_STATE,
  contextOf,
  redirectUrlOf,
  reduce,
  type CheckoutContext,
  type CheckoutEvent,
  type CheckoutState,
} from "./machine";
import type { SupportedRail } from "./rails";
import { intentClientSecretOf } from "./redirect";
import type {
  CheckoutError,
  CheckoutSessionView,
  PaymentIntent,
} from "./types";

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

  /**
   * Reads the session and enters the state it implies. Resumes a poll when
   * one is owed.
   *
   * Two reads, not one, for a `requires_action` intent — see `#withNextAction`
   * below. The session read is the only one that authorises the second, and
   * the second is the only one that carries a rail URL.
   */
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
      context: await this.#withNextAction(result.value),
    });
    if (this.#state.name === "waiting") {
      await this.#poll();
    } else if (this.#state.name === "outcome") {
      await this.#announceOutcome();
    }
  }

  /**
   * The session read as a context, with `next_action` filled in from the
   * route that renders it — for the one status where the difference decides
   * a screen.
   *
   * **`GET /v1/browser/checkout/sessions/{id}` never answers a
   * `next_action`.** Its expanded intent is
   * `PaymentIntentObject::try_from(&row)`, whose `next_action` is `None`
   * unconditionally — "`next_action` lives on the charge, not the intent",
   * `vpay_api::model`'s own comment — and neither checkout-session route
   * calls `with_next_action`, which is the only thing that attaches one. The
   * route that does is `GET /v1/browser/payment_intents/{id}`: it answers
   * through `v1::payment_intents::rendered_intent`, which reconstructs
   * `next_action` from the stored charge row for a `requires_action` intent
   * and hard-errors rather than answering `null`.
   *
   * So a payer who abandoned a redirect rail's page and came back is
   * `requires_action` with nowhere to go **on the session read's object**,
   * and a reducer given only that would put them on `waiting` — a spinner
   * for a payment nothing is going to move, because the thing that stopped
   * is the payer. One more read turns it into `resume_redirect` carrying the
   * rail's own URL. `redirect.ts`'s module doc is the same two-read ladder
   * for `/c/{id}/redirect`, and says why reading `next_action` off the
   * session "looks right, type-checks, and passes against any stub that
   * renders one — and answers `null` on every real deployment".
   *
   * This lives here and not in `machine.ts` because `stateForContext` is
   * pure: it reduces whatever intent it is handed, and filling one in is a
   * `fetch`.
   *
   * Every way the second read can fail leaves the context exactly as the
   * session answered it, so the reducer falls back to `waiting` rather than
   * to a state this page invented — and `resume_redirect` is never reached
   * with a URL that did not come from the server.
   */
  async #withNextAction(view: CheckoutSessionView): Promise<CheckoutContext> {
    const context = contextOf(view, this.#options.allowedMethods ?? null);
    if (context.intent.status !== "requires_action") {
      return context;
    }
    if (redirectUrlOf(context.intent) !== null) {
      // Already there. A server that begins rendering `next_action` on this
      // route costs the page a request rather than a behaviour.
      return context;
    }
    const intentSecret = intentClientSecretOf(view);
    if (intentSecret === null) {
      // No credential for the intent route — the session is no longer
      // `open`, so the server stopped rendering one. Nothing to resume.
      return context;
    }
    const result =
      await this.#options.stripe.retrievePaymentIntent(intentSecret);
    if (result.paymentIntent === undefined) {
      return context;
    }
    // The fresher read replaces the intent whole rather than being spliced
    // into the older one: it is the same object from a route that renders
    // strictly more, and an intent that moved on between the two reads (the
    // payer finished on the rail in another tab) should land on its outcome
    // rather than on a redirect that is already spent.
    return { ...context, intent: result.paymentIntent };
  }

  chooseRail(rail: SupportedRail): void {
    this.#dispatch({ type: "choose_rail", rail });
  }

  back(): void {
    this.#dispatch({ type: "back" });
  }

  /**
   * The push path (a push rail such as MTN's, or a payment-method type the
   * server ranked to one): normalise, confirm, then poll — or follow the
   * redirect the confirm answered with.
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
    // `confirmPayment` with `redirect: 'if_required'` rather than
    // `confirmMobileMoneyPayment`, which would follow a redirect itself: the
    // code may be a payment-method type (`mtn_wallet`), whose rail the server
    // picks again at confirm (the spec's flow is its ranking at render, which
    // is advisory, D10), so the answer can be a redirect after all. Who
    // navigates is this page's call (see `startRedirect`). The body is the
    // same `type` plus `[<type>][msisdn]` the mobile-money helper writes.
    const result = await this.#options.stripe.confirmPayment({
      clientSecret,
      confirmParams: {
        payment_method_data: { type: rail.code, [rail.code]: { msisdn } },
      },
      redirect: "if_required",
    });
    await this.#afterConfirm(result);
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
    await this.#afterConfirm(result);
  }

  /**
   * After a confirm whose answer decides the next step (a redirect rail, or a
   * wallet the server routed): follow a `next_action.redirect_to_url`, or
   * else wait on the intent like a push.
   */
  async #afterConfirm(result: PaymentIntentResult): Promise<void> {
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
      // No redirect: a push (the payer approves on the handset), or a
      // redirect rail that answered without one. Not an error — poll and let
      // the intent say what happened.
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
   * The `resume_redirect` screen's primary action: send the payer back to
   * the rail's page they abandoned.
   *
   * No confirm, no dispatch — the URL was already resolved from a stored
   * charge row by the reads that put this state on screen (`#withNextAction`
   * here, then `redirectUrlOf`/`stateForContext` in `machine.ts`), so there
   * is nothing to attempt again, only somewhere to go back to. Goes through
   * the same `#navigateTopLevel` the initial redirect uses, so the
   * framed/popup/top-level split stays decided in the one place that owns
   * it (D8).
   */
  resumeRedirect(): void {
    const state = this.#state;
    if (state.name !== "resume_redirect") {
      return;
    }
    this.#navigateTopLevel(state.url);
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

/** Re-exported from `machine.ts`, which now owns it — see that export's own doc comment. */
export { redirectUrlOf };
