/**
 * A real `node:http` server standing in for vpay's `/v1/browser` surface —
 * the three checkout routes this app reads and the two payment-intent
 * routes `@vaam-apps/vpay-stripe-js` reads on its behalf.
 *
 * **This is not a test double reachable from a shipping process.** AGENTS.md
 * rule 1 and ADR-0006 forbid a mock, fake or stub being linked into
 * `vpay-server` or `vpay-worker-bin`; this file is TypeScript in a frontend
 * app, imported only from `*.test.ts`, and excluded from the Next build by
 * `next.config.ts`'s page/route conventions — nothing under `src/testing`
 * is reachable from `app/`. Nothing in the Rust workspace can reach it, and
 * no byte of the built page can either.
 *
 * A real server on a real socket, not a patched `fetch`, for the same reason
 * `sdks/stripe-js/src/testing/browser-stub.ts` is one: the assertions that
 * matter are about **bytes on the wire** — which parameter carries the
 * secret, that nothing carries it in a path segment, that the confirm is
 * form-encoded — and a mocked `fetch` would assert the arguments this app
 * passes to a function the test also controls.
 *
 * It models just enough behaviour to drive the page's whole state machine:
 * a confirm moves the intent, a poll settles it after a set number of
 * queries, and the settlement flips the session the way lane 1's worker hook
 * does. It invents no status the wire contract does not define.
 */
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';

import type { FailureCode } from '@vaam-apps/vpay-stripe-js';

import type {
  CheckoutSession,
  CheckoutSessionPaymentStatus,
  CheckoutSessionStatus,
} from '../lib/types';

export interface RecordedRequest {
  method: string;
  /** The raw request target, query string included — asserted on verbatim. */
  url: string;
  headers: Record<string, string | string[] | undefined>;
  /** The raw request body, undecoded. */
  body: string;
}

/** How the stub's intent ends once it has been polled enough times. */
export type StubTerminal =
  | { kind: 'succeeded' }
  | { kind: 'canceled' }
  | { kind: 'failed'; failure: FailureCode; message: string };

/**
 * What the stub renders as the response's `merchant` member.
 *
 * Three shapes because the server can genuinely answer with three: a name
 * (lane 1b's shape), no member at all (a merchant with no display name, or
 * a server version where the member has not landed), and something that is
 * not `{ name: string }` (a `merchant_name` rename arriving as a bare
 * string). The page must pay a payment in all three — `merchantOf` in
 * `machine.ts` — so the stub has to be able to send all three.
 */
export type StubMerchant =
  | { kind: 'named'; name: string }
  | { kind: 'absent' }
  | { kind: 'malformed'; value: unknown };

export interface CheckoutStubOptions {
  publishableKey?: string;
  sessionId?: string;
  sessionSecret?: string;
  returnToken?: string;
  intentId?: string;
  intentSecret?: string;
  /** Defaults to `{ kind: 'named', name: 'Boutique Test' }`. */
  merchant?: StubMerchant;
  amount?: number;
  currency?: string;
  paymentMethodTypes?: string[];
  uiMode?: 'hosted' | 'embedded';
  successUrl?: string | null;
  cancelUrl?: string | null;
  returnUrl?: string | null;
  origins?: string[];
  /** Where a redirect rail sends the payer. */
  redirectUrl?: string;
  /**
   * `true` models a PaymentIntent that belongs to **no** checkout session —
   * one a merchant created directly through `/v1/payment_intents`. The two
   * checkout-session routes then answer 404 (there is no session), and the
   * confirm applies the server's `return_url` rule in full: see the confirm
   * handler below.
   */
  standaloneIntent?: boolean;
  /** Status queries answered as still moving before {@link terminal} is applied. */
  pollsBeforeTerminal?: number;
  terminal?: StubTerminal;
}

export interface CheckoutStub {
  /** Origin to hand to `BrowserCheckoutApi` and `loadStripe` as `baseUrl`. */
  url: string;
  publishableKey: string;
  sessionId: string;
  sessionSecret: string;
  returnToken: string;
  intentSecret: string;
  requests: RecordedRequest[];
  /** Every URL the stub was asked for, in order. */
  urls(): string[];
  close(): Promise<void>;
}

