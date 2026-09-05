/**
 * The transition table, enumerated.
 *
 * Every assertion here is about a state name or a field the reducer wrote —
 * never about a status this file supplied and then read back. The intents
 * are the wire contract's shapes; the outcomes are derived from them by the
 * same two-field rule `@vaam-apps/vpay-stripe-js` polls on.
 */
import { describe, expect, it } from 'vitest';

import { makeContext, makeIntent } from '../testing/fixtures';
import {
  INITIAL_STATE,
  contextOf,
  intentOutcome,
  merchantOf,
  reduce,
  stateForContext,
  type CheckoutState,
} from './machine';
import { railChoices } from './rails';

const MTN = { code: 'mtn_momo', flow: 'mobile_money_push', label: 'rail.mtn_momo' } as const;
const ORANGE = { code: 'orange_money', flow: 'redirect', label: 'rail.orange_money' } as const;

describe('intentOutcome', () => {
  it('is null for an intent nobody has confirmed', () => {
    expect(intentOutcome(makeIntent({ status: 'requires_payment_method' }))).toBeNull();
  });

  it('is null while the intent is still in flight', () => {
    expect(intentOutcome(makeIntent({ status: 'processing' }))).toBeNull();
    expect(intentOutcome(makeIntent({ status: 'requires_action' }))).toBeNull();
  });

  it('reads a failure off requires_payment_method + last_payment_error', () => {
    expect(
      intentOutcome(
        makeIntent({
          status: 'requires_payment_method',
          last_payment_error: { code: 'insufficient_funds', message: 'no' },
        }),
      ),
    ).toEqual({ kind: 'failed', failure: 'insufficient_funds' });
  });

  it('reads succeeded and canceled', () => {
    expect(intentOutcome(makeIntent({ status: 'succeeded' }))).toEqual({
      kind: 'succeeded',
      failure: null,
    });
    expect(intentOutcome(makeIntent({ status: 'canceled' }))).toEqual({
      kind: 'canceled',
      failure: null,
    });
  });
});

describe('the state a freshly-read session lands in', () => {
  it('goes straight to the MSISDN form when the intent offers one push rail', () => {
    const state = stateForContext(makeContext());
    expect(state.name).toBe('collect_msisdn');
  });

  it('shows the selector when the intent offers more than one rail this page can drive', () => {
    const state = stateForContext(
      makeContext({}, { payment_method_types: ['mtn_momo', 'orange_money'] }),
    );
    expect(state.name).toBe('select_rail');
  });

  it('goes straight to the redirect prompt for a single redirect rail', () => {
    const state = stateForContext(makeContext({}, { payment_method_types: ['orange_money'] }));
    expect(state.name).toBe('ready_redirect');
  });

  it('refuses when the intent offers only rails this page has no flow for (D9)', () => {
    const state = stateForContext(makeContext({}, { payment_method_types: ['zzz_pay'] }));
    expect(state).toMatchObject({ name: 'refused', reason: 'no_supported_rail' });
  });

  it('lists an unknown rail as unsupported beside the ones it can drive', () => {
    const state = stateForContext(
      makeContext({}, { payment_method_types: ['mtn_momo', 'zzz_pay'] }),
    );
    expect(state.name).toBe('collect_msisdn');
    expect(state).toMatchObject({ rails: { unsupported: ['zzz_pay'] } });
  });

  it('returns to the waiting screen for an intent that is already in flight', () => {
    expect(stateForContext(makeContext({}, { status: 'processing' })).name).toBe('waiting');
    expect(stateForContext(makeContext({}, { status: 'requires_action' })).name).toBe('waiting');
  });

  it('shows the outcome when the session is already complete', () => {
    const state = stateForContext(makeContext({ status: 'complete', payment_status: 'paid' }));
    expect(state).toMatchObject({ name: 'outcome', kind: 'succeeded' });
  });

  it('shows the expiry screen for an expired, unpaid session', () => {
    expect(stateForContext(makeContext({ status: 'expired' })).name).toBe('expired');
  });

  it('shows the failure for an expired session whose payment failed (D10)', () => {
    const state = stateForContext(
      makeContext(
        { status: 'expired', payment_status: 'failed' },
        {
          status: 'requires_payment_method',
          last_payment_error: { code: 'payer_timeout', message: 'x' },
        },
      ),
    );
    expect(state).toMatchObject({ name: 'outcome', kind: 'failed', failure: 'payer_timeout' });
  });

  it('trusts the intent over an open session, so a settled intent is never shown as a form', () => {
    const state = stateForContext(makeContext({ status: 'open' }, { status: 'succeeded' }));
    expect(state).toMatchObject({ name: 'outcome', kind: 'succeeded' });
  });
});

