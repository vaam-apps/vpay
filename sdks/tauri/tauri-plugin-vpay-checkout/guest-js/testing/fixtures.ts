/**
 * The doubles this package's own suite runs against: a `fetch` that answers
 * vpay's two browser reads, a clock with no real time in it, a host that
 * hands the test the trigger for the one event, and the two response bodies.
 *
 * Excluded from `dist` by `tsconfig.build.json` and imported only from
 * `*.test.ts`. AGENTS.md's rule 1 forbids a test double reachable from a
 * shipping process; an SDK's own unit-test fixtures are neither.
 *
 * The session body is the interesting one. It carries **no top-level
 * `client_secret`**, because the real server does not send one back — the
 * Flutter e2e suite found that on 2026-09-14, and
 * `@vaam-apps/vpay-stripe-js` still types the field `string`. A fixture that
 * supplied it would make `session.client_secret` look like a usable polling
 * credential in every test, which is the exact mistake this package's
 * controller exists to not make.
 */
import type {
  CheckoutHost,
  CheckoutWindowEvent,
  CheckoutWindowOutcome,
  ShowCheckoutRequest,
  VpayClock,
} from "../types.js";

export const BASE_URL = "https://api.example";
export const PUBLISHABLE_KEY = "pk_test_abcdefghijklmnop";
export const SESSION_ID = "cs_123";
export const INTENT_ID = "pi_456";
/** 32 characters, the floor migration `0023`'s CHECK puts on a secret suffix. */
export const SESSION_SECRET_SUFFIX = "s".repeat(32);
export const INTENT_SECRET_SUFFIX = "i".repeat(32);
export const SESSION_SECRET = `${SESSION_ID}_secret_${SESSION_SECRET_SUFFIX}`;
export const INTENT_SECRET = `${INTENT_ID}_secret_${INTENT_SECRET_SUFFIX}`;
/** `{base}/c/{cs_id}?key={pk}#{cs_secret}` — exactly what `vpay_api` renders (D6). */
export const SESSION_URL = `https://checkout.example/c/${SESSION_ID}?key=${PUBLISHABLE_KEY}#${SESSION_SECRET}`;

/** The thirteen keys `PaymentIntentWithSecret` carries, `client_secret` included. */
export function paymentIntentBody(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    id: INTENT_ID,
    object: "payment_intent",
    amount: 5000,
    currency: "xaf",
    status: "processing",
    payment_method_types: ["mtn_momo"],
    next_action: null,
    last_payment_error: null,
    metadata: {},
    description: null,
    created: 1_757_000_000,
    livemode: false,
    client_secret: INTENT_SECRET,
    ...overrides,
  };
}

/**
 * A hosted `checkout.session` as `GET /v1/browser/checkout/sessions/{id}`
 * renders it — `payment_intent` expanded, and no top-level `client_secret`.
 */
export function checkoutSessionBody(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    object: "checkout.session",
    id: SESSION_ID,
    livemode: false,
    payment_intent: paymentIntentBody(),
    ui_mode: "hosted",
    status: "open",
    payment_status: "unpaid",
    success_url: "https://shop.example/return/{CHECKOUT_SESSION_ID}",
    cancel_url: "https://shop.example/cart",
    return_url: null,
    url: SESSION_URL,
    expires_at: 1_757_086_400,
    created: 1_757_000_000,
    rails: [],
    ...overrides,
  };
}

/** One canned answer, or a function of the request count for a poll sequence. */
export interface FetchScript {
  /** The session read. A `number` answers that status with an empty envelope. */
  session?: Record<string, unknown> | number | undefined;
  /** The intent reads, in order. The last entry repeats once exhausted. */
  intents?: (Record<string, unknown> | number)[] | undefined;
}

export interface ScriptedFetch {
  fetch: typeof fetch;
  /** Every URL the client asked for, in order — so a test can assert the credential never moved. */
  readonly urls: string[];
  readonly sessionReads: number;
  readonly intentReads: number;
}

