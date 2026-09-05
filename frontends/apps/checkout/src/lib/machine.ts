/**
 * The page's state machine, as a pure reducer.
 *
 * Pure on purpose. Everything that can go wrong on a payment page goes
 * wrong in the transitions — a status rendered before it was read, a
 * "succeeded" screen shown for an intent that only reached `processing`, a
 * forward fired twice — and a reducer with no `fetch`, no timer and no DOM
 * in it is the only version of those transitions a test can enumerate.
 * `controller.ts` is the impure half: it turns network answers into the
 * events below and nothing else.
 *
 * One rule the shape enforces: **no state carries a status this page
 * invented.** `outcome` is reachable only from an `intent_updated` event
 * whose intent {@link intentOutcome} judged terminal, and `intentOutcome`
 * reads the same two fields `@vaam-apps/vpay-stripe-js`'s poll ladder does.
 */
import type { FailureCode, PaymentIntentStatus } from '@vaam-apps/vpay-stripe-js';

import type { MessageKey } from '../i18n/index';
import { railChoices, type RailChoices, type SupportedRail } from './rails';
import type {
  CheckoutError,
  CheckoutMerchant,
  CheckoutSession,
  PaymentIntent,
  PublicPaymentIntent,
} from './types';

/**
 * What the page knows about this payment. Read from the API, never composed
 * here — and with no credential on it: `session.client_secret` is stripped
 * by {@link contextOf}.
 */
export interface CheckoutContext {
  session: CheckoutSession;
  intent: PaymentIntent | PublicPaymentIntent;
  /**
   * `null` where the read carried no usable merchant name. The screens then
   * show a neutral heading rather than a hole in a sentence, an empty
   * string, or an identifier standing in for a name.
   */
  merchant: CheckoutMerchant | null;
}

export type OutcomeKind = 'succeeded' | 'failed' | 'canceled';

/**
 * The browser read → the context the machine carries.
 *
 * The wire renders one object: the session, with `payment_intent` expanded
 * and a `merchant` member beside it. The machine wants three named parts,
 * because `context.payment_intent.payment_method_types` reads like a
 * mistake and because the session half is what `forwardTarget` takes. One
 * conversion, here, rather than a rename argument settled differently in
 * each caller.
 */
export function contextOf(
  view: CheckoutSession & {
    payment_intent: PaymentIntent | PublicPaymentIntent;
    merchant?: CheckoutMerchant | undefined;
  },
): CheckoutContext {
  const {
    payment_intent: intent,
    merchant: rawMerchant,
    client_secret: _sessionSecret,
    ...session
  } = view;
  const merchant = merchantOf(rawMerchant);
  // The session's own `client_secret` is dropped on the way in. The
  // controller already holds it as a credential; keeping a second copy on
  // the object every screen renders from would put it inside anything that
  // ever serialises a state — a devtools snapshot, an error report, a
  // `postMessage` written in a hurry. `secrets.test.ts` and
  // `machine.test.ts` both pin its absence.
  return { session, intent, merchant };
}

/**
 * The merchant's display name as this page will use it, or `null`.
 *
 * Takes `unknown` on purpose. `isSessionEnvelope` deliberately does **not**
 * require a `merchant` member (see `api.ts`), so by the time a value gets
 * here it has only been through `JSON.parse`: it can be absent, `null`, a
 * bare string because the server renders `merchant_name`, or an object
 * whose `name` is a number. Every one of those is the same answer — this
 * page has no name to show — and none of them is a reason to refuse a
 * payment.
 *
 * A blank or whitespace-only name is also `null`: rendering `Pay ` reads
 * like finished copy with the name missing, which is worse than the neutral
 * heading.
 */
export function merchantOf(value: unknown): CheckoutMerchant | null {
  if (typeof value !== 'object' || value === null) {
    return null;
  }
  const name: unknown = (value as { name?: unknown }).name;
  return typeof name === 'string' && name.trim().length > 0 ? { name } : null;
}

/**
 * Why the page will not show a payment form.
 *
 * `embed_not_allowed` — the origin framing this page is not one the merchant
 * registered (D4). `no_supported_rail` — the intent offers only rails this
 * page has no flow for (D9).
 */