describe('the MTN path', () => {
  it('walks open → confirming → waiting → outcome → forwarding', () => {
    let state: CheckoutState = INITIAL_STATE;
    state = reduce(state, { type: 'loaded', context: makeContext() });
    expect(state.name).toBe('collect_msisdn');

    state = reduce(state, { type: 'confirm_started' });
    expect(state).toMatchObject({ name: 'confirming', rail: { code: 'mtn_momo' } });

    state = reduce(state, { type: 'intent_updated', intent: makeIntent({ status: 'processing' }) });
    expect(state).toMatchObject({ name: 'waiting', notice: null });

    state = reduce(state, { type: 'intent_updated', intent: makeIntent({ status: 'succeeded' }) });
    expect(state).toMatchObject({ name: 'outcome', kind: 'succeeded' });

    state = reduce(state, { type: 'forward', url: 'https://shop.example/ok' });
    expect(state).toMatchObject({ name: 'forwarding', url: 'https://shop.example/ok' });
  });

  it('returns a rejected confirm to the form with the reason on it', () => {
    let state: CheckoutState = reduce(INITIAL_STATE, { type: 'loaded', context: makeContext() });
    state = reduce(state, { type: 'confirm_started' });
    state = reduce(state, { type: 'problem', problem: 'error.network' });
    expect(state).toMatchObject({ name: 'collect_msisdn', problem: 'error.network' });
  });

  it('shows an invalid number on the form without leaving it', () => {
    let state: CheckoutState = reduce(INITIAL_STATE, { type: 'loaded', context: makeContext() });
    state = reduce(state, { type: 'problem', problem: 'msisdn.invalid' });
    expect(state).toMatchObject({ name: 'collect_msisdn', problem: 'msisdn.invalid' });
  });

  it('keeps the payer on the waiting screen when a poll cannot be answered', () => {
    let state: CheckoutState = reduce(INITIAL_STATE, { type: 'loaded', context: makeContext() });
    state = reduce(state, { type: 'confirm_started' });
    state = reduce(state, { type: 'intent_updated', intent: makeIntent({ status: 'processing' }) });
    state = reduce(state, { type: 'problem', problem: 'error.network' });
    expect(state).toMatchObject({ name: 'waiting', notice: 'error.network' });
  });

  it('reaches a failure outcome from a rail decline', () => {
    let state: CheckoutState = reduce(INITIAL_STATE, { type: 'loaded', context: makeContext() });
    state = reduce(state, { type: 'confirm_started' });
    state = reduce(state, {
      type: 'intent_updated',
      intent: makeIntent({
        status: 'requires_payment_method',
        last_payment_error: { code: 'insufficient_funds', message: 'x' },
      }),
    });
    expect(state).toMatchObject({ name: 'outcome', kind: 'failed', failure: 'insufficient_funds' });
  });
});

describe('the Orange path', () => {
  const orangeContext = makeContext({}, { payment_method_types: ['orange_money'] });

  it('walks open → confirming → redirecting', () => {
    let state: CheckoutState = reduce(INITIAL_STATE, {
      type: 'loaded',
      context: orangeContext,
    });
    expect(state).toMatchObject({ name: 'ready_redirect', rail: { code: 'orange_money' } });
    state = reduce(state, { type: 'confirm_started' });
    state = reduce(state, { type: 'redirect_required', url: 'https://rail.example/pay' });
    expect(state).toMatchObject({ name: 'redirecting', url: 'https://rail.example/pay' });
  });

  it('does not accept a redirect from a state that never confirmed', () => {
    const state = reduce(reduce(INITIAL_STATE, { type: 'loaded', context: orangeContext }), {
      type: 'redirect_required',
      url: 'https://rail.example/pay',
    });
    expect(state.name).toBe('ready_redirect');
  });
});

describe('the selector', () => {
  const both = makeContext({}, { payment_method_types: ['mtn_momo', 'orange_money'] });

  it('chooses a rail and can go back', () => {
    let state: CheckoutState = reduce(INITIAL_STATE, { type: 'loaded', context: both });
    state = reduce(state, { type: 'choose_rail', rail: ORANGE });
    expect(state.name).toBe('ready_redirect');
    state = reduce(state, { type: 'back' });
    expect(state.name).toBe('select_rail');
    state = reduce(state, { type: 'choose_rail', rail: MTN });
    expect(state.name).toBe('collect_msisdn');
  });

  it('cannot be reached by `back` when there was only ever one rail', () => {
    const single = reduce(INITIAL_STATE, { type: 'loaded', context: makeContext() });
    expect(reduce(single, { type: 'back' }).name).toBe('collect_msisdn');
  });
});

