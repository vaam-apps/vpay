/**
 * The state machine, driven through a real `@vaam-apps/vpay-stripe-js`
 * client over a scripted `fetch`.
 *
 * Deliberately not against a stubbed `Stripe` object: the one bug this
 * package could ship that no unit test of a stub would catch is reading
 * `session.client_secret` (typed `string`, never sent) instead of
 * `session.payment_intent.client_secret`, and only a real client building a
 * real URL out of it shows that.
 */
import { loadStripe, type Stripe } from "@vaam-apps/vpay-stripe-js";
import { describe, expect, it } from "vitest";

import {
  CheckoutController,
  type CheckoutPreflightReady,
} from "./controller.js";
import {
  BASE_URL,
  INTENT_ID,
  INTENT_SECRET,
  PUBLISHABLE_KEY,
  SESSION_ID,
  SESSION_SECRET,
  checkoutSessionBody,
  fakeClock,
  paymentIntentBody,
  scriptedFetch,
  type FetchScript,
  type ScriptedFetch,
} from "./testing/fixtures.js";

async function harness(
  script: FetchScript,
): Promise<{ controller: CheckoutController; http: ScriptedFetch }> {
  const http = scriptedFetch(script);
  const stripe: Stripe = await loadStripe(PUBLISHABLE_KEY, {
    baseUrl: BASE_URL,
    fetch: http.fetch,
  });
  return {
    controller: new CheckoutController({ stripe, clock: fakeClock() }),
    http,
  };
}

function readyFixture(): CheckoutPreflightReady {
  return {
    sessionId: SESSION_ID,
    paymentIntentId: INTENT_ID,
    intentClientSecret: INTENT_SECRET,
    successStopUrl: null,
    cancelStopUrl: null,
    stopUrls: [],
  };
}

describe("D1 — the outcome is never read off a URL", () => {
  it("reaching success_url with an intent that is processing yields pending, not succeeded", async () => {
    const { controller } = await harness({
      intents: [paymentIntentBody({ status: "processing" })],
    });

    const result = await controller.resolveAfterStopUrlReached(readyFixture(), {
      timeoutMs: 4_000,
      intervalMs: 2_000,
    });

    expect(result.kind).toBe("pending");
  });

  it("reaching success_url with an intent that has actually succeeded reports succeeded", async () => {
    const { controller } = await harness({
      intents: [paymentIntentBody({ status: "succeeded" })],
    });

    const result = await controller.resolveAfterStopUrlReached(readyFixture());

    expect(result.kind).toBe("succeeded");
  });

  it("resolveAfterStopUrlReached takes no URL argument at all — the signature itself is the proof", async () => {
    const { controller } = await harness({});

    // `ready` and an optional budget. There is nowhere to pass a reached
    // URL, which is the strongest form D1 can take on this side.
    expect(controller.resolveAfterStopUrlReached.length).toBe(1);
  });
});

describe("D4 — dismissal polls before it reports", () => {
  it("a dismissal with a succeeded intent reports succeeded, not canceled", async () => {
    const { controller } = await harness({
      intents: [paymentIntentBody({ status: "succeeded" })],
    });

    const result = await controller.resolveAfterDismissal(readyFixture());

    expect(result.kind).toBe("succeeded");
  });

  it("a dismissal with an intent still in flight after the short poll reports pending, not canceled", async () => {
    const { controller, http } = await harness({
      intents: [paymentIntentBody({ status: "processing" })],
    });

    const result = await controller.resolveAfterDismissal(readyFixture());

    expect(result.kind).toBe("pending");
    // The 5 s / 1 s budget really was spent rather than short-circuited:
    // six reads, the last of which found the deadline elapsed.
    expect(http.intentReads).toBe(6);
  });

  it("a dismissal with a genuinely canceled intent reports canceled", async () => {
    const { controller } = await harness({
      intents: [paymentIntentBody({ status: "canceled" })],
    });

    const result = await controller.resolveAfterDismissal(readyFixture());

    expect(result.kind).toBe("canceled");
  });

  it("a dismissal runs the same loop as a reached stop URL, only shorter", async () => {
    const processing = paymentIntentBody({ status: "processing" });
    const short = await harness({ intents: [processing] });
    const long = await harness({ intents: [processing] });

    await short.controller.resolveAfterDismissal(readyFixture());
    await long.controller.resolveAfterStopUrlReached(readyFixture());

    // 5 s / 1 s against 180 s / 2 s — same loop, same terminal rule, one
    // number different.
    expect(short.http.intentReads).toBe(6);
    expect(long.http.intentReads).toBe(91);
  });
});

describe("resolve — the answer must be about the intent that was asked for", () => {
  it("a succeeded intent with a different id is unresolved, never succeeded", async () => {
    const { controller } = await harness({
      intents: [
        paymentIntentBody({ id: "pi_someone_else", status: "succeeded" }),
      ],
    });

    const result = await controller.resolveAfterStopUrlReached(readyFixture());

    expect(result.kind).toBe("unresolved");
    if (result.kind !== "unresolved") {
      throw new Error("unreachable");
    }
    expect(result.error.code).toBe("unexpected_response");
    expect(result.paymentIntentId).toBe(INTENT_ID);
    // Neither id is interpolated: one of them came off the wire.
    expect(result.error.message).not.toContain("pi_someone_else");
  });
});

