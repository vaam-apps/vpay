/**
 * The return page: no secret, one route, and a poll that stops.
 */
import { afterEach, describe, expect, it } from 'vitest';

import { startCheckoutStub, type CheckoutStub } from '../testing/browser-stub';
import { makePublicIntent, makeSession } from '../testing/fixtures';
import { BrowserCheckoutApi } from './api';
import { ReturnController, reduceReturn, stateForReturn, type ReturnState } from './return';

let open: CheckoutStub | null = null;
afterEach(async () => {
  await open?.close();
  open = null;
});

function context(session = {}, intent = {}) {
  return {
    session: makeSession(session),
    intent: makePublicIntent(intent),
    merchant: { name: 'Boutique Test' },
  };
}

describe('stateForReturn', () => {
  it('polls while the rail has not answered', () => {
    expect(stateForReturn(context({}, { status: 'requires_action' })).name).toBe('polling');
    expect(stateForReturn(context({}, { status: 'processing' })).name).toBe('polling');
  });

  it('shows the outcome once the session is complete', () => {
    expect(
      stateForReturn(context({ status: 'complete', payment_status: 'paid' })),
    ).toMatchObject({ name: 'outcome', kind: 'succeeded' });
  });

  it('shows the failure for an expired session whose payment failed', () => {
    expect(
      stateForReturn(
        context(
          { status: 'expired', payment_status: 'failed' },
          {
            status: 'requires_payment_method',
            last_payment_error: { code: 'payer_timeout', message: 'x' },
          },
        ),
      ),
    ).toMatchObject({ name: 'outcome', kind: 'failed', failure: 'payer_timeout' });
  });

  it('shows the expiry screen for a session that simply ran out', () => {
    expect(stateForReturn(context({ status: 'expired' })).name).toBe('expired');
  });
});

describe('reduceReturn', () => {
  it('keeps the payer on the polling screen when a poll cannot be answered', () => {
    const polling = stateForReturn(context({}, { status: 'requires_action' }));
    const after = reduceReturn(polling, { type: 'poll_failed', problem: 'error.network' });
    expect(after).toMatchObject({ name: 'polling', notice: 'error.network' });
  });

  it('drops a late read after an outcome is on screen', () => {
    const outcome: ReturnState = stateForReturn(context({ status: 'complete' }));
    expect(reduceReturn(outcome, { type: 'read', context: context() })).toBe(outcome);
  });

  it('forwards only from an outcome', () => {
    const polling = stateForReturn(context({}, { status: 'processing' }));
    expect(reduceReturn(polling, { type: 'forward', url: 'https://shop.example' })).toBe(polling);
  });
});

describe('the return controller against the stub', () => {
  async function drive(options: Parameters<typeof startCheckoutStub>[0] = {}) {
    const stub = await startCheckoutStub({ paymentMethodTypes: ['orange_money'], ...options });
    open = stub;
    const navigated: string[] = [];
    // Put the intent in flight the way an Orange confirm does, without the
    // page: this controller has no credential that could confirm anything.
    await fetch(`${stub.url}/v1/browser/payment_intents/${'pi_test_stub0000000000000001'}/confirm`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        key: stub.publishableKey,
        client_secret: stub.intentSecret,
        'payment_method_data[type]': 'orange_money',
      }).toString(),
    });
    const controller = new ReturnController({
      sessionId: stub.sessionId,
      credentials: { key: stub.publishableKey, returnToken: stub.returnToken },
      api: new BrowserCheckoutApi({ baseUrl: stub.url }),
      navigate: (url) => navigated.push(url),
      channel: null,
      intervalMs: 0,
      timeoutMs: 2_000,
      sleep: () => Promise.resolve(),
    });
    return { stub, controller, navigated };
  }

  it('polls the return route until the rail settles, then shows the outcome', async () => {
    const d = await drive({ pollsBeforeTerminal: 2 });
    await d.controller.start();
    expect(d.controller.state).toMatchObject({ name: 'outcome', kind: 'succeeded' });
    const returnReads = d.stub.urls().filter((u) => u.includes('/return?'));
    expect(returnReads.length).toBeGreaterThan(1);
  });

  it('never receives the intent’s client_secret on the return route', async () => {
    const d = await drive({ pollsBeforeTerminal: 0 });
    await d.controller.start();
    const state = d.controller.state;
    expect(state.name).toBe('outcome');
    if ('context' in state) {
      expect(state.context.intent).not.toHaveProperty('client_secret');
      expect(state.context.session).not.toHaveProperty('client_secret');
    }
  });

  it('forwards with {CHECKOUT_SESSION_ID} substituted', async () => {
    const d = await drive({ pollsBeforeTerminal: 0 });
    await d.controller.start();
    d.controller.forward(`https://shop.example/ok?sid=${d.stub.sessionId}`);
    expect(d.navigated).toEqual([`https://shop.example/ok?sid=${d.stub.sessionId}`]);
  });

  it('reports a bad return token as the same message as a bad session', async () => {
    const stub = await startCheckoutStub();
    open = stub;
    const controller = new ReturnController({
      sessionId: stub.sessionId,
      credentials: { key: stub.publishableKey, returnToken: 'wrong' },
      api: new BrowserCheckoutApi({ baseUrl: stub.url }),
      navigate: () => undefined,
      channel: null,
    });
    await controller.start();
    expect(controller.state).toMatchObject({
      name: 'error',
      error: { code: 'error.session_not_found' },
    });
  });

  it('gives up on its budget with the payer still on the polling screen', async () => {
    const d = await drive({ pollsBeforeTerminal: 10_000 });
    const controller = new ReturnController({
      sessionId: d.stub.sessionId,
      credentials: { key: d.stub.publishableKey, returnToken: d.stub.returnToken },
      api: new BrowserCheckoutApi({ baseUrl: d.stub.url }),
      navigate: () => undefined,
      channel: null,
      intervalMs: 0,
      timeoutMs: 0,
      sleep: () => Promise.resolve(),
    });
    await controller.start();
    expect(controller.state).toMatchObject({ name: 'polling', notice: 'error.network' });
  });
});
