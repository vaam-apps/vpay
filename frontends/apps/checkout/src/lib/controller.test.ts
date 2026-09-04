/**
 * The whole page flow, driven against a real HTTP stub of the five routes
 * it speaks to.
 *
 * Not a mocked `fetch`: `@vpay/stripe-js` builds the confirm body and the
 * poll URL itself, and the point of these tests is that vpay's browser
 * surface, that package and this page agree on the wire — which a stubbed
 * function they all share cannot show.
 */
import { loadStripe } from '@vpay/stripe-js';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { startCheckoutStub, type CheckoutStub } from '../testing/browser-stub';
import { BrowserCheckoutApi } from './api';
import { CheckoutController } from './controller';
import type { ChildMessage, FrameChannel } from './frame';
import type { SupportedRail } from './rails';

const MTN: SupportedRail = { code: 'mtn_momo', flow: 'mobile_money_push', label: 'rail.mtn_momo' };
const ORANGE: SupportedRail = {
  code: 'orange_money',
  flow: 'redirect',
  label: 'rail.orange_money',
};
const VALID_MSISDN = '237600000400';

let open: CheckoutStub | null = null;

afterEach(async () => {
  await open?.close();
  open = null;
});

interface Harness {
  stub: CheckoutStub;
  controller: CheckoutController;
  navigated: string[];
  posted: ChildMessage[];
  states: string[];
}

async function harness(
  options: Parameters<typeof startCheckoutStub>[0] = {},
  framed = false,
): Promise<Harness> {
  const stub = await startCheckoutStub(options);
  open = stub;
  const navigated: string[] = [];
  const posted: ChildMessage[] = [];
  const channel: FrameChannel = {
    parentOrigin: 'https://shop.example',
    post: (message) => posted.push(message),
    postHeight: () => undefined,
    dispose: () => undefined,
  };
  const stripe = await loadStripe(stub.publishableKey, { baseUrl: stub.url });
  const controller = new CheckoutController({
    sessionId: stub.sessionId,
    credentials: { key: stub.publishableKey, clientSecret: stub.sessionSecret },
    api: new BrowserCheckoutApi({ baseUrl: stub.url }),
    stripe,
    navigate: (url) => navigated.push(url),
    channel: framed ? channel : null,
    pollIntervalMs: 1,
    pollTimeoutMs: 4_000,
  });
  const states: string[] = [];
  controller.subscribe((state) => states.push(state.name));
  return { stub, controller, navigated, posted, states };
}

describe('the MTN push, end to end', () => {
  it('walks open → confirming → waiting → outcome and forwards with the id substituted', async () => {
    const h = await harness({ pollsBeforeTerminal: 1 });
    await h.controller.start();
    expect(h.controller.state.name).toBe('collect_msisdn');

    await h.controller.submitMsisdn(VALID_MSISDN);

    expect(h.states).toEqual([
      'collect_msisdn',
      'confirming',
      'waiting',
      'outcome',
      // `session_refreshed` writes into the outcome state without renaming it.
      'outcome',
    ]);
    expect(h.controller.state).toMatchObject({ name: 'outcome', kind: 'succeeded' });

    h.controller.forward('https://shop.example/ok?sid=' + h.stub.sessionId);
    expect(h.navigated).toEqual([`https://shop.example/ok?sid=${h.stub.sessionId}`]);
  });

  it('sends the canonical MSISDN to the rail, not whatever the payer typed', async () => {
    const h = await harness({ pollsBeforeTerminal: 0 });
    await h.controller.start();
    await h.controller.submitMsisdn(' +237 600 000 400 ');
    const confirm = h.stub.requests.find((r) => r.method === 'POST');
    // `@vpay/stripe-js`'s own form encoding, asserted verbatim: brackets
    // are legal in a query component and it does not escape them.
    expect(confirm?.body).toContain('payment_method_data[mtn_momo][msisdn]=237600000400');
    expect(confirm?.body).not.toContain('+237');
  });

  it('refuses an invalid number without touching the rail', async () => {
    const h = await harness();
    await h.controller.start();
    const before = h.stub.requests.length;
    await h.controller.submitMsisdn('237600000ce0');
    expect(h.controller.state).toMatchObject({
      name: 'collect_msisdn',
      problem: 'msisdn.invalid',
    });
    expect(h.stub.requests.length).toBe(before);
  });

  it('reaches a failure outcome the stub decided, not one this test wrote into the page', async () => {
    const h = await harness({
      pollsBeforeTerminal: 0,
      terminal: { kind: 'failed', failure: 'insufficient_funds', message: 'no funds' },
    });
    await h.controller.start();
    await h.controller.submitMsisdn(VALID_MSISDN);
    expect(h.controller.state).toMatchObject({
      name: 'outcome',
      kind: 'failed',
      failure: 'insufficient_funds',
    });
  });

  it('reaches a canceled outcome', async () => {
    const h = await harness({ pollsBeforeTerminal: 0, terminal: { kind: 'canceled' } });
    await h.controller.start();
    await h.controller.submitMsisdn(VALID_MSISDN);
    expect(h.controller.state).toMatchObject({ name: 'outcome', kind: 'canceled' });
  });

  it('drops a second submit while the first is confirming', async () => {
    const h = await harness({ pollsBeforeTerminal: 0 });
    await h.controller.start();
    const first = h.controller.submitMsisdn(VALID_MSISDN);
    const second = h.controller.submitMsisdn(VALID_MSISDN);
    await Promise.all([first, second]);
    expect(h.stub.requests.filter((r) => r.method === 'POST')).toHaveLength(1);
  });
});