describe("resolve — failure mapping", () => {
  it("requires_payment_method with a last_payment_error reports failed with the rail code", async () => {
    const { controller } = await harness({
      intents: [
        paymentIntentBody({
          status: "requires_payment_method",
          last_payment_error: {
            code: "insufficient_funds",
            message: "Solde insuffisant",
          },
        }),
      ],
    });

    const result = await controller.resolveAfterStopUrlReached(readyFixture());

    expect(result).toEqual({
      kind: "failed",
      sessionId: SESSION_ID,
      paymentIntentId: INTENT_ID,
      code: "insufficient_funds",
      providerMessage: "Solde insuffisant",
    });
  });

  it("a bare requires_payment_method keeps polling rather than reporting failed", async () => {
    // There is no `failed` status in `vpay_core::state::IntentStatus`: a
    // rail failure returns the intent to `requires_payment_method` **with**
    // `last_payment_error`. Without one it is an intent nobody has
    // confirmed yet, and calling that a failure would be the plugin
    // deciding an outcome it did not observe.
    const { controller, http } = await harness({
      intents: [paymentIntentBody({ status: "requires_payment_method" })],
    });

    const result = await controller.resolveAfterDismissal(readyFixture());

    expect(result.kind).toBe("pending");
    expect(http.intentReads).toBe(6);
  });

  it("a poll that itself errors resolves unresolved with the typed error", async () => {
    const { controller } = await harness({ intents: [404] });

    const result = await controller.resolveAfterStopUrlReached(readyFixture());

    expect(result.kind).toBe("unresolved");
    if (result.kind !== "unresolved") {
      throw new Error("unreachable");
    }
    // The uniform 404, carried through as the server rendered it.
    expect(result.error.code).toBe("resource_missing");
  });

  it("an intent that settles on a later poll is reported from that poll, not the first", async () => {
    const { controller, http } = await harness({
      intents: [
        paymentIntentBody({ status: "processing" }),
        paymentIntentBody({ status: "processing" }),
        paymentIntentBody({ status: "succeeded" }),
      ],
    });

    const result = await controller.resolveAfterStopUrlReached(readyFixture());

    expect(result.kind).toBe("succeeded");
    expect(http.intentReads).toBe(3);
  });
});

describe("CheckoutController.preflight (D2)", () => {
  it("an embedded session is refused here, before any window opens, with a typed error", async () => {
    const { controller, http } = await harness({
      session: checkoutSessionBody({
        ui_mode: "embedded",
        success_url: null,
        cancel_url: null,
        url: null,
        return_url: "https://shop.example/return",
      }),
    });

    const preflight = await controller.preflight(SESSION_SECRET);

    expect(preflight.ok).toBe(false);
    if (preflight.ok) {
      throw new Error("unreachable");
    }
    expect(preflight.error.code).toBe("embedded_session_not_supported");
    // One read, and no intent read: nothing was polled and nothing shown.
    expect(http.sessionReads).toBe(1);
    expect(http.intentReads).toBe(0);
  });

  it("a hosted session buys the intent id, its client_secret, and derives the stop URLs from the session", async () => {
    const { controller } = await harness({});

    const preflight = await controller.preflight(SESSION_SECRET);

    expect(preflight.ok).toBe(true);
    if (!preflight.ok) {
      throw new Error("unreachable");
    }
    expect(preflight.ready.sessionId).toBe(SESSION_ID);
    expect(preflight.ready.paymentIntentId).toBe(INTENT_ID);
    expect(preflight.ready.stopUrls).toEqual([
      {
        scheme: "https",
        host: "shop.example",
        port: 443,
        path: `/return/${SESSION_ID}`,
      },
      { scheme: "https", host: "shop.example", port: 443, path: "/cart" },
    ]);
  });

  it("the polling credential is the intent's client_secret, never the session's", async () => {
    // `CheckoutSession.client_secret` is typed `string` by
    // `@vaam-apps/vpay-stripe-js` and the server never sends it back (found
    // by the Flutter e2e suite, 2026-09-14), which is why the fixture omits
    // it. Reading it would put `undefined` in every poll URL.
    const { controller, http } = await harness({});

    const preflight = await controller.preflight(SESSION_SECRET);
    if (!preflight.ok) {
      throw new Error("unreachable");
    }
    expect(preflight.ready.intentClientSecret).toBe(INTENT_SECRET);

    await controller.resolveAfterStopUrlReached(preflight.ready, {
      timeoutMs: 0,
    });
    const pollUrl = http.urls[http.urls.length - 1] ?? "";
    expect(pollUrl).toContain(`/v1/browser/payment_intents/${INTENT_ID}`);
    expect(pollUrl).toContain(encodeURIComponent(INTENT_SECRET));
    expect(pollUrl).not.toContain("undefined");
  });

  it("a bad or expired link — the uniform 404 — fails the pre-flight with the same typed error", async () => {
    const { controller } = await harness({ session: 404 });

    const preflight = await controller.preflight(SESSION_SECRET);

    expect(preflight.ok).toBe(false);
    if (preflight.ok) {
      throw new Error("unreachable");
    }
    expect(preflight.error.code).toBe("resource_missing");
  });

  it("a session with neither success_url nor cancel_url yields no stop rules at all", async () => {
    const { controller } = await harness({
      session: checkoutSessionBody({ success_url: null, cancel_url: null }),
    });

    const preflight = await controller.preflight(SESSION_SECRET);
    if (!preflight.ok) {
      throw new Error("unreachable");
    }
    expect(preflight.ready.stopUrls).toEqual([]);
  });
});