const DEFAULTS = {
  publishableKey: 'pk_test_0123456789abcdefghij',
  sessionId: 'cs_test_stub0000000000000001',
  sessionSecretSuffix: 'c'.repeat(32),
  returnToken: 'r'.repeat(40),
  intentId: 'pi_test_stub0000000000000001',
  intentSecretSuffix: 'a'.repeat(32),
  merchantName: 'Boutique Test',
  amount: 5000,
  currency: 'xaf',
  redirectUrl: 'https://rail.example/stub-hosted-page/tok_123',
} as const;

/**
 * `vpay_api::error_envelope_with_param`, key for key.
 *
 * `param` is **absent** rather than present-and-null when the error names no
 * parameter — `ApiError::param()` returns `Option`, Stripe omits the key,
 * and an SDK that tests `'param' in error` has to see the same thing.
 */
export function errorEnvelope(
  type: string,
  code: string,
  message: string,
  param?: string,
): { error: Record<string, string> } {
  const error: Record<string, string> = { type, code, message };
  if (param !== undefined) {
    error['param'] = param;
  }
  return { error };
}

/**
 * `ApiError::invalid_param(param, message)` on the wire: 400, the
 * `InvalidRequest` category's `type` and `code`, and the offending field
 * name in `param`.
 */
export function invalidParamEnvelope(
  param: string,
  message: string,
): { error: Record<string, string> } {
  return errorEnvelope('invalid_request_error', 'invalid_request', message, param);
}

/**
 * Rails whose confirm answers with a `redirect_to_url` next action.
 *
 * The stub's own list, not `RAIL_PAGE_FLOWS` from `src/lib/rails.ts`: this
 * models the **server's** knowledge of its rails, and reading the page's map
 * here would make the two agree by construction rather than by contract.
 */
const REDIRECT_RAILS: ReadonlySet<string> = new Set(['orange_money']);

/** The uniform 404 every credential failure on the browser surface renders. */
export function notFoundEnvelope(resource: string, id: string): {
  error: Record<string, string>;
} {
  return errorEnvelope('invalid_request_error', 'resource_missing', `No such ${resource}: ${id}`);
}

function json(res: ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify(body));
}

interface IntentModel {
  id: string;
  status: 'requires_payment_method' | 'requires_action' | 'processing' | 'succeeded' | 'canceled';
  next_action: { type: 'redirect_to_url'; redirect_to_url: { url: string; return_url: string | null } } | null;
  last_payment_error: { code: FailureCode; message: string } | null;
}

/**
 * Starts the stub on an ephemeral loopback port.
 *
 * Each test starts its own, so the suite is order-independent and no test
 * can see another's mutations.
 */