describe('the Orange redirect', () => {
  it('confirms, records the redirect, and navigates top-level when not framed', async () => {
    const h = await harness({
      paymentMethodTypes: ['orange_money'],
      redirectUrl: 'https://rail.example/stub-hosted-page/tok_abc',
    });
    await h.controller.start();
    expect(h.controller.state.name).toBe('ready_redirect');

    await h.controller.startRedirect();
    const confirm = h.stub.requests.find((r) => r.method === 'POST');
    expect(confirm?.body).toContain('payment_method_data[type]=orange_money');
    expect(h.controller.state).toMatchObject({
      name: 'redirecting',
      url: 'https://rail.example/stub-hosted-page/tok_abc',
    });
    expect(h.navigated).toEqual(['https://rail.example/stub-hosted-page/tok_abc']);
  });

  it('asks the parent to navigate when framed, and never navigates itself', async () => {
    const h = await harness({ paymentMethodTypes: ['orange_money'] }, true);
    await h.controller.start();
    await h.controller.startRedirect();
    expect(h.navigated).toEqual([]);
    expect(h.posted).toEqual([
      { type: 'vpay:redirect', url: 'https://rail.example/stub-hosted-page/tok_123' },
    ]);
    // The parent sandboxes the frame without `allow-top-navigation`, so a
    // page that tried to navigate itself would silently do nothing.
    expect(h.controller.state.name).toBe('redirecting');
  });

  it('does not confirm twice when the payer double-presses', async () => {
    const h = await harness({ paymentMethodTypes: ['orange_money'] });
    await h.controller.start();
    await Promise.all([h.controller.startRedirect(), h.controller.startRedirect()]);
    expect(h.stub.requests.filter((r) => r.method === 'POST')).toHaveLength(1);
  });
});

