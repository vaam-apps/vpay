/**
 * `confirmPayment`'s redirect contract, against the same `node:http` stub
 * the rest of the suite uses (see `src/testing/browser-stub.ts` for why an
 * SDK's own test server is not a test double in ADR-0006's sense).
 *
 * The load-bearing assertion is the negative one: when the browser is being
 * navigated away, the returned promise **must not settle**. Stripe.js
 * behaves this way so that a caller's `.then`/`.finally` cannot run a
 * "payment failed" branch during the unload, and a payer cannot be shown a
 * failure for a payment that is on its way to succeeding.
 *
 * A test that merely asserted `window.location.assign` was called would pass
 * for an implementation that also resolved. `settled` is what makes the
 * difference observable — `pnpm --filter @vaam-apps/vpay-stripe-js test` fails if
 * `#followRedirect` is changed to `return Promise.resolve(result)` after
 * navigating.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadStripe } from "./index.js";
import {
  json,
  samplePaymentIntent,
  startBrowserStub,
  type BrowserStub,
} from "./testing/browser-stub.js";
import type { Stripe } from "./types.js";

const PK = "pk_test_abcdefghijklmnop";
const SECRET = `pi_123_secret_${"a".repeat(32)}`;
const RAIL_URL = "https://webpayment.orange.example/pay/tok_abc";

const stubs: BrowserStub[] = [];

/** An intent a redirect rail has answered with. */
function redirectingIntent(url: string = RAIL_URL): Record<string, unknown> {
  return samplePaymentIntent({
    status: "requires_action",
    payment_method_types: ["orange_money"],
    next_action: {
      type: "redirect_to_url",
      redirect_to_url: {
        url,
        // D3: vpay appends nothing to this — no `redirect_status`, no
        // `payment_intent_client_secret`. It is echoed back as a label.
        return_url: "https://shop.example/thanks",
      },
    },
  });
}

async function stripeReturning(body: Record<string, unknown>): Promise<Stripe> {
  const stub = await startBrowserStub((_req, res) => json(res, 200, body));
  stubs.push(stub);
  return loadStripe(PK, { baseUrl: stub.url });
}

/** Installs a minimal `window.location` and returns the `assign` spy. */
function installWindow(): ReturnType<typeof vi.fn> {
  const assign = vi.fn();
  (globalThis as { window?: unknown }).window = { location: { assign } };
  return assign;
}

/** True once `promise` has settled either way. Never awaits it. */
function watchSettlement(promise: Promise<unknown>): { settled: boolean } {
  const state = { settled: false };
  void promise.then(
    () => {
      state.settled = true;
    },
    () => {
      state.settled = true;
    },
  );
  return state;
}

/** Lets the event loop run long enough for a settled promise to have settled. */
async function letTheEventLoopRun(): Promise<void> {
  for (let i = 0; i < 5; i += 1) {
    await new Promise<void>((resolve) => setTimeout(resolve, 5));
  }
}

afterEach(async () => {
  delete (globalThis as { window?: unknown }).window;
  vi.restoreAllMocks();
  await Promise.all(stubs.splice(0).map((stub) => stub.close()));
});

