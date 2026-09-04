// @vitest-environment jsdom
/**
 * `initEmbeddedCheckout` against a **real** `iframe` in a real DOM.
 *
 * The rest of this package's suite runs on Node against a `node:http` stub,
 * because its contract is bytes on the wire. This file's contract is a DOM
 * and an event, so it runs under jsdom and drives the actual element: the
 * `src` attribute the frame is given, the `message` events the handle does
 * and does not act on, and the listener `destroy()` must remove.
 *
 * The decisive assertion is the negative one. `wrongOrigin` posts a
 * perfectly well-formed `vpay:redirect` from a host that is not the
 * checkout app, and nothing happens. Delete the `event.origin` comparison
 * in `src/embedded.ts` and that test fails — which is the only reason to
 * trust the positive ones, since every message here is synthetic and a
 * handler that accepted everything would pass all of them.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { loadStripe } from "./index.js";
import type { EmbeddedCheckout, Stripe } from "./types.js";

const PK = "pk_test_abcdefghijklmnop";
const SUFFIX = "a".repeat(32);
const SESSION_SECRET = `cs_123_secret_${SUFFIX}`;
const API_URL = "https://api.vpay.example";
const CHECKOUT_URL = "https://checkout.vpay.example";
const CHECKOUT_ORIGIN = "https://checkout.vpay.example";

/** A `fetch` that fails the test if this package ever calls it here. */
const noFetch = (() =>
  Promise.reject(
    new Error("initEmbeddedCheckout must not make a request"),
  )) as unknown as typeof fetch;

const originalTop = Object.getOwnPropertyDescriptor(window, "top");

async function stripeWithCheckout(
  checkoutBaseUrl: string | undefined = CHECKOUT_URL,
): Promise<Stripe> {
  return loadStripe(PK, {
    baseUrl: API_URL,
    ...(checkoutBaseUrl === undefined ? {} : { checkoutBaseUrl }),
    fetch: noFetch,
  });
}

/** An embedded checkout mounted into a fresh `<div id="checkout">`. */
async function mounted(
  onComplete?: (event: { session: string; status: string }) => void,
): Promise<{ handle: EmbeddedCheckout; frame: HTMLIFrameElement }> {
  const stripe = await stripeWithCheckout();
  const handle = await stripe.initEmbeddedCheckout({
    fetchClientSecret: () => Promise.resolve(SESSION_SECRET),
    ...(onComplete === undefined ? {} : { onComplete }),
  });
  handle.mount("#checkout");
  const frame = document.querySelector("#checkout iframe");
  if (!(frame instanceof HTMLIFrameElement)) {
    throw new Error("mount() did not attach an iframe");
  }
  return { handle, frame };
}

/**
 * Posts a message at the parent as the framed page would.
 *
 * `source` is the frame's own `contentWindow`, because that is what a real
 * `postMessage` from inside it would carry — and the handle checks it, so a
 * test that omitted it would be asserting against a code path no browser
 * takes.
 */
function post(
  frame: HTMLIFrameElement,
  data: unknown,
  origin: string = CHECKOUT_ORIGIN,
): void {
  window.dispatchEvent(
    new MessageEvent("message", {
      origin,
      data,
      source: frame.contentWindow,
    }),
  );
}

beforeEach(() => {
  document.body.innerHTML = '<div id="checkout"></div>';
});

afterEach(() => {
  if (originalTop !== undefined) {
    Object.defineProperty(window, "top", originalTop);
  }
  document.body.innerHTML = "";
});

/** Replaces `window.top` with a stub whose `location.assign` is a spy. */
function installTop(): ReturnType<typeof vi.fn> {
  const assign = vi.fn();
  Object.defineProperty(window, "top", {
    configurable: true,
    get: () => ({ location: { assign } }),
  });
  return assign;
}