describe('the confirm’s return_url, as the server rules on it', () => {
  /*
   * The ruling (integrator, Step 9 lane 3b): a confirm on an intent that
   * belongs to an **open checkout session** does not need a `return_url` —
   * the server substitutes the session's own return page, which is the only
   * URL that carries the `t=` token this page's return trip needs. So the
   * page sends none, and must not invent one. The stub applies the rule in
   * both directions, so a regression on either side is visible here.
   */
  it('sends no return_url — the session’s return page is the server’s to choose', async () => {
    const h = await harness({ paymentMethodTypes: ['orange_money'] });
    await h.controller.start();
    await h.controller.startRedirect();
    const confirm = h.stub.requests.find((r) => r.method === 'POST');
    expect(confirm?.body).toContain('payment_method_data[type]=orange_money');
    expect(confirm?.body).not.toContain('return_url');
    // Accepted: the stub's intent belongs to a session that is still open.
    expect(h.controller.state.name).toBe('redirecting');
  });

  it('is refused 400 invalid_param by the stub when the intent has no open session', async () => {
    const stub = await startCheckoutStub({
      standaloneIntent: true,
      paymentMethodTypes: ['orange_money'],
    });
    open = stub;
    const stripe = await loadStripe(stub.publishableKey, { baseUrl: stub.url });
    const result = await stripe.confirmPayment({
      clientSecret: stub.intentSecret,
      confirmParams: { payment_method_data: { type: 'orange_money' } },
      redirect: 'if_required',
    });
    expect(result.paymentIntent).toBeUndefined();
    expect(result.error).toMatchObject({
      type: 'invalid_request_error',
      code: 'invalid_request',
      param: 'return_url',
    });
  });

  it('accepts the same session-less confirm once a return_url is named', async () => {
    const stub = await startCheckoutStub({
      standaloneIntent: true,
      paymentMethodTypes: ['orange_money'],
    });
    open = stub;
    const stripe = await loadStripe(stub.publishableKey, { baseUrl: stub.url });
    const result = await stripe.confirmPayment({
      clientSecret: stub.intentSecret,
      confirmParams: {
        payment_method_data: { type: 'orange_money' },
        return_url: 'https://shop.example/after-orange',
      },
      redirect: 'if_required',
    });
    expect(result.error).toBeUndefined();
    expect(result.paymentIntent).toMatchObject({
      status: 'requires_action',
      next_action: {
        type: 'redirect_to_url',
        redirect_to_url: { return_url: 'https://shop.example/after-orange' },
      },
    });
  });

  it('does not apply the rule to a push rail, which redirects nowhere', async () => {
    const stub = await startCheckoutStub({ standaloneIntent: true });
    open = stub;
    const stripe = await loadStripe(stub.publishableKey, { baseUrl: stub.url });
    const result = await stripe.confirmMobileMoneyPayment(stub.intentSecret, {
      type: 'mtn_momo',
      msisdn: VALID_MSISDN,
    });
    expect(result.error).toBeUndefined();
    expect(result.paymentIntent).toMatchObject({ status: 'processing' });
  });
});

describe('the merchant name is a nicety, not a precondition', () => {
  it('reads the name when the server sends `merchant: { name }` — lane 1b’s shape', async () => {
    const h = await harness({ merchant: { kind: 'named', name: 'Boutique Test' } });
    await h.controller.start();
    expect(h.controller.state).toMatchObject({
      name: 'collect_msisdn',
      context: { merchant: { name: 'Boutique Test' } },
    });
  });

  it('pays a session whose read carried no `merchant` member at all', async () => {
    const h = await harness({ merchant: { kind: 'absent' }, pollsBeforeTerminal: 0 });
    await h.controller.start();
    // The whole point: not `error.unexpected`. A missing display name is not
    // a reason to refuse a payment.
    expect(h.controller.state).toMatchObject({
      name: 'collect_msisdn',
      context: { merchant: null },
    });
    await h.controller.submitMsisdn(VALID_MSISDN);
    expect(h.controller.state).toMatchObject({ name: 'outcome', kind: 'succeeded' });
  });

  it('pays a session whose `merchant` is not the documented shape either', async () => {
    // What a server that renamed the member to `merchant_name` looks like
    // from here: `merchant` present, but a bare string.
    const h = await harness({ merchant: { kind: 'malformed', value: 'Boutique Test' } });
    await h.controller.start();
    expect(h.controller.state).toMatchObject({
      name: 'collect_msisdn',
      context: { merchant: null },
    });
  });
});

describe('the rail selector', () => {
  it('offers both rails when the intent does, and drives whichever is chosen', async () => {
    const h = await harness({ paymentMethodTypes: ['mtn_momo', 'orange_money'] });
    await h.controller.start();
    expect(h.controller.state.name).toBe('select_rail');
    h.controller.chooseRail(ORANGE);
    expect(h.controller.state.name).toBe('ready_redirect');
    h.controller.back();
    h.controller.chooseRail(MTN);
    expect(h.controller.state.name).toBe('collect_msisdn');
  });
});