describe("confirmPayment redirect semantics", () => {
  it("navigates and never settles when the rail asks for a redirect", async () => {
    const assign = installWindow();
    const stripe = await stripeReturning(redirectingIntent());

    const promise = stripe.confirmPayment({
      clientSecret: SECRET,
      confirmParams: {
        payment_method_data: { type: "orange_money" },
        return_url: "https://shop.example/thanks",
      },
    });
    const state = watchSettlement(promise);
    await letTheEventLoopRun();

    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith(RAIL_URL);
    // The mutation proof: flip `#followRedirect` to resolve after
    // `location.assign(url)` and this line fails.
    expect(state.settled).toBe(false);
  });

  it("navigates under an explicit redirect: 'always' too", async () => {
    const assign = installWindow();
    const stripe = await stripeReturning(redirectingIntent());

    const state = watchSettlement(
      stripe.confirmPayment({ clientSecret: SECRET, redirect: "always" }),
    );
    await letTheEventLoopRun();

    expect(assign).toHaveBeenCalledWith(RAIL_URL);
    expect(state.settled).toBe(false);
  });

  it("resolves with the intent, and does not navigate, under redirect: 'if_required'", async () => {
    const assign = installWindow();
    const stripe = await stripeReturning(redirectingIntent());

    const result = await stripe.confirmPayment({
      clientSecret: SECRET,
      redirect: "if_required",
    });

    expect(assign).not.toHaveBeenCalled();
    expect(result.error).toBeUndefined();
    expect(result.paymentIntent?.next_action?.redirect_to_url.url).toBe(
      RAIL_URL,
    );
  });

  it("resolves normally on a push rail, where there is no next_action", async () => {
    const assign = installWindow();
    const stripe = await stripeReturning(
      samplePaymentIntent({ status: "processing" }),
    );

    const result = await stripe.confirmPayment({ clientSecret: SECRET });

    expect(assign).not.toHaveBeenCalled();
    expect(result.paymentIntent?.status).toBe("processing");
  });

  it("does not navigate to an empty URL", async () => {
    const assign = installWindow();
    const stripe = await stripeReturning(redirectingIntent(""));

    const result = await stripe.confirmPayment({ clientSecret: SECRET });

    expect(assign).not.toHaveBeenCalled();
    expect(result.paymentIntent?.status).toBe("requires_action");
  });

  it("answers redirect_unavailable where there is no window, rather than inventing a resolution", async () => {
    // No `installWindow()` here: this is Node, SSR, or a worker.
    const stripe = await stripeReturning(redirectingIntent());

    const result = await stripe.confirmPayment({ clientSecret: SECRET });

    expect(result.paymentIntent).toBeUndefined();
    expect(result.error?.type).toBe("api_error");
    expect(result.error?.code).toBe("redirect_unavailable");
  });

  it("refuses a javascript: URL rather than navigating to it", async () => {
    const assign = installWindow();
    const stripe = await stripeReturning(
      redirectingIntent("javascript:alert(1)"),
    );

    const result = await stripe.confirmPayment({ clientSecret: SECRET });

    expect(assign).not.toHaveBeenCalled();
    expect(result.paymentIntent).toBeUndefined();
    expect(result.error?.type).toBe("api_error");
    expect(result.error?.code).toBe("invalid_redirect");
  });

  it("refuses a relative path rather than navigating to it", async () => {
    const assign = installWindow();
    const stripe = await stripeReturning(redirectingIntent("/pay/tok_abc"));

    const result = await stripe.confirmPayment({ clientSecret: SECRET });

    expect(assign).not.toHaveBeenCalled();
    expect(result.paymentIntent).toBeUndefined();
    expect(result.error?.type).toBe("api_error");
    expect(result.error?.code).toBe("invalid_redirect");
  });

  it("still navigates to an ordinary https URL", async () => {
    const assign = installWindow();
    const stripe = await stripeReturning(redirectingIntent());

    const state = watchSettlement(
      stripe.confirmPayment({ clientSecret: SECRET }),
    );
    await letTheEventLoopRun();

    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith(RAIL_URL);
    expect(state.settled).toBe(false);
  });

  it("does not navigate when the confirm itself failed", async () => {
    const assign = installWindow();
    const stub = await startBrowserStub((_req, res) =>
      json(res, 409, {
        error: {
          type: "invalid_request_error",
          code: "invalid_state",
          message: "This payment intent has already been confirmed.",
        },
      }),
    );
    stubs.push(stub);
    const stripe = await loadStripe(PK, { baseUrl: stub.url });

    const result = await stripe.confirmPayment({ clientSecret: SECRET });

    expect(assign).not.toHaveBeenCalled();
    expect(result.error?.code).toBe("invalid_state");
  });
});

describe("handleNextAction", () => {
  it("navigates for an intent already in requires_action, and never settles", async () => {
    const assign = installWindow();
    const stripe = await stripeReturning(redirectingIntent());

    const state = watchSettlement(
      stripe.handleNextAction({ clientSecret: SECRET }),
    );
    await letTheEventLoopRun();

    expect(assign).toHaveBeenCalledWith(RAIL_URL);
    expect(state.settled).toBe(false);
  });

  it("resolves when there is nothing to act on", async () => {
    installWindow();
    const stripe = await stripeReturning(
      samplePaymentIntent({ status: "processing" }),
    );
    const result = await stripe.handleNextAction({ clientSecret: SECRET });
    expect(result.paymentIntent?.status).toBe("processing");
  });
});
