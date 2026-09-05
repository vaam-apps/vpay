/**
 * Where a credential is allowed to appear, and where it must never.
 *
 * The honest version of "no secret in a request URL": on this surface a
 * secret **is** in a request URL, by contract —
 * `GET /v1/browser/checkout/sessions/{id}?key&client_secret` and
 * `GET /v1/browser/payment_intents/{id}?key&client_secret` are how the
 * browser authenticates (`docs/flows/browser-checkout.md`). A test that
 * asserted otherwise would be asserting that the page cannot work.
 *
 * What is actually checkable, and what this file checks, is narrower and
 * more useful:
 *
 * 1. **No `console.*` call ever carries a secret.** A payment page's console
 *    is read over a payer's shoulder, scraped by extensions and shipped to
 *    error reporters.
 * 2. **No URL this page navigates to, or asks its parent to navigate to,
 *    carries a secret** — not the merchant's `success_url`, not the rail's
 *    redirect.
 * 3. **No `postMessage` payload carries a secret.**
 * 4. **A secret only ever appears as the value of the parameter named for
 *    it, and never in a path segment** — so it cannot end up in a route
 *    pattern, a metrics label or a proxy's access-log path.
 * 5. **The session secret and the intent secret are not interchanged** —
 *    the intent's never goes to a checkout-session route and the session's
 *    never goes to a payment-intent route.
 * 6. **No credential is retained on the state the screens render from.** The
 *    spies above cannot see this one: a `client_secret` kept on
 *    `context.session` leaks through a devtools snapshot or an error
 *    reporter, neither of which is a `console` call or a `postMessage`. So
 *    the state is walked directly, and the *only* path allowed to hold a
 *    secret is `context.intent.client_secret` — the credential the
 *    controller confirms and polls with.
 * 7. **The return trip's `t=` token obeys the same rules as a secret.** It
 *    authorises reading and polling one session; it must appear in exactly
 *    one place, the return read's query string.
 *
 * All three paths a payer can take are traced, not just the first:
 * {@link traceAPayment} (MTN push), {@link traceARedirect} (Orange, framed
 * and not) and {@link traceAReturn} (the return page, which is a different
 * document with a different credential).
 */
import { loadStripe } from '@vaam-apps/vpay-stripe-js';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { startCheckoutStub, type CheckoutStub } from '../testing/browser-stub';
import { BrowserCheckoutApi } from './api';
import { CheckoutController } from './controller';
import type { ChildMessage, FrameChannel } from './frame';
import { ReturnController } from './return';

const CONSOLE_METHODS = ['log', 'info', 'warn', 'error', 'debug', 'trace'] as const;

let open: CheckoutStub | null = null;
afterEach(async () => {
  await open?.close();
  open = null;
  vi.restoreAllMocks();
});

interface Trace {
  stub: CheckoutStub;
  consoleArgs: string[];
  navigated: string[];
  posted: ChildMessage[];
  fetched: string[];
  /** The state the screens would render from at the end of the trace. */
  state: unknown;
}

/** Records every `console.*` argument for the rest of the test. */
function spyOnConsole(sink: string[]): void {
  for (const method of CONSOLE_METHODS) {
    vi.spyOn(console, method).mockImplementation((...args: unknown[]) => {
      for (const arg of args) {
        sink.push(typeof arg === 'string' ? arg : safeStringify(arg));
      }
    });
  }
}

/** Records every requested URL, passing the call through to the real socket. */
function spyOnFetch(sink: string[]): { mockRestore: () => void } {
  const realFetch = globalThis.fetch.bind(globalThis);
  return vi
    .spyOn(globalThis, 'fetch')
    .mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      // Not `String(input)`: a `Request` stringifies to "[object Request]",
      // and this sink is what the credential trace greps for a secret.
      sink.push(
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.href
            : input.url,
      );
      return realFetch(input, init);
    });
}

function recordingChannel(posted: ChildMessage[]): FrameChannel {
  return {
    parentOrigin: 'https://shop.example',
    post: (message) => posted.push(message),
    postHeight: () => undefined,
    dispose: () => undefined,
  };
}

/**
 * Every path through `value` whose string holds `_secret_`, as dotted paths.
 *
 * The point of returning paths rather than a boolean: the intent's own
 * `client_secret` is *supposed* to be on the state — the controller confirms
 * and polls with it — so the assertion that means something is **which**
 * paths hold one, not whether any does.
 */