function respond(entry: Record<string, unknown> | number): Response {
  if (typeof entry === "number") {
    return new Response(
      JSON.stringify({
        error: {
          type: "invalid_request_error",
          code: "resource_missing",
          message: "No such resource.",
        },
      }),
      { status: entry, headers: { "content-type": "application/json" } },
    );
  }
  return new Response(JSON.stringify(entry), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

/**
 * A `fetch` that answers the two `/v1/browser` reads and nothing else.
 *
 * Not a `node:http` server, unlike `sdks/stripe-js`'s own suite: the bytes
 * on the wire are *that* package's contract and are tested there. What is
 * under test here is the state machine above it, so the double is at the
 * seam this package actually owns.
 */
export function scriptedFetch(script: FetchScript): ScriptedFetch {
  const urls: string[] = [];
  let sessionReads = 0;
  let intentReads = 0;
  const intents = script.intents ?? [paymentIntentBody()];

  const impl = (input: RequestInfo | URL): Promise<Response> => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.href
          : input.url;
    urls.push(url);
    if (url.includes("/checkout/sessions/")) {
      sessionReads += 1;
      return Promise.resolve(respond(script.session ?? checkoutSessionBody()));
    }
    if (url.includes("/payment_intents/")) {
      const entry = intents[Math.min(intentReads, intents.length - 1)];
      intentReads += 1;
      if (entry === undefined) {
        throw new Error("scriptedFetch: no intent response scripted");
      }
      return Promise.resolve(respond(entry));
    }
    throw new Error("scriptedFetch: unexpected request path");
  };

  return {
    fetch: impl,
    urls,
    get sessionReads() {
      return sessionReads;
    },
    get intentReads() {
      return intentReads;
    },
  };
}

/**
 * A clock with no real time in it: `delay` records the wait and returns
 * immediately, having advanced `now` by exactly that much.
 *
 * Advancing on `delay` rather than making the test advance it is what keeps
 * a poll-budget test honest *and* fast — the loop really does run until its
 * budget is spent, it just spends it instantly.
 */
export interface FakeClock extends VpayClock {
  readonly delays: number[];
  advance(ms: number): void;
}

export function fakeClock(startMs = 1_757_000_000_000): FakeClock {
  let current = startMs;
  const delays: number[] = [];
  return {
    now: () => current,
    delay: (ms: number) => {
      delays.push(ms);
      current += ms;
      return Promise.resolve();
    },
    advance: (ms: number) => {
      current += ms;
    },
    delays,
  };
}

/** A host that records what it was shown and lets the test fire the one event. */
export interface FakeHost extends CheckoutHost {
  readonly requests: ShowCheckoutRequest[];
  readonly dismissals: number;
  /** Fires the host's event. Returns `false` when `show` has not been called. */
  report(outcome: CheckoutWindowOutcome, reachedUrl?: string | null): boolean;
}

export interface FakeHostOptions {
  /** Report this outcome from inside `show`, before it resolves. */
  reportOnShow?: CheckoutWindowOutcome | undefined;
  /** Reject `show` with this value. */
  rejectWith?: unknown;
}

export function fakeHost(options: FakeHostOptions = {}): FakeHost {
  const requests: ShowCheckoutRequest[] = [];
  let dismissals = 0;
  let sink: ((event: CheckoutWindowEvent) => void) | null = null;

  return {
    show(request, onEvent) {
      requests.push(request);
      sink = onEvent;
      if (options.rejectWith !== undefined) {
        // Deliberately whatever the test supplied, Error or not: a real
        // Tauri command rejects with a plain **string** message, and the
        // rule this package must keep is that `start` never reads it.
        // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors
        return Promise.reject(options.rejectWith);
      }
      if (options.reportOnShow !== undefined) {
        onEvent({ outcome: options.reportOnShow, reachedUrl: null });
      }
      return Promise.resolve();
    },
    dismiss() {
      dismissals += 1;
      return Promise.resolve();
    },
    report(outcome, reachedUrl = null) {
      if (sink === null) {
        return false;
      }
      sink({ outcome, reachedUrl });
      return true;
    },
    requests,
    get dismissals() {
      return dismissals;
    },
  };
}