export type RefusalReason = 'embed_not_allowed' | 'no_supported_rail';

export type CheckoutState =
  | { name: 'loading' }
  | { name: 'error'; error: CheckoutError }
  | { name: 'refused'; reason: RefusalReason; context: CheckoutContext | null }
  | { name: 'expired'; context: CheckoutContext }
  | { name: 'select_rail'; context: CheckoutContext; rails: RailChoices }
  | {
      name: 'collect_msisdn';
      context: CheckoutContext;
      rails: RailChoices;
      rail: SupportedRail;
      problem: MessageKey | null;
    }
  | {
      name: 'ready_redirect';
      context: CheckoutContext;
      rails: RailChoices;
      rail: SupportedRail;
      problem: MessageKey | null;
    }
  | { name: 'confirming'; context: CheckoutContext; rail: SupportedRail }
  | {
      name: 'waiting';
      context: CheckoutContext;
      rail: SupportedRail | null;
      /**
       * A poll that could not be answered. The payer stays on the waiting
       * screen — the payment is in flight and saying otherwise would be a
       * claim this page cannot support — with the reason shown beside it.
       */
      notice: MessageKey | null;
    }
  | { name: 'redirecting'; context: CheckoutContext; rail: SupportedRail; url: string }
  | {
      name: 'outcome';
      context: CheckoutContext;
      kind: OutcomeKind;
      failure: FailureCode | null;
    }
  | { name: 'forwarding'; context: CheckoutContext; kind: OutcomeKind; url: string };

export type CheckoutEvent =
  | { type: 'loaded'; context: CheckoutContext }
  | { type: 'load_failed'; error: CheckoutError }
  | { type: 'refuse'; reason: RefusalReason }
  | { type: 'choose_rail'; rail: SupportedRail }
  | { type: 'back' }
  | { type: 'problem'; problem: MessageKey }
  | { type: 'confirm_started' }
  | { type: 'intent_updated'; intent: PaymentIntent | PublicPaymentIntent }
  | { type: 'redirect_required'; url: string }
  | { type: 'session_refreshed'; session: CheckoutSession }
  | { type: 'forward'; url: string };

export const INITIAL_STATE: CheckoutState = { name: 'loading' };

/**
 * Whether an intent has stopped moving, and how it ended.
 *
 * `null` means "still in flight" — including `requires_payment_method` with
 * no `last_payment_error`, which is *also* the status of an intent nobody
 * has confirmed. Treating that as an outcome would render "payment not
 * completed" on a page the payer just opened. Same reading as
 * `@vaam-apps/vpay-stripe-js`'s `hasStoppedMoving`, deliberately.
 */
export function intentOutcome(
  intent: PaymentIntent | PublicPaymentIntent,
): { kind: OutcomeKind; failure: FailureCode | null } | null {
  const status: PaymentIntentStatus = intent.status;
  if (status === 'succeeded') {
    return { kind: 'succeeded', failure: null };
  }
  if (status === 'canceled') {
    return { kind: 'canceled', failure: null };
  }
  if (status === 'requires_payment_method' && intent.last_payment_error !== null) {
    return { kind: 'failed', failure: intent.last_payment_error.code };
  }
  return null;
}

/**
 * The state a freshly-read session lands in.
 *
 * The session's own `status` wins where it is terminal, because the worker
 * writes it in the settlement transaction (lane 1) and it is the thing the
 * merchant's `success_url` correlates against. Where the session is still
 * `open` the intent decides, which is what makes a reload during a push land
 * back on "check your phone" rather than on an empty form.
 */
export function stateForContext(context: CheckoutContext): CheckoutState {
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

  const rails = railChoices(intent);

  if (intent.status === 'processing' || intent.status === 'requires_action') {
    // Already confirmed — a reload, or a payer coming back to the tab. The
    // rail that was chosen is not recoverable from the intent (a confirmed
    // intent does not name it), so the waiting screen shows without one.
    return { name: 'waiting', context, rail: null, notice: null };
  }

  if (rails.supported.length === 0) {
    return { name: 'refused', reason: 'no_supported_rail', context };
  }
  if (rails.supported.length === 1) {
    const only = rails.supported[0] as SupportedRail;
    return entryStateFor(context, rails, only);
  }
  return { name: 'select_rail', context, rails };
}