function secretPaths(value: unknown, path = '$'): string[] {
  if (typeof value === 'string') {
    return value.includes('_secret_') ? [path] : [];
  }
  if (Array.isArray(value)) {
    return value.flatMap((item, index) => secretPaths(item, `${path}[${index}]`));
  }
  if (typeof value === 'object' && value !== null) {
    return Object.entries(value).flatMap(([key, item]) => secretPaths(item, `${path}.${key}`));
  }
  return [];
}

/** Drives a whole MTN payment with every observable channel recorded. */
async function traceAPayment(framed: boolean): Promise<Trace> {
  const stub = await startCheckoutStub({ pollsBeforeTerminal: 1, uiMode: 'embedded' });
  open = stub;

  const consoleArgs: string[] = [];
  spyOnConsole(consoleArgs);
  const fetched: string[] = [];
  const spy = spyOnFetch(fetched);

  const navigated: string[] = [];
  const posted: ChildMessage[] = [];
  const controller = new CheckoutController({
    sessionId: stub.sessionId,
    credentials: { key: stub.publishableKey, clientSecret: stub.sessionSecret },
    api: new BrowserCheckoutApi({ baseUrl: stub.url }),
    stripe: await loadStripe(stub.publishableKey, { baseUrl: stub.url }),
    navigate: (url) => navigated.push(url),
    channel: framed ? recordingChannel(posted) : null,
    pollIntervalMs: 1,
    pollTimeoutMs: 4_000,
  });

  await controller.start();
  await controller.submitMsisdn('237600000400');
  expect(controller.state.name).toBe('outcome');
  controller.forward(`https://shop.example/done?sid=${stub.sessionId}`);

  spy.mockRestore();
  return { stub, consoleArgs, navigated, posted, fetched, state: controller.state };
}

/**
 * The Orange path: a confirm, then a redirect this page either performs or
 * asks its parent to perform. Both halves carry a URL, and neither may carry
 * a credential.
 */
async function traceARedirect(framed: boolean): Promise<Trace> {
  const stub = await startCheckoutStub({
    paymentMethodTypes: ['orange_money'],
    uiMode: framed ? 'embedded' : 'hosted',
    redirectUrl: 'https://rail.example/stub-hosted-page/tok_abc',
  });
  open = stub;

  const consoleArgs: string[] = [];
  spyOnConsole(consoleArgs);
  const fetched: string[] = [];
  const spy = spyOnFetch(fetched);

  const navigated: string[] = [];
  const posted: ChildMessage[] = [];
  const controller = new CheckoutController({
    sessionId: stub.sessionId,
    credentials: { key: stub.publishableKey, clientSecret: stub.sessionSecret },
    api: new BrowserCheckoutApi({ baseUrl: stub.url }),
    stripe: await loadStripe(stub.publishableKey, { baseUrl: stub.url }),
    navigate: (url) => navigated.push(url),
    channel: framed ? recordingChannel(posted) : null,
    pollIntervalMs: 1,
    pollTimeoutMs: 4_000,
  });

  await controller.start();
  await controller.startRedirect();
  expect(controller.state.name).toBe('redirecting');

  spy.mockRestore();
  return { stub, consoleArgs, navigated, posted, fetched, state: controller.state };
}

/**
 * The return page, which is a different document with a different
 * credential: the `t=` token, and no `client_secret` at all.
 *
 * The rail is driven to its answer **before** the spies are installed, so
 * what the trace holds is the return page's own traffic and nothing the
 * checkout page did earlier. One read then settles it — the return route is
 * also the status query — which is what makes "exactly one" assertable.
 */
async function traceAReturn(framed: boolean): Promise<Trace> {
  const stub = await startCheckoutStub({
    paymentMethodTypes: ['orange_money'],
    uiMode: framed ? 'embedded' : 'hosted',
    pollsBeforeTerminal: 0,
  });
  open = stub;
  // The confirm an Orange payer's browser already made, before it left for
  // the rail. Raw, because the return page holds no credential that could.
  await fetch(`${stub.url}/v1/browser/payment_intents/pi_test_stub0000000000000001/confirm`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      key: stub.publishableKey,
      client_secret: stub.intentSecret,
      'payment_method_data[type]': 'orange_money',
    }).toString(),
  });

  const consoleArgs: string[] = [];
  spyOnConsole(consoleArgs);
  const fetched: string[] = [];
  const spy = spyOnFetch(fetched);

  const navigated: string[] = [];
  const posted: ChildMessage[] = [];
  const controller = new ReturnController({
    sessionId: stub.sessionId,
    credentials: { key: stub.publishableKey, returnToken: stub.returnToken },
    api: new BrowserCheckoutApi({ baseUrl: stub.url }),
    navigate: (url) => navigated.push(url),
    channel: framed ? recordingChannel(posted) : null,
    intervalMs: 0,
    timeoutMs: 2_000,
    sleep: () => Promise.resolve(),
  });

  await controller.start();
  expect(controller.state.name).toBe('outcome');
  controller.forward(`https://shop.example/done?sid=${stub.sessionId}`);

  spy.mockRestore();
  return { stub, consoleArgs, navigated, posted, fetched, state: controller.state };
}