describe("initEmbeddedCheckout", () => {
  it("mounts an iframe whose src carries the session id, the key in the query and the secret in the fragment", async () => {
    const { frame } = await mounted();

    // The whole of D6 in one string: the key is public and in the query
    // (the page's own server reads it before any script runs), the secret
    // is after the `#` and is therefore never sent to any server.
    expect(frame.getAttribute("src")).toBe(
      `${CHECKOUT_URL}/e/cs_123?key=${PK}#${SESSION_SECRET}`,
    );
    expect(new URL(frame.src).search).not.toContain(SUFFIX);
  });

  it("sandboxes the frame without allow-top-navigation, so only the parent can navigate", async () => {
    const { frame } = await mounted();

    const sandbox = frame.getAttribute("sandbox") ?? "";
    expect(sandbox.split(" ")).toEqual([
      "allow-scripts",
      "allow-same-origin",
      "allow-forms",
    ]);
    expect(sandbox).not.toContain("allow-top-navigation");
    expect(frame.getAttribute("title")).toBe("Checkout");
  });

  it("sets the frame height from a vpay:resize message", async () => {
    const { frame } = await mounted();
    expect(frame.style.height).toBe("0px");

    post(frame, { type: "vpay:resize", height: 512 });

    expect(frame.style.height).toBe("512px");
  });

  it("ignores a message from an origin that is not the checkout app", async () => {
    // The decisive test. A well-formed message, the right `source`, every
    // field correct — and the wrong origin, which is the only thing
    // standing between a merchant's page and an arbitrary framed document
    // driving its top-level navigation.
    const assign = installTop();
    const { frame } = await mounted();

    post(frame, { type: "vpay:resize", height: 999 }, "https://evil.example");
    post(
      frame,
      { type: "vpay:redirect", url: "https://evil.example/steal" },
      "https://evil.example",
    );

    expect(frame.style.height).toBe("0px");
    expect(assign).not.toHaveBeenCalled();
  });

  it("ignores a message that is not from this frame, even from the checkout origin", async () => {
    const { frame } = await mounted();

    window.dispatchEvent(
      new MessageEvent("message", {
        origin: CHECKOUT_ORIGIN,
        data: { type: "vpay:resize", height: 777 },
        source: window,
      }),
    );

    expect(frame.style.height).toBe("0px");
  });

  it("calls onComplete with the payload of a vpay:complete message", async () => {
    const onComplete = vi.fn();
    const { frame } = await mounted(onComplete);

    post(frame, {
      type: "vpay:complete",
      session: "cs_123",
      status: "complete",
    });

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(onComplete).toHaveBeenCalledWith({
      session: "cs_123",
      status: "complete",
    });
  });

  it("ignores a vpay:complete whose session or status is not a string", async () => {
    const onComplete = vi.fn();
    const { frame } = await mounted(onComplete);

    post(frame, { type: "vpay:complete", session: { id: "cs_123" } });
    post(frame, { type: "vpay:complete", session: "cs_123", status: 7 });

    // A callback fired with a half-understood payload is worse than one
    // that did not fire: the merchant would render an outcome from fields
    // it could not read.
    expect(onComplete).not.toHaveBeenCalled();
  });

  it("assigns window.top.location for a vpay:redirect message, with the exact URL", async () => {
    const assign = installTop();
    const { frame } = await mounted();
    const railUrl = "https://webpayment.orange.example/pay/tok_abc?lang=fr";

    post(frame, { type: "vpay:redirect", url: railUrl });

    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith(railUrl);
  });

  it("refuses a javascript: URL in a vpay:redirect rather than navigating to it", async () => {
    const assign = installTop();
    const { frame } = await mounted();

    post(frame, { type: "vpay:redirect", url: "javascript:alert(1)" });
    post(frame, { type: "vpay:redirect", url: "/relative/path" });
    post(frame, { type: "vpay:redirect", url: 42 });

    expect(assign).not.toHaveBeenCalled();
  });

  it("ignores a message whose type it does not know", async () => {
    const { frame } = await mounted();

    post(frame, { type: "vpay:something-later" });
    post(frame, "a string, not an object");
    post(frame, null);

    expect(frame.style.height).toBe("0px");
  });

  it("unmount() detaches the frame and mount() re-attaches the same one", async () => {
    const { handle, frame } = await mounted();

    handle.unmount();
    expect(document.querySelector("#checkout iframe")).toBeNull();

    handle.mount("#checkout");
    expect(document.querySelector("#checkout iframe")).toBe(frame);
  });

  it("destroy() removes the message listener", async () => {
    const assign = installTop();
    const { handle, frame } = await mounted();

    handle.destroy();
    post(frame, { type: "vpay:resize", height: 640 });
    post(frame, { type: "vpay:redirect", url: "https://rail.example/pay" });

    expect(frame.style.height).toBe("0px");
    expect(assign).not.toHaveBeenCalled();
    expect(document.querySelector("#checkout iframe")).toBeNull();
    expect(() => handle.mount("#checkout")).toThrow(TypeError);
  });

  it("refuses a second mount() while already mounted", async () => {
    const { handle } = await mounted();

    expect(() => handle.mount("#checkout")).toThrow(TypeError);
  });

  it("refuses a selector that matches nothing", async () => {
    const stripe = await stripeWithCheckout();
    const handle = await stripe.initEmbeddedCheckout({
      fetchClientSecret: () => Promise.resolve(SESSION_SECRET),
    });

    expect(() => handle.mount("#not-on-this-page")).toThrow(TypeError);
  });

  it("mounts into an element as well as a selector", async () => {
    const stripe = await stripeWithCheckout();
    const handle = await stripe.initEmbeddedCheckout({
      fetchClientSecret: () => Promise.resolve(SESSION_SECRET),
    });
    const target = document.createElement("section");
    document.body.appendChild(target);

    handle.mount(target);

    expect(target.querySelector("iframe")).not.toBeNull();
  });

  it("rejects when loadStripe was given no checkoutBaseUrl", async () => {
    // Explicitly *not* through `stripeWithCheckout`, whose default
    // parameter would fill one in.
    const stripe = await loadStripe(PK, { baseUrl: API_URL, fetch: noFetch });

    await expect(
      stripe.initEmbeddedCheckout({
        fetchClientSecret: () => Promise.resolve(SESSION_SECRET),
      }),
    ).rejects.toThrow(TypeError);
  });

  it("rejects a checkoutBaseUrl that is not an absolute http(s) URL, at loadStripe", async () => {
    await expect(
      loadStripe(PK, {
        baseUrl: API_URL,
        checkoutBaseUrl: "/checkout",
        fetch: noFetch,
      }),
    ).rejects.toThrow(TypeError);
  });

  it("rejects when fetchClientSecret does not return a checkout-session secret", async () => {
    const stripe = await stripeWithCheckout();

    await expect(
      stripe.initEmbeddedCheckout({
        fetchClientSecret: () => Promise.resolve(`pi_123_secret_${SUFFIX}`),
      }),
    ).rejects.toThrow(TypeError);
    // The rejected credential is never quoted into the message.
    await stripe
      .initEmbeddedCheckout({
        fetchClientSecret: () => Promise.resolve(`pi_123_secret_${SUFFIX}`),
      })
      .catch((err: unknown) => {
        expect(String(err)).not.toContain(SUFFIX);
      });
  });

  it("propagates a rejection from fetchClientSecret unchanged", async () => {
    const stripe = await stripeWithCheckout();
    const failure = new Error("the merchant's own /create-session route 500ed");

    await expect(
      stripe.initEmbeddedCheckout({
        fetchClientSecret: () => Promise.reject(failure),
      }),
    ).rejects.toBe(failure);
  });

  it("strips trailing slashes from checkoutBaseUrl so the frame src cannot become //e", async () => {
    const stripe = await loadStripe(PK, {
      baseUrl: API_URL,
      checkoutBaseUrl: `${CHECKOUT_URL}///`,
      fetch: noFetch,
    });
    const handle = await stripe.initEmbeddedCheckout({
      fetchClientSecret: () => Promise.resolve(SESSION_SECRET),
    });
    handle.mount("#checkout");

    const frame = document.querySelector("#checkout iframe");
    expect((frame as HTMLIFrameElement).getAttribute("src")).toBe(
      `${CHECKOUT_URL}/e/cs_123?key=${PK}#${SESSION_SECRET}`,
    );
  });
});