/** The first screen for a chosen rail: a form for a push, a button for a redirect. */
function entryStateFor(
  context: CheckoutContext,
  rails: RailChoices,
  rail: SupportedRail,
): CheckoutState {
  return rail.flow === 'mobile_money_push'
    ? { name: 'collect_msisdn', context, rails, rail, problem: null }
    : { name: 'ready_redirect', context, rails, rail, problem: null };
}

/** The state a `back` from a rail's entry screen returns to. */
function backStateFor(context: CheckoutContext, rails: RailChoices): CheckoutState {
  return rails.supported.length > 1
    ? { name: 'select_rail', context, rails }
    : stateForContext(context);
}

/**
 * The whole transition table.
 *
 * An event that does not apply to the current state returns the state
 * unchanged rather than throwing. A payment page that crashed because a late
 * poll answered after the payer pressed Continue would be a worse bug than a
 * dropped event, and `machine.test.ts` asserts the drops that matter (a
 * `forward` from `waiting` does nothing; a second `intent_updated` after
 * `forwarding` does nothing).
 */
export function reduce(state: CheckoutState, event: CheckoutEvent): CheckoutState {
  switch (event.type) {
    case 'loaded':
      return state.name === 'loading' ? stateForContext(event.context) : state;

    case 'load_failed':
      return state.name === 'loading' ? { name: 'error', error: event.error } : state;

    case 'refuse':
      // Reachable from any state: the embed check runs before the session
      // read and can also be re-run when the parent changes.
      return {
        name: 'refused',
        reason: event.reason,
        context: 'context' in state ? state.context : null,
      };

    case 'choose_rail':
      return state.name === 'select_rail'
        ? entryStateFor(state.context, state.rails, event.rail)
        : state;

    case 'back':
      return state.name === 'collect_msisdn' || state.name === 'ready_redirect'
        ? backStateFor(state.context, state.rails)
        : state;

    case 'problem':
      if (state.name === 'collect_msisdn' || state.name === 'ready_redirect') {
        return { ...state, problem: event.problem };
      }
      if (state.name === 'waiting') {
        return { ...state, notice: event.problem };
      }
      if (state.name === 'confirming') {
        // A confirm that never reached the rail returns the payer to the
        // screen they submitted from, with the reason shown there.
        return {
          ...entryStateFor(state.context, railChoices(state.context.intent), state.rail),
          problem: event.problem,
        } as CheckoutState;
      }
      return state;

    case 'confirm_started':
      return state.name === 'collect_msisdn' || state.name === 'ready_redirect'
        ? { name: 'confirming', context: state.context, rail: state.rail }
        : state;

    case 'intent_updated': {
      if (state.name !== 'confirming' && state.name !== 'waiting') {
        return state;
      }
      const context: CheckoutContext = { ...state.context, intent: event.intent };
      const outcome = intentOutcome(event.intent);
      if (outcome !== null) {
        return { name: 'outcome', context, kind: outcome.kind, failure: outcome.failure };
      }
      return { name: 'waiting', context, rail: state.rail, notice: null };
    }

    case 'redirect_required':
      return state.name === 'confirming'
        ? { name: 'redirecting', context: state.context, rail: state.rail, url: event.url }
        : state;

    case 'session_refreshed':
      // Only where a fresher session changes nothing about what is on
      // screen. Re-deriving the state here would let a late read move a
      // payer off an outcome they are reading.
      return state.name === 'outcome'
        ? { ...state, context: { ...state.context, session: event.session } }
        : state;

    case 'forward':
      return state.name === 'outcome'
        ? { name: 'forwarding', context: state.context, kind: state.kind, url: event.url }
        : state;

    default: {
      // Exhaustive: adding an event without a case is a compile error.
      const unreachable: never = event;
      return unreachable;
    }
  }
}