function safeStringify(value: unknown): string {
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

describe('a whole payment leaks nothing', () => {
  it('writes no secret to any console method', async () => {
    const trace = await traceAPayment(false);
    const joined = trace.consoleArgs.join('\n');
    expect(joined).not.toContain(trace.stub.sessionSecret);
    expect(joined).not.toContain(trace.stub.intentSecret);
  });

  it('navigates to no URL carrying a secret', async () => {
    const trace = await traceAPayment(false);
    expect(trace.navigated.length).toBeGreaterThan(0);
    for (const url of trace.navigated) {
      expect(url).not.toContain(trace.stub.sessionSecret);
      expect(url).not.toContain(trace.stub.intentSecret);
      expect(url).not.toContain('_secret_');
    }
  });

  it('posts no secret to the parent, in any message', async () => {
    const trace = await traceAPayment(true);
    expect(trace.posted.length).toBeGreaterThan(0);
    const serialised = JSON.stringify(trace.posted);
    expect(serialised).not.toContain(trace.stub.sessionSecret);
    expect(serialised).not.toContain(trace.stub.intentSecret);
    expect(serialised).not.toContain('_secret_');
  });

  it('puts a secret only in the query parameter named for it, never in a path', async () => {
    const trace = await traceAPayment(false);
    expect(trace.fetched.length).toBeGreaterThan(2);
    for (const raw of trace.fetched) {
      const url = new URL(raw);
      expect(url.pathname, raw).not.toContain('_secret_');
      for (const [name, value] of url.searchParams) {
        if (value.includes('_secret_')) {
          expect(name, `${raw} carries a secret as "${name}"`).toBe('client_secret');
        }
      }
    }
  });

  it('sends each secret only to the routes it authenticates', async () => {
    const trace = await traceAPayment(false);
    for (const raw of trace.fetched) {
      const url = new URL(raw);
      const presented = url.searchParams.get('client_secret');
      if (presented === null) {
        continue;
      }
      if (url.pathname.includes('/checkout/sessions/')) {
        expect(presented, raw).toBe(trace.stub.sessionSecret);
      } else {
        expect(url.pathname).toContain('/payment_intents/');
        expect(presented, raw).toBe(trace.stub.intentSecret);
      }
    }
  });

  it('sends the publishable key on every call, since it is not a secret', async () => {
    const trace = await traceAPayment(false);
    for (const raw of trace.fetched) {
      const url = new URL(raw);
      if (url.pathname.startsWith('/v1/browser') && !url.pathname.endsWith('/confirm')) {
        expect(url.searchParams.get('key'), raw).toBe(trace.stub.publishableKey);
      }
    }
  });

  it('keeps the confirm’s credentials in the body, out of the URL entirely', async () => {
    const trace = await traceAPayment(false);
    const confirm = trace.stub.requests.find((r) => r.method === 'POST');
    expect(confirm?.url).not.toContain('_secret_');
    expect(confirm?.url).not.toContain('key=');
    expect(confirm?.body).toContain('client_secret=');
  });
});

describe('the state the screens render from carries no credential but the one it must', () => {
  it('holds a secret at exactly one path: the intent’s own client_secret', async () => {
    const trace = await traceAPayment(false);
    // Not "no secret anywhere": the controller confirms and polls with
    // `context.intent.client_secret`, so it is on the state by necessity.
    // The session's is not, and `contextOf` is what strips it.
    expect(secretPaths(trace.state)).toEqual(['$.context.intent.client_secret']);
  });

  it('does the same on the Orange path, where the state also holds a rail URL', async () => {
    const trace = await traceARedirect(false);
    expect(secretPaths(trace.state)).toEqual(['$.context.intent.client_secret']);
  });

  it('holds none at all on the return page, which has no secret to hold', async () => {
    const trace = await traceAReturn(false);
    expect(secretPaths(trace.state)).toEqual([]);
    expect(JSON.stringify(trace.state)).not.toContain(trace.stub.returnToken);
  });
});

describe('the Orange redirect leaks nothing', () => {
  it('writes no secret to any console method', async () => {
    const trace = await traceARedirect(false);
    const joined = trace.consoleArgs.join('\n');
    expect(joined).not.toContain(trace.stub.sessionSecret);
    expect(joined).not.toContain(trace.stub.intentSecret);
  });

  it('sends the payer to the rail’s URL and nothing else — no credential appended', async () => {
    const trace = await traceARedirect(false);
    expect(trace.navigated).toEqual(['https://rail.example/stub-hosted-page/tok_abc']);
    for (const url of trace.navigated) {
      expect(url).not.toContain(trace.stub.sessionSecret);
      expect(url).not.toContain(trace.stub.intentSecret);
      expect(url).not.toContain('_secret_');
      expect(url).not.toContain(trace.stub.returnToken);
    }
  });

  it('asks the parent to navigate with a payload carrying no secret', async () => {
    const trace = await traceARedirect(true);
    expect(trace.posted).toEqual([
      { type: 'vpay:redirect', url: 'https://rail.example/stub-hosted-page/tok_abc' },
    ]);
    const serialised = JSON.stringify(trace.posted);
    expect(serialised).not.toContain(trace.stub.sessionSecret);
    expect(serialised).not.toContain(trace.stub.intentSecret);
    expect(serialised).not.toContain('_secret_');
    // The framed page must not navigate itself as well as asking.
    expect(trace.navigated).toEqual([]);
  });

  it('keeps the confirm’s credential in the body, out of every URL', async () => {
    const trace = await traceARedirect(false);
    for (const raw of trace.fetched) {
      expect(new URL(raw).pathname).not.toContain('_secret_');
    }
    const confirm = trace.stub.requests.find((r) => r.method === 'POST');
    expect(confirm?.url).not.toContain('_secret_');
    expect(confirm?.body).toContain('client_secret=');
  });
});

describe('the return page’s token', () => {
  it('appears in exactly one fetch URL, as the `t` parameter of the return read', async () => {
    const trace = await traceAReturn(false);
    const carrying = trace.fetched.filter((raw) => raw.includes(trace.stub.returnToken));
    expect(carrying).toHaveLength(1);
    const url = new URL(carrying[0] as string);
    expect(url.pathname).toMatch(/\/v1\/browser\/checkout\/sessions\/[^/]+\/return$/);
    expect(url.pathname).not.toContain(trace.stub.returnToken);
    expect(url.searchParams.get('t')).toBe(trace.stub.returnToken);
  });

  it('is never written to a console method', async () => {
    const trace = await traceAReturn(false);
    expect(trace.consoleArgs.join('\n')).not.toContain(trace.stub.returnToken);
  });

  it('is not in the vpay:complete message the return page posts', async () => {
    const trace = await traceAReturn(true);
    expect(trace.posted).toEqual([
      { type: 'vpay:complete', session: trace.stub.sessionId, status: 'complete' },
    ]);
    const serialised = JSON.stringify(trace.posted);
    expect(serialised).not.toContain(trace.stub.returnToken);
    expect(serialised).not.toContain('_secret_');
  });

  it('is not in the URL the payer is forwarded to', async () => {
    const trace = await traceAReturn(false);
    expect(trace.navigated).toEqual([`https://shop.example/done?sid=${trace.stub.sessionId}`]);
    for (const url of trace.navigated) {
      expect(url).not.toContain(trace.stub.returnToken);
      expect(url).not.toContain('_secret_');
    }
  });

  it('brings back no client_secret to leak: the return route renders neither', async () => {
    const trace = await traceAReturn(false);
    const body = trace.stub.requests.find((r) => r.url.includes('/return?'));
    expect(body).toBeDefined();
    expect(JSON.stringify(trace.state)).not.toContain('_secret_');
  });
});

describe('an error path leaks nothing either', () => {
  it('reports an unreachable API without echoing the URL it could not reach', async () => {
    const stub = await startCheckoutStub();
    const url = stub.url;
    const secret = stub.sessionSecret;
    await stub.close();

    const consoleArgs: string[] = [];
    for (const method of CONSOLE_METHODS) {
      vi.spyOn(console, method).mockImplementation((...args: unknown[]) => {
        consoleArgs.push(args.map(safeStringify).join(' '));
      });
    }

    const controller = new CheckoutController({
      sessionId: 'cs_test_stub0000000000000001',
      credentials: { key: 'pk_test_0123456789abcdefghij', clientSecret: secret },
      api: new BrowserCheckoutApi({ baseUrl: url }),
      stripe: await loadStripe('pk_test_0123456789abcdefghij', { baseUrl: url }),
      navigate: () => undefined,
      channel: null,
    });
    await controller.start();

    const state = controller.state;
    expect(state).toMatchObject({ name: 'error', error: { code: 'error.network' } });
    expect(JSON.stringify(state)).not.toContain(secret);
    expect(consoleArgs.join('\n')).not.toContain(secret);
  });
});