export function startCheckoutStub(options: CheckoutStubOptions = {}): Promise<CheckoutStub> {
  const publishableKey = options.publishableKey ?? DEFAULTS.publishableKey;
  const sessionId = options.sessionId ?? DEFAULTS.sessionId;
  const sessionSecret = options.sessionSecret ?? `${sessionId}_secret_${DEFAULTS.sessionSecretSuffix}`;
  const returnToken = options.returnToken ?? DEFAULTS.returnToken;
  const intentId = options.intentId ?? DEFAULTS.intentId;
  const intentSecret = options.intentSecret ?? `${intentId}_secret_${DEFAULTS.intentSecretSuffix}`;
  const amount = options.amount ?? DEFAULTS.amount;
  const currency = options.currency ?? DEFAULTS.currency;
  const paymentMethodTypes = options.paymentMethodTypes ?? ['mtn_momo'];
  const merchant: StubMerchant = options.merchant ?? {
    kind: 'named',
    name: DEFAULTS.merchantName,
  };
  const uiMode = options.uiMode ?? 'hosted';
  const redirectUrl = options.redirectUrl ?? DEFAULTS.redirectUrl;
  const origins = options.origins ?? [];
  const hasSession = options.standaloneIntent !== true;
  const terminal: StubTerminal = options.terminal ?? { kind: 'succeeded' };
  let pollsRemaining = options.pollsBeforeTerminal ?? 1;

  const intent: IntentModel = {
    id: intentId,
    status: 'requires_payment_method',
    next_action: null,
    last_payment_error: null,
  };
  let sessionStatus: CheckoutSessionStatus = 'open';
  let paymentStatus: CheckoutSessionPaymentStatus = 'unpaid';
  let confirmedReturnUrl: string | null = null;

  const requests: RecordedRequest[] = [];

  const sessionObject = (withSecret: boolean): CheckoutSession => {
    const session: CheckoutSession = {
      id: sessionId,
      object: 'checkout.session',
      livemode: false,
      ui_mode: uiMode,
      status: sessionStatus,
      payment_status: paymentStatus,
      success_url: options.successUrl ?? (uiMode === 'hosted' ? 'https://shop.example/ok?sid={CHECKOUT_SESSION_ID}' : null),
      cancel_url: options.cancelUrl ?? (uiMode === 'hosted' ? 'https://shop.example/cancel' : null),
      return_url: options.returnUrl ?? (uiMode === 'embedded' ? 'https://shop.example/done?sid={CHECKOUT_SESSION_ID}' : null),
      url: uiMode === 'hosted' ? `https://checkout.example/c/${sessionId}` : null,
      expires_at: 1_757_000_000,
      created: 1_756_913_600,
    };
    if (withSecret) {
      session.client_secret = sessionSecret;
    }
    return session;
  };

  /**
   * The `merchant` member, as a spreadable fragment: `{ kind: 'absent' }`
   * contributes **no key at all**, which is the shape a `'merchant' in body`
   * check has to see.
   */
  const merchantMember = (): Record<string, unknown> => {
    if (merchant.kind === 'absent') {
      return {};
    }
    return { merchant: merchant.kind === 'named' ? { name: merchant.name } : merchant.value };
  };

  const intentObject = (withSecret: boolean): Record<string, unknown> => {
    const body: Record<string, unknown> = {
      id: intent.id,
      object: 'payment_intent',
      amount,
      currency,
      status: intent.status,
      payment_method_types: paymentMethodTypes,
      next_action: intent.next_action,
      last_payment_error: intent.last_payment_error,
      metadata: {},
      description: null,
      created: 1_756_913_600,
      livemode: false,
    };
    if (withSecret) {
      body['client_secret'] = intentSecret;
    }
    return body;
  };

  /** What lane 1's worker hook does in the settlement transaction. */
  const settle = (): void => {
    if (terminal.kind === 'succeeded') {
      intent.status = 'succeeded';
      intent.next_action = null;
      sessionStatus = 'complete';
      paymentStatus = 'paid';
      return;
    }
    if (terminal.kind === 'canceled') {
      intent.status = 'canceled';
      intent.next_action = null;
      sessionStatus = 'expired';
      paymentStatus = 'failed';
      return;
    }
    intent.status = 'requires_payment_method';
    intent.next_action = null;
    intent.last_payment_error = { code: terminal.failure, message: terminal.message };
    sessionStatus = 'expired';
    paymentStatus = 'failed';
  };

  const server = createServer((req: IncomingMessage, res: ServerResponse) => {
    const chunks: Buffer[] = [];
    req.on('data', (chunk: Buffer) => chunks.push(chunk));
    req.on('end', () => {
      const record: RecordedRequest = {
        method: req.method ?? '',
        url: req.url ?? '',
        headers: req.headers,
        body: Buffer.concat(chunks).toString('utf8'),
      };
      requests.push(record);

      const parsed = new URL(record.url, 'http://stub.invalid');
      const path = parsed.pathname;
      const query = parsed.searchParams;

      if (path === '/v1/browser/checkout/origins') {
        if (query.get('key') !== publishableKey) {
          json(res, 404, notFoundEnvelope('publishable key', query.get('key') ?? ''));
          return;
        }
        json(res, 200, { origins });
        return;
      }

      if (path === `/v1/browser/checkout/sessions/${sessionId}`) {
        if (!hasSession) {
          json(res, 404, notFoundEnvelope('checkout session', sessionId));
          return;
        }
        if (query.get('key') !== publishableKey || query.get('client_secret') !== sessionSecret) {
          json(res, 404, notFoundEnvelope('checkout session', sessionId));
          return;
        }
        json(res, 200, {
          ...sessionObject(true),
          payment_intent: intentObject(true),
          ...merchantMember(),
        });
        return;
      }

      if (path === `/v1/browser/checkout/sessions/${sessionId}/return`) {
        if (!hasSession) {
          json(res, 404, notFoundEnvelope('checkout session', sessionId));
          return;
        }
        if (query.get('key') !== publishableKey || query.get('t') !== returnToken) {
          json(res, 404, notFoundEnvelope('checkout session', sessionId));
          return;
        }
        // Every read of the return route is also a status query: the page
        // has no other way to learn the outcome.
        if (intent.status === 'processing' || intent.status === 'requires_action') {
          if (pollsRemaining <= 0) {
            settle();
          } else {
            pollsRemaining -= 1;
          }
        }
        json(res, 200, {
          ...sessionObject(false),
          payment_intent: intentObject(false),
          ...merchantMember(),
        });
        return;
      }

      if (path === `/v1/browser/payment_intents/${intentId}` && record.method === 'GET') {
        if (query.get('key') !== publishableKey || query.get('client_secret') !== intentSecret) {
          json(res, 404, notFoundEnvelope('payment intent', intentId));
          return;
        }
        if (intent.status === 'processing' || intent.status === 'requires_action') {
          if (pollsRemaining <= 0) {
            settle();
          } else {
            pollsRemaining -= 1;
          }
        }
        json(res, 200, intentObject(true));
        return;
      }

      if (path === `/v1/browser/payment_intents/${intentId}/confirm` && record.method === 'POST') {
        const form = new URLSearchParams(record.body);
        if (form.get('key') !== publishableKey || form.get('client_secret') !== intentSecret) {
          json(res, 404, notFoundEnvelope('payment intent', intentId));
          return;
        }
        if (intent.status !== 'requires_payment_method' || intent.last_payment_error !== null) {
          json(
            res,
            409,
            errorEnvelope('invalid_request_error', 'intent_already_confirmed', 'This PaymentIntent already has a charge.'),
          );
          return;
        }
        confirmedReturnUrl = form.get('return_url');
        const railCode = form.get('payment_method_data[type]') ?? '';
        // The server's rule, mirrored rather than assumed away.
        //
        // A redirect rail needs somewhere to send the payer back to. The
        // merchant normally names it (`return_url` on the confirm); the one
        // case where it may be absent is a confirm on an intent that belongs
        // to an **open checkout session**, because the server then
        // substitutes that session's own return page. So the hosted page
        // sends none — and this stub must refuse the same call for an intent
        // with no open session, or it would be quietly certifying a request
        // the API rejects.
        if (
          REDIRECT_RAILS.has(railCode) &&
          (confirmedReturnUrl === null || confirmedReturnUrl.length === 0) &&
          !(hasSession && sessionStatus === 'open')
        ) {
          json(
            res,
            400,
            invalidParamEnvelope(
              'return_url',
              'A return_url is required to confirm a payment method that redirects, unless the PaymentIntent belongs to an open Checkout Session.',
            ),
          );
          return;
        }
        if (REDIRECT_RAILS.has(railCode)) {
          intent.status = 'requires_action';
          intent.next_action = {
            type: 'redirect_to_url',
            redirect_to_url: { url: redirectUrl, return_url: confirmedReturnUrl },
          };
        } else {
          intent.status = 'processing';
        }
        json(res, 200, intentObject(true));
        return;
      }

      json(res, 404, notFoundEnvelope('route', path));
    });
  });

  return new Promise<CheckoutStub>((resolve, reject) => {
    const onStartupError = (err: Error): void => reject(err);
    server.once('error', onStartupError);
    server.listen(0, '127.0.0.1', () => {
      server.off('error', onStartupError);
      const address = server.address() as AddressInfo;
      resolve({
        url: `http://127.0.0.1:${address.port}`,
        publishableKey,
        sessionId,
        sessionSecret,
        returnToken,
        intentSecret,
        requests,
        urls: () => requests.map((r) => r.url),
        close: () =>
          new Promise<void>((res, rej) => {
            server.closeAllConnections();
            server.close((err) => (err ? rej(err) : res()));
          }),
      });
    });
  });
}
