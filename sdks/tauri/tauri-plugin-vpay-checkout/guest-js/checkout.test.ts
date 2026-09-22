/**
 * `VpayCheckout.start` end to end on Node: a scripted `fetch` for the two
 * reads, a fake clock for the poll, and a fake host standing in for the
 * window.
 */
import { describe, expect, it } from "vitest";

import { VpayCheckout } from "./checkout.js";
import {
  BASE_URL,
  PUBLISHABLE_KEY,
  SESSION_ID,
  SESSION_SECRET,
  SESSION_URL,
  checkoutSessionBody,
  fakeClock,
  fakeHost,
  paymentIntentBody,
  scriptedFetch,
  type FetchScript,
  type FakeHost,
  type FakeHostOptions,
} from "./testing/fixtures.js";
import type { VpayCheckoutOptions } from "./types.js";

function build(
  script: FetchScript,
  hostOptions: FakeHostOptions = {},
  overrides: Partial<VpayCheckoutOptions> = {},
): { checkout: VpayCheckout; host: FakeHost; sessionReads: () => number } {
  const http = scriptedFetch(script);
  const host = fakeHost(hostOptions);
  const checkout = new VpayCheckout({
    baseUrl: BASE_URL,
    publishableKey: PUBLISHABLE_KEY,
    fetch: http.fetch,
    host,
    clock: fakeClock(),
    ...overrides,
  });
  return { checkout, host, sessionReads: () => http.sessionReads };
}

/**
 * Waits until `start` has actually reached the host.
 *
 * A fixed number of microtask ticks would be a guess about how many
 * `await`s the pre-flight takes — through this package, through
 * `@vaam-apps/vpay-stripe-js`, and through `Response.json()` — and a guess
 * that is one short turns "the event arrived late" into "the event never
 * arrived".
 */
async function untilShown(host: FakeHost): Promise<void> {
  for (let tick = 0; tick < 500; tick += 1) {
    if (host.requests.length > 0) {
      return;
    }
    await new Promise<void>((resolve) => {
      setTimeout(resolve, 0);
    });
  }
  throw new Error("host.show was never called");
}

describe("VpayCheckout.start — the outcome comes from the poll", () => {
  it("a reached stop URL resolves through the poll, not off the URL", async () => {
    const { checkout, host } = build(
      { intents: [paymentIntentBody({ status: "succeeded" })] },
      { reportOnShow: "stopUrlReached" },
    );

    const result = await checkout.start(SESSION_URL);

    expect(result.kind).toBe("succeeded");
    // The host was handed a diagnostics URL it never had to look at: the
    // request it received carries no outcome at all.
    expect(host.requests).toHaveLength(1);
    expect(host.requests[0]?.url).toBe(SESSION_URL);
  });

  it("a dismissal with a still-processing intent resolves to pending", async () => {
    const { checkout } = build(
      { intents: [paymentIntentBody({ status: "processing" })] },
      { reportOnShow: "dismissed" },
    );

    const result = await checkout.start(SESSION_URL);

    expect(result.kind).toBe("pending");
  });

  it("a reached stop URL with a still-processing intent is pending, never succeeded", async () => {
    const { checkout } = build(
      { intents: [paymentIntentBody({ status: "processing" })] },
      { reportOnShow: "stopUrlReached" },
    );

    const result = await checkout.start(SESSION_URL);

    expect(result.kind).toBe("pending");
  });
});

describe("VpayCheckout.start — an embedded session is refused before any window opens", () => {
  it("never calls the host at all", async () => {
    const { checkout, host } = build({
      session: checkoutSessionBody({
        ui_mode: "embedded",
        success_url: null,
        cancel_url: null,
        url: null,
      }),
    });

    const result = await checkout.start(SESSION_URL);

    expect(result.kind).toBe("unresolved");
    if (result.kind !== "unresolved") {
      throw new Error("unreachable");
    }
    expect(result.error.code).toBe("embedded_session_not_supported");
    expect(result.sessionId).toBe(SESSION_ID);
    expect(host.requests).toHaveLength(0);
  });
});

