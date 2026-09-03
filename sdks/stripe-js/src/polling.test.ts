/**
 * `waitForPaymentIntent`, in two halves.
 *
 * **Transitions** run against the real `node:http` stub with real timers and
 * a 5 ms interval: the property under test is which statuses end the poll,
 * and that has to be decided from a body that actually crossed a socket.
 *
 * **Timing** runs under `vi.useFakeTimers()` with an injected `fetch` that
 * answers from memory. Fake timers and a real socket fight each other —
 * `fetch` is undici, which drives its own timer wheel, so freezing the clock
 * makes the wire half nondeterministic. The property under test in that
 * half is clock arithmetic (the deadline, the jitter bounds, the clamp of
 * the last sleep), and injecting `fetch` is what lets it be asserted
 * exactly rather than approximately. Neither half is a mocked assertion
 * about the package's own calls: the first checks bytes, the second checks
 * the schedule.
 *
 * See `src/testing/browser-stub.ts` on why an SDK's own test server is not a
 * test double in ADR-0006's sense.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadStripe } from "./index.js";
import {
  json,
  notFoundEnvelope,
  samplePaymentIntent,
  startBrowserStub,
  type BrowserStub,
} from "./testing/browser-stub.js";
import type { Stripe } from "./types.js";

const PK = "pk_test_abcdefghijklmnop";
const SECRET = `pi_123_secret_${"a".repeat(32)}`;

const stubs: BrowserStub[] = [];

afterEach(async () => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  await Promise.all(stubs.splice(0).map((stub) => stub.close()));
});

/** A stub that walks a scripted list of responses, repeating the last one. */
async function stripeWalking(
  script: Array<{ status: number; body: unknown }>,
): Promise<{ stub: BrowserStub; stripe: Stripe }> {
  let index = 0;
  const stub = await startBrowserStub((_req, res) => {
    const step = script[Math.min(index, script.length - 1)]!;
    index += 1;
    json(res, step.status, step.body);
  });
  stubs.push(stub);
  return { stub, stripe: await loadStripe(PK, { baseUrl: stub.url }) };
}

const ok = (overrides: Record<string, unknown>) => ({
  status: 200,
  body: samplePaymentIntent(overrides),
});

describe("waitForPaymentIntent transitions", () => {
  it("polls through processing and resolves on succeeded", async () => {
    const { stub, stripe } = await stripeWalking([
      ok({ status: "processing" }),
      ok({ status: "processing" }),
      ok({ status: "succeeded" }),
    ]);

    const result = await stripe.waitForPaymentIntent(SECRET, {
      intervalMs: 5,
      timeoutMs: 5_000,
    });

    expect(result.error).toBeUndefined();
    expect(result.paymentIntent?.status).toBe("succeeded");
    expect(stub.requests).toHaveLength(3);
  });

  it("resolves on canceled", async () => {
    const { stripe } = await stripeWalking([
      ok({ status: "processing" }),
      ok({ status: "canceled" }),
    ]);
    const result = await stripe.waitForPaymentIntent(SECRET, {
      intervalMs: 5,
      timeoutMs: 5_000,
    });
    expect(result.paymentIntent?.status).toBe("canceled");
  });

  it("resolves on requires_payment_method once last_payment_error is populated", async () => {
    // There is no `failed` status: a rail refusal returns the intent to
    // `requires_payment_method` and fills `last_payment_error`.
    const { stub, stripe } = await stripeWalking([
      ok({ status: "processing" }),
      ok({
        status: "requires_payment_method",
        last_payment_error: {
          code: "insufficient_funds",
          message: "The payer's account has insufficient funds.",
        },
      }),
    ]);

    const result = await stripe.waitForPaymentIntent(SECRET, {
      intervalMs: 5,
      timeoutMs: 5_000,
    });

    expect(result.paymentIntent?.last_payment_error?.code).toBe(
      "insufficient_funds",
    );
    expect(stub.requests).toHaveLength(2);
  });

  it("keeps polling through an unconfirmed requires_payment_method, which has no error", async () => {
    // The status of an intent nobody has confirmed. Treating it as final
    // would make this method return the instant it was called.
    const { stub, stripe } = await stripeWalking([
      ok({ status: "requires_payment_method" }),
      ok({ status: "requires_payment_method" }),
      ok({ status: "succeeded" }),
    ]);

    const result = await stripe.waitForPaymentIntent(SECRET, {
      intervalMs: 5,
      timeoutMs: 5_000,
    });

    expect(result.paymentIntent?.status).toBe("succeeded");
    expect(stub.requests).toHaveLength(3);
  });

  it("returns the first error it meets rather than retrying it to the deadline", async () => {
    const { stub, stripe } = await stripeWalking([
      ok({ status: "processing" }),
      { status: 404, body: notFoundEnvelope("pi_123") },
    ]);

    const result = await stripe.waitForPaymentIntent(SECRET, {
      intervalMs: 5,
      timeoutMs: 5_000,
    });

    expect(result.error?.code).toBe("resource_missing");
    expect(stub.requests).toHaveLength(2);
  });

  it("refuses a malformed clientSecret without polling at all", async () => {
    const { stub, stripe } = await stripeWalking([ok({ status: "succeeded" })]);
    const result = await stripe.waitForPaymentIntent("nope", { intervalMs: 5 });
    expect(result.error?.param).toBe("clientSecret");
    expect(stub.requests).toHaveLength(0);
  });

  it.each([
    ["timeoutMs", { timeoutMs: -1 }],
    ["timeoutMs", { timeoutMs: Number.NaN }],
    ["intervalMs", { intervalMs: 0 }],
    ["intervalMs", { intervalMs: Number.POSITIVE_INFINITY }],
  ])("refuses an unusable %s", async (param, options) => {
    const { stub, stripe } = await stripeWalking([ok({ status: "succeeded" })]);
    const result = await stripe.waitForPaymentIntent(SECRET, options);
    expect(result.error?.type).toBe("invalid_request_error");
    expect(result.error?.param).toBe(param);
    expect(stub.requests).toHaveLength(0);
  });
});

