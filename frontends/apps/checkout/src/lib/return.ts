/**
 * The return page (`/c/{cs_id}/return?t=…`), for both modes.
 *
 * A payer arriving here has come back from a rail's own page, top-level. Two
 * facts shape everything below.
 *
 * **There is no intent secret on this trip.** D6: the `return_token` in the
 * query authorises reading the session and polling its intent, and nothing
 * else. So this page cannot use `@vaam-apps/vpay-stripe-js`'s poll ladder at all — it
 * polls `GET /v1/browser/checkout/sessions/{id}/return`, which renders the
 * intent *without* its `client_secret`. That is a smaller credential doing a
 * smaller job, and it is why this file has its own reducer rather than
 * reusing `machine.ts`: a state machine with a `confirm` transition in it
 * has no business on a page that cannot confirm.
 *
 * **A fragment does not survive a redirect.** Whatever was in this page's
 * fragment before the payer left is gone; the token in the query string is
 * all there is.
 */
import type { FailureCode } from '@vaam-apps/vpay-stripe-js';

import type { MessageKey } from '../i18n/index';
import type { BrowserCheckoutApi, ReturnCredentials } from './api';
import type { FrameChannel } from './frame';
import { intentOutcome, merchantOf, type OutcomeKind } from './machine';
import type {
  CheckoutError,
  CheckoutMerchant,
  CheckoutSession,
  PublicPaymentIntent,
} from './types';

export interface ReturnContext {
  session: CheckoutSession;
  intent: PublicPaymentIntent;
  /** `null` where the read carried no usable name — see `CheckoutContext.merchant`. */
  merchant: CheckoutMerchant | null;
}

export type ReturnState =
  | { name: 'loading' }
  | { name: 'error'; error: CheckoutError }
  | { name: 'expired'; context: ReturnContext }
  | { name: 'polling'; context: ReturnContext; notice: MessageKey | null }
  | { name: 'outcome'; context: ReturnContext; kind: OutcomeKind; failure: FailureCode | null }
  | { name: 'forwarding'; context: ReturnContext; kind: OutcomeKind; url: string };

export type ReturnEvent =
  | { type: 'read'; context: ReturnContext }
  | { type: 'read_failed'; error: CheckoutError }
  | { type: 'poll_failed'; problem: MessageKey }
  | { type: 'forward'; url: string };

export const RETURN_INITIAL_STATE: ReturnState = { name: 'loading' };

/**
 * The state a read of the return view implies.
 *
 * The session's terminal statuses win, then the intent's. Anything else is
 * `polling`: the payer is back from the rail but the rail's own answer
 * reaches vpay through the worker's status query, not through the browser,
 * so "I am back" and "it is decided" are different moments and this page
 * must not conflate them.
 */
export function stateForReturn(context: ReturnContext, previousNotice: MessageKey | null = null): ReturnState {
  const { session, intent } = context;
  if (session.status === 'complete') {
    return { name: 'outcome', context, kind: 'succeeded', failure: null };
  }
  if (session.status === 'expired') {
    if (session.payment_status === 'failed') {
      const outcome = intentOutcome(intent);
      return {
        name: 'outcome',
        context,
        kind: outcome?.kind ?? 'failed',
        failure: outcome?.failure ?? null,
      };
    }
    return { name: 'expired', context };
  }
  const outcome = intentOutcome(intent);
  if (outcome !== null) {
    return { name: 'outcome', context, kind: outcome.kind, failure: outcome.failure };
  }
  return { name: 'polling', context, notice: previousNotice };
}

export function reduceReturn(state: ReturnState, event: ReturnEvent): ReturnState {
  switch (event.type) {
    case 'read':
      return state.name === 'loading' || state.name === 'polling'
        ? stateForReturn(event.context, state.name === 'polling' ? state.notice : null)
        : state;
    case 'read_failed':
      return state.name === 'loading' ? { name: 'error', error: event.error } : state;
    case 'poll_failed':
      return state.name === 'polling' ? { ...state, notice: event.problem } : state;
    case 'forward':
      return state.name === 'outcome'
        ? { name: 'forwarding', context: state.context, kind: state.kind, url: event.url }
        : state;
    default: {
      const unreachable: never = event;
      return unreachable;
    }
  }
}

export interface ReturnControllerOptions {
  sessionId: string;
  credentials: ReturnCredentials;
  api: BrowserCheckoutApi;
  navigate: (url: string) => void;
  /** Non-null only when the return page is somehow framed by an allowed origin. */
  channel: FrameChannel | null;
  /** Delay between polls, milliseconds. */
  intervalMs?: number | undefined;
  /** Total polling budget, milliseconds. */
  timeoutMs?: number | undefined;
  /** Injected clock and sleep, so the poll ladder is testable without waiting. */
  now?: (() => number) | undefined;
  sleep?: ((ms: number) => Promise<void>) | undefined;
}

const DEFAULT_RETURN_INTERVAL_MS = 2_000;
const DEFAULT_RETURN_TIMEOUT_MS = 180_000;

export class ReturnController {
  #state: ReturnState = RETURN_INITIAL_STATE;
  readonly #listeners = new Set<(state: ReturnState) => void>();
  readonly #options: ReturnControllerOptions;

  constructor(options: ReturnControllerOptions) {
    this.#options = options;
  }

  get state(): ReturnState {
    return this.#state;
  }

  subscribe(listener: (state: ReturnState) => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  /** Reads once, then polls until the session or its intent stops moving. */
  async start(): Promise<void> {
    const now = this.#options.now ?? (() => Date.now());
    const sleep = this.#options.sleep ?? defaultSleep;
    const interval = this.#options.intervalMs ?? DEFAULT_RETURN_INTERVAL_MS;
    const deadline = now() + (this.#options.timeoutMs ?? DEFAULT_RETURN_TIMEOUT_MS);

    for (;;) {
      const result = await this.#options.api.readReturn(
        this.#options.sessionId,
        this.#options.credentials,
      );
      if (result.ok) {
        const { payment_intent: intent, merchant: rawMerchant, ...session } = result.value;
        this.#dispatch({
          type: 'read',
          context: { session, intent, merchant: merchantOf(rawMerchant) },
        });
      } else if (this.#state.name === 'loading') {
        this.#dispatch({ type: 'read_failed', error: result.error });
        return;
      } else {
        // Already showing something. A failed poll leaves the payer where
        // they are, with the reason beside the spinner.
        this.#dispatch({ type: 'poll_failed', problem: result.error.code });
      }

      if (this.#state.name !== 'polling') {
        break;
      }
      const remaining = deadline - now();
      if (remaining <= 0) {
        this.#dispatch({ type: 'poll_failed', problem: 'error.network' });
        return;
      }
      await sleep(Math.min(interval, remaining));
    }

    if (this.#state.name === 'outcome') {
      this.#options.channel?.post({
        type: 'vpay:complete',
        session: this.#state.context.session.id,
        status: this.#state.context.session.status,
      });
    }
  }

  forward(url: string): void {
    this.#dispatch({ type: 'forward', url });
    if (this.#state.name === 'forwarding') {
      this.#options.navigate(url);
    }
  }

  #dispatch(event: ReturnEvent): void {
    const next = reduceReturn(this.#state, event);
    if (next === this.#state) {
      return;
    }
    this.#state = next;
    for (const listener of this.#listeners) {
      listener(next);
    }
  }
}

function defaultSleep(ms: number): Promise<void> {
  return new Promise<void>((resolve) => {
    setTimeout(resolve, ms);
  });
}