describe('events that do not apply', () => {
  it('drops a forward while still waiting', () => {
    let state: CheckoutState = reduce(INITIAL_STATE, { type: 'loaded', context: makeContext() });
    state = reduce(state, { type: 'confirm_started' });
    state = reduce(state, { type: 'intent_updated', intent: makeIntent({ status: 'processing' }) });
    expect(reduce(state, { type: 'forward', url: 'https://shop.example/ok' })).toBe(state);
  });

  it('drops a late poll that lands after the payer pressed Continue', () => {
    let state: CheckoutState = reduce(INITIAL_STATE, { type: 'loaded', context: makeContext() });
    state = reduce(state, { type: 'confirm_started' });
    state = reduce(state, { type: 'intent_updated', intent: makeIntent({ status: 'succeeded' }) });
    state = reduce(state, { type: 'forward', url: 'https://shop.example/ok' });
    const after = reduce(state, {
      type: 'intent_updated',
      intent: makeIntent({ status: 'canceled' }),
    });
    expect(after).toBe(state);
  });

  it('drops a second load', () => {
    const first = reduce(INITIAL_STATE, { type: 'loaded', context: makeContext() });
    expect(reduce(first, { type: 'loaded', context: makeContext() })).toBe(first);
  });

  it('refuses from anywhere, keeping whatever context was already read', () => {
    const loaded = reduce(INITIAL_STATE, { type: 'loaded', context: makeContext() });
    const refused = reduce(loaded, { type: 'refuse', reason: 'embed_not_allowed' });
    expect(refused).toMatchObject({ name: 'refused', reason: 'embed_not_allowed' });
  });

  it('reports a read that failed', () => {
    const state = reduce(INITIAL_STATE, {
      type: 'load_failed',
      error: { code: 'error.session_not_found' },
    });
    expect(state).toMatchObject({ name: 'error', error: { code: 'error.session_not_found' } });
  });
});

describe('contextOf', () => {
  it('splits the expanded session into session, intent and merchant', () => {
    const session = makeContext().session;
    const intent = makeIntent();
    const view = { ...session, payment_intent: intent, merchant: { name: 'Boutique' } };
    expect(contextOf(view)).toEqual({ session, intent, merchant: { name: 'Boutique' } });
  });

  it('drops the session’s client_secret, so no rendered state carries one', () => {
    const view = {
      ...makeContext().session,
      client_secret: 'cs_test_fixture000000000001_secret_zzzz',
      payment_intent: makeIntent(),
      merchant: { name: 'Boutique' },
    };
    const context = contextOf(view);
    expect(context.session).not.toHaveProperty('client_secret');
    expect(JSON.stringify(context.session)).not.toContain('_secret_');
  });

  it('leaves the expanded intent out of the session half, so nothing reads it twice', () => {
    const view = {
      ...makeContext().session,
      payment_intent: makeIntent(),
      merchant: { name: 'Boutique' },
    };
    expect('payment_intent' in contextOf(view).session).toBe(false);
    expect('merchant' in contextOf(view).session).toBe(false);
  });
});

describe('merchantOf', () => {
  it('reads the documented shape', () => {
    expect(merchantOf({ name: 'Boutique Test' })).toEqual({ name: 'Boutique Test' });
  });

  it('is null for every shape that is not one — none of which is a reason to refuse a payment', () => {
    for (const value of [
      undefined,
      null,
      'Boutique Test', // a server rendering `merchant_name` as a bare string
      42,
      {},
      { name: null },
      { name: 42 },
      { name: '' },
      { name: '   ' }, // renders `Pay ` — a sentence with the name missing
      { display_name: 'Boutique Test' },
      [],
    ]) {
      expect(merchantOf(value), JSON.stringify(value ?? String(value))).toBeNull();
    }
  });
});

describe('contextOf and a missing merchant', () => {
  it('carries null rather than throwing when the read had no merchant member', () => {
    const view = { ...makeContext().session, payment_intent: makeIntent() };
    expect(contextOf(view).merchant).toBeNull();
    // Everything the page actually needs is still there.
    expect(contextOf(view).intent.amount).toBe(5000);
  });
});

describe('railChoices', () => {
  it('takes the offered rails from the intent, never from a list written here', () => {
    expect(railChoices(makeIntent({ payment_method_types: [] })).supported).toEqual([]);
    expect(
      railChoices(makeIntent({ payment_method_types: ['orange_money'] })).supported.map(
        (r) => r.code,
      ),
    ).toEqual(['orange_money']);
  });

  it('preserves the intent’s order', () => {
    expect(
      railChoices(makeIntent({ payment_method_types: ['orange_money', 'mtn_momo'] })).supported.map(
        (r) => r.code,
      ),
    ).toEqual(['orange_money', 'mtn_momo']);
  });
});