describe('sessions that cannot be paid', () => {
  it('reports the uniform 404 as one message, whichever half of the link was wrong', async () => {
    const stub = await startCheckoutStub();
    open = stub;
    const stripe = await loadStripe('pk_test_wrongwrongwrongwrong', { baseUrl: stub.url });
    const controller = new CheckoutController({
      sessionId: stub.sessionId,
      credentials: { key: 'pk_test_wrongwrongwrongwrong', clientSecret: stub.sessionSecret },
      api: new BrowserCheckoutApi({ baseUrl: stub.url }),
      stripe,
      navigate: () => undefined,
      channel: null,
    });
    await controller.start();
    expect(controller.state).toMatchObject({
      name: 'error',
      error: { code: 'error.session_not_found', serverCode: 'resource_missing' },
    });
  });

  it('shows the expiry screen for an expired session', async () => {
    const h = await harness({ pollsBeforeTerminal: 0, terminal: { kind: 'canceled' } });
    // Drive the stub's session to `expired` first, then read it fresh.
    await h.controller.start();
    await h.controller.submitMsisdn(VALID_MSISDN);
    const second = new CheckoutController({
      sessionId: h.stub.sessionId,
      credentials: { key: h.stub.publishableKey, clientSecret: h.stub.sessionSecret },
      api: new BrowserCheckoutApi({ baseUrl: h.stub.url }),
      stripe: await loadStripe(h.stub.publishableKey, { baseUrl: h.stub.url }),
      navigate: () => undefined,
      channel: null,
    });
    await second.start();
    // `expired` + `payment_status: failed` is a failure, not a blank expiry
    // screen — D10 says so and the stub renders it that way.
    expect(second.state).toMatchObject({ name: 'outcome', kind: 'canceled' });
  });

  it('reports a network failure as such, without a thrown value reaching the page', async () => {
    const stub = await startCheckoutStub();
    const url = stub.url;
    await stub.close();
    const controller = new CheckoutController({
      sessionId: 'cs_test_stub0000000000000001',
      credentials: { key: 'pk_test_0123456789abcdefghij', clientSecret: 'cs_x_secret_y' },
      api: new BrowserCheckoutApi({ baseUrl: url }),
      stripe: await loadStripe('pk_test_0123456789abcdefghij', { baseUrl: url }),
      navigate: () => undefined,
      channel: null,
    });
    await controller.start();
    expect(controller.state).toMatchObject({ name: 'error', error: { code: 'error.network' } });
  });
});

describe('the embedded protocol', () => {
  it('posts vpay:complete with the session id and the session’s own refreshed status', async () => {
    const h = await harness({ pollsBeforeTerminal: 0, uiMode: 'embedded' }, true);
    await h.controller.start();
    await h.controller.submitMsisdn(VALID_MSISDN);
    expect(h.posted).toEqual([
      { type: 'vpay:complete', session: h.stub.sessionId, status: 'complete' },
    ]);
  });

  it('never carries a secret in a vpay:complete message', async () => {
    const h = await harness({ pollsBeforeTerminal: 0, uiMode: 'embedded' }, true);
    await h.controller.start();
    await h.controller.submitMsisdn(VALID_MSISDN);
    const serialised = JSON.stringify(h.posted);
    expect(serialised).not.toContain(h.stub.sessionSecret);
    expect(serialised).not.toContain(h.stub.intentSecret);
  });

  it('asks the parent to perform the forward rather than navigating the frame', async () => {
    const h = await harness({ pollsBeforeTerminal: 0, uiMode: 'embedded' }, true);
    await h.controller.start();
    await h.controller.submitMsisdn(VALID_MSISDN);
    h.controller.forward('https://shop.example/done');
    expect(h.navigated).toEqual([]);
    expect(h.posted).toContainEqual({
      type: 'vpay:redirect',
      url: 'https://shop.example/done',
    });
  });
});

describe('polling', () => {
  it('keeps the payer on the waiting screen when the API stops answering, and can retry', async () => {
    const h = await harness({ pollsBeforeTerminal: 50 });
    await h.controller.start();
    const spy = vi.spyOn(globalThis, 'fetch');
    // A poll budget this short expires before the stub settles, which is
    // what `polling_timeout` looks like to the page.
    const short = new CheckoutController({
      sessionId: h.stub.sessionId,
      credentials: { key: h.stub.publishableKey, clientSecret: h.stub.sessionSecret },
      api: new BrowserCheckoutApi({ baseUrl: h.stub.url }),
      stripe: await loadStripe(h.stub.publishableKey, { baseUrl: h.stub.url }),
      navigate: () => undefined,
      channel: null,
      pollIntervalMs: 1,
      pollTimeoutMs: 10,
    });
    await short.start();
    await short.submitMsisdn(VALID_MSISDN);
    expect(short.state).toMatchObject({ name: 'waiting', notice: 'error.unexpected' });
    spy.mockRestore();
  });
});