describe("VpayCheckout.start — the window event cannot be lost", () => {
  it("an outcome reported from inside show(), before it resolves, is still received", async () => {
    // The whole reason `onEvent` is wired before `show` is awaited. The web
    // host returns with a popup already open, so a payer who closes it
    // instantly reports into a callback that must already exist.
    const { checkout } = build(
      { intents: [paymentIntentBody({ status: "canceled" })] },
      { reportOnShow: "dismissed" },
    );

    const result = await checkout.start(SESSION_URL);

    expect(result.kind).toBe("canceled");
  });

  it("an outcome reported after show() resolves is still received", async () => {
    const { checkout, host } = build({
      intents: [paymentIntentBody({ status: "succeeded" })],
    });

    const started = checkout.start(SESSION_URL);
    // Let the pre-flight and `show` run before the window reports.
    await untilShown(host);
    expect(
      host.report("stopUrlReached", "https://shop.example/return/cs_123"),
    ).toBe(true);

    expect((await started).kind).toBe("succeeded");
  });

  it("a host that reports twice drives exactly one poll and answers once", async () => {
    const http = scriptedFetch({
      intents: [paymentIntentBody({ status: "succeeded" })],
    });
    const host = fakeHost();
    const checkout = new VpayCheckout({
      baseUrl: BASE_URL,
      publishableKey: PUBLISHABLE_KEY,
      fetch: http.fetch,
      host,
      clock: fakeClock(),
    });

    const started = checkout.start(SESSION_URL);
    await untilShown(host);
    host.report("stopUrlReached");
    host.report("dismissed");

    expect((await started).kind).toBe("succeeded");
    expect(http.intentReads).toBe(1);
  });
});

describe("VpayCheckout.start — a window that will not open", () => {
  it("resolves unresolved with a fixed message that never quotes the thrown value", async () => {
    // The thrown value is the shape a real host produces: a message that
    // quotes the URL it failed on, and that URL's fragment is the session
    // secret (D6).
    const { checkout } = build(
      {},
      { rejectWith: new Error("could not open https://x/#cs_secret_LEAK") },
    );

    const result = await checkout.start(SESSION_URL);

    expect(result.kind).toBe("unresolved");
    if (result.kind !== "unresolved") {
      throw new Error("unreachable");
    }
    expect(result.error.code).toBe("platform_window_failed");
    expect(result.error.message).not.toContain("LEAK");
    expect(JSON.stringify(result)).not.toContain("LEAK");
    expect(JSON.stringify(result)).not.toContain(SESSION_SECRET);
  });
});

describe("VpayCheckout — the insecure-base opt-in", () => {
  it("allowInsecureUrl mirrors allowInsecureBaseUrl instead of being hard-coded false", async () => {
    const { checkout, host } = build(
      { intents: [paymentIntentBody({ status: "canceled" })] },
      { reportOnShow: "dismissed" },
      { baseUrl: "http://localhost:8081", allowInsecureBaseUrl: true },
    );

    await checkout.start(SESSION_URL);

    expect(host.requests[0]?.allowInsecureUrl).toBe(true);
  });

  it("and stays false for an https base, which is every non-demo deployment", async () => {
    const { checkout, host } = build(
      { intents: [paymentIntentBody({ status: "canceled" })] },
      { reportOnShow: "dismissed" },
    );

    await checkout.start(SESSION_URL);

    expect(host.requests[0]?.allowInsecureUrl).toBe(false);
  });

  it("a non-https baseUrl is refused by the constructor unless the opt-in is named", () => {
    // D6, and the one thing in this package that throws: an integration
    // mistake visible on the merchant's first page load, not a payer-facing
    // failure. `loadStripe` itself accepts `http://` happily, which is why
    // this check has to happen before it.
    expect(
      () =>
        new VpayCheckout({
          baseUrl: "http://localhost:8081",
          publishableKey: PUBLISHABLE_KEY,
          host: fakeHost(),
        }),
    ).toThrow(TypeError);
  });

  it("a blank publishable key or base URL is refused by the constructor", () => {
    expect(
      () =>
        new VpayCheckout({
          baseUrl: BASE_URL,
          publishableKey: "  ",
          host: fakeHost(),
        }),
    ).toThrow(TypeError);
    expect(
      () =>
        new VpayCheckout({
          baseUrl: "",
          publishableKey: PUBLISHABLE_KEY,
          host: fakeHost(),
        }),
    ).toThrow(TypeError);
  });
});

describe("VpayCheckout.start — a malformed sessionUrl", () => {
  it("is refused without any network call", async () => {
    const { checkout, host, sessionReads } = build({});

    const result = await checkout.start(
      `https://checkout.example/c/${SESSION_ID}?key=${PUBLISHABLE_KEY}`,
    );

    expect(result.kind).toBe("unresolved");
    if (result.kind !== "unresolved") {
      throw new Error("unreachable");
    }
    expect(result.error.code).toBe("invalid_request");
    expect(sessionReads()).toBe(0);
    expect(host.requests).toHaveLength(0);
  });

  it("an empty fragment is refused too, and the URL is never echoed back", async () => {
    const { checkout } = build({});

    const result = await checkout.start("https://checkout.example/c/cs_123#");

    expect(result.kind).toBe("unresolved");
    if (result.kind !== "unresolved") {
      throw new Error("unreachable");
    }
    expect(result.error.message).not.toContain("checkout.example");
  });
});

describe("VpayCheckout.dismiss", () => {
  it("closes the window through the host without inventing a result", async () => {
    const { checkout, host } = build({});

    await checkout.dismiss();

    expect(host.dismissals).toBe(1);
  });
});