describe("waitForPaymentIntent timing", () => {
  /** A `fetch` that answers from memory and records the fake clock at each call. */
  function cannedFetch(body: unknown, at: number[]): { fetch: typeof fetch } {
    return {
      fetch: () => {
        at.push(Date.now());
        return Promise.resolve(
          new Response(JSON.stringify(body), {
            status: 200,
            headers: { "Content-Type": "application/json" },
          }),
        );
      },
    };
  }

  it("polls on the requested interval and gives up at the deadline", async () => {
    vi.useFakeTimers();
    vi.spyOn(Math, "random").mockReturnValue(0.5); // no jitter: exactly intervalMs
    const at: number[] = [];
    const stripe = await loadStripe(PK, {
      baseUrl: "https://api.example",
      ...cannedFetch(samplePaymentIntent({ status: "processing" }), at),
    });
    const started = Date.now();

    const promise = stripe.waitForPaymentIntent(SECRET, {
      intervalMs: 2_000,
      timeoutMs: 6_000,
    });
    await vi.advanceTimersByTimeAsync(10_000);
    const result = await promise;

    expect(result.paymentIntent).toBeUndefined();
    expect(result.error).toEqual({
      type: "api_error",
      code: "polling_timeout",
      message:
        "Timed out waiting for the payment intent to reach a final state.",
    });
    // t=0, 2000, 4000, 6000 — then `remaining <= 0` ends it. Four polls, and
    // the clock never runs past the budget.
    expect(at.map((t) => t - started)).toEqual([0, 2_000, 4_000, 6_000]);
  });

  it("defaults to a three-minute budget polled every two seconds", async () => {
    vi.useFakeTimers();
    vi.spyOn(Math, "random").mockReturnValue(0.5);
    const at: number[] = [];
    const stripe = await loadStripe(PK, {
      baseUrl: "https://api.example",
      ...cannedFetch(samplePaymentIntent({ status: "processing" }), at),
    });
    const started = Date.now();

    const promise = stripe.waitForPaymentIntent(SECRET);
    await vi.advanceTimersByTimeAsync(200_000);
    const result = await promise;

    expect(result.error?.code).toBe("polling_timeout");
    expect(at.at(-1)! - started).toBe(180_000);
    expect(at).toHaveLength(91); // 180000 / 2000 + 1
  });

  it("clamps the last sleep to the remaining budget rather than overshooting it", async () => {
    vi.useFakeTimers();
    vi.spyOn(Math, "random").mockReturnValue(0.5);
    const at: number[] = [];
    const stripe = await loadStripe(PK, {
      baseUrl: "https://api.example",
      ...cannedFetch(samplePaymentIntent({ status: "processing" }), at),
    });
    const started = Date.now();

    const promise = stripe.waitForPaymentIntent(SECRET, {
      intervalMs: 2_000,
      timeoutMs: 3_000,
    });
    await vi.advanceTimersByTimeAsync(10_000);
    await promise;

    expect(at.map((t) => t - started)).toEqual([0, 2_000, 3_000]);
  });

  it.each([
    [0, 1_500],
    [0.5, 2_000],
    [0.999_999, 2_500],
  ])(
    "jitters the interval to ±25%% (Math.random %s → %s ms)",
    async (random, expected) => {
      vi.useFakeTimers();
      vi.spyOn(Math, "random").mockReturnValue(random);
      const at: number[] = [];
      const stripe = await loadStripe(PK, {
        baseUrl: "https://api.example",
        ...cannedFetch(samplePaymentIntent({ status: "processing" }), at),
      });
      const started = Date.now();

      const promise = stripe.waitForPaymentIntent(SECRET, {
        intervalMs: 2_000,
        timeoutMs: 60_000,
      });
      await vi.advanceTimersByTimeAsync(expected);
      void promise;

      expect(at.map((t) => t - started)).toEqual([0, expected]);
    },
  );
});
