/**
 * The popup surface: the window that is opened, when it is opened, what
 * message is believed, and what happens when the payer closes it.
 *
 * The windows here are **stubs**, not jsdom's: jsdom implements neither
 * `window.open` nor cross-window `postMessage`, so a suite built on it
 * would be asserting against "not implemented" rather than against this
 * package. `openCheckoutPopup` takes its window as a parameter for exactly
 * that reason, and `VpayStripe.openCheckoutPopup` is the one-line adapter
 * that supplies the global one.
 *
 * The decisive assertions are the negative ones — a `vpay:complete` from
 * the wrong origin, and one from a different window on the right origin,
 * both ignored. Delete either check in `src/popup.ts` and one of them
 * fails; without them the positive cases prove nothing, since every message
 * here is synthetic.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  CheckoutPopupBlockedError,
  notifyCheckoutOpener,
  openCheckoutPopup,
} from "./popup.js";
import { loadStripe } from "./index.js";

const SHOP_ORIGIN = "https://shop.example";
const HOSTED_URL = "https://checkout.vpay.example/c/cs_123#secret";

interface StubPopup {
  closed: boolean;
  assigned: string[];
  closeCalls: number;
  focusCalls: number;
  close(): void;
  focus(): void;
  location: { href: string; assign(url: string): void };
}

/**
 * A popup window whose `Location` behaves like a **cross-origin** one.
 *
 * That is not a detail of the stub, it is the case that matters: `win.open('',
 * name, …)` returns an existing window of that name *without navigating it*,
 * so the second `openCheckoutPopup` of a session is handed a window already
 * sitting on vpay's origin. On such a window an opener may set `location.href`
 * and call `location.replace()`, and `location.assign()` throws — so `assign`
 * here throws too, and every navigation assertion in this file reads the
 * `href` setter. Point `src/popup.ts` back at `assign` and the whole suite
 * fails rather than one case.
 */
function stubPopup(): StubPopup {
  const assigned: string[] = [];
  const popup: StubPopup = {
    closed: false,
    assigned,
    closeCalls: 0,
    focusCalls: 0,
    close() {
      popup.closeCalls += 1;
      popup.closed = true;
    },
    focus() {
      popup.focusCalls += 1;
    },
    location: {
      get href(): string {
        return assigned[assigned.length - 1] ?? "about:blank";
      },
      set href(url: string) {
        assigned.push(url);
      },
      assign(): void {
        throw new Error(
          "SecurityError: Location.assign is not accessible cross-origin",
        );
      },
    },
  };
  return popup;
}

interface StubOpener {
  win: Window;
  popup: StubPopup | null;
  openCalls: { url: string; name: string; features: string }[];
  listeners: number;
  post(data: unknown, origin: string, source: unknown): void;
}

/**
 * A window that records `open` calls and dispatches `message` events by hand.
 *
 * `open` hands back whatever `popup` holds *at call time*, so a test can
 * swap it to make a second `openCheckoutPopup` get a different window.
 */
function stubWindow(popup: StubPopup | null = stubPopup()): StubOpener {
  const handlers = new Set<(event: MessageEvent) => void>();
  const openCalls: { url: string; name: string; features: string }[] = [];
  const state = { popup };
  const win = {
    location: { origin: SHOP_ORIGIN },
    screenX: 0,
    screenY: 0,
    outerWidth: 1280,
    outerHeight: 800,
    open(url: string, name: string, features: string) {
      openCalls.push({ url, name, features });
      return state.popup;
    },
    addEventListener(type: string, handler: (event: MessageEvent) => void) {
      if (type === "message") {
        handlers.add(handler);
      }
    },
    removeEventListener(type: string, handler: (event: MessageEvent) => void) {
      if (type === "message") {
        handlers.delete(handler);
      }
    },
  } as unknown as Window;
  return {
    win,
    get popup() {
      return state.popup;
    },
    set popup(next: StubPopup | null) {
      state.popup = next;
    },
    openCalls,
    get listeners() {
      return handlers.size;
    },
    post(data: unknown, origin: string, source: unknown) {
      for (const handler of [...handlers]) {
        handler({ data, origin, source } as MessageEvent);
      }
    },
  };
}

describe("openCheckoutPopup", () => {
  it("opens the window before awaiting the merchant's server call", async () => {
    const opener = stubWindow();
    let openedBeforeFetch = false;
    await openCheckoutPopup({
      win: opener.win,
      options: {
        fetchCheckoutUrl: () => {
          openedBeforeFetch = opener.openCalls.length === 1;
          return Promise.resolve(HOSTED_URL);
        },
      },
    });
    expect(openedBeforeFetch).toBe(true);
    expect(opener.openCalls[0]?.url).toBe("");
    expect(opener.openCalls[0]?.name).toBe("vpay-checkout");
  });

  it("navigates the window to the hosted url the merchant's server returned", async () => {
    const opener = stubWindow();
    await openCheckoutPopup({
      win: opener.win,
      options: { fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL) },
    });
    expect(opener.popup?.assigned).toEqual([HOSTED_URL]);
  });

  it("navigates a window it is REUSING, which a cross-origin assign() could not", async () => {
    // The second click of a session. `win.open('', name, …)` hands back the
    // window the first call opened, still on vpay's origin and untouched by
    // the empty url — so this navigation crosses an origin boundary, and the
    // stub's `assign` throws exactly as a real one would.
    const popup = stubPopup();
    const opener = stubWindow(popup);
    await openCheckoutPopup({
      win: opener.win,
      options: { fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL) },
    });
    const second = "https://checkout.vpay.example/c/cs_456#secret";
    await openCheckoutPopup({
      win: opener.win,
      options: { fetchCheckoutUrl: () => Promise.resolve(second) },
    });
    expect(popup.assigned).toEqual([HOSTED_URL, second]);
    expect(popup.closed).toBe(false);
    expect(opener.openCalls.map((call) => call.name)).toEqual([
      "vpay-checkout",
      "vpay-checkout",
    ]);
  });

  it("keeps the address bar, so a payer can see whose page is asking", async () => {
    const opener = stubWindow();
    await openCheckoutPopup({
      win: opener.win,
      options: { fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL) },
    });
    expect(opener.openCalls[0]?.features).toContain("location=yes");
  });

  it("throws CheckoutPopupBlockedError when the browser refuses the window", async () => {
    const opener = stubWindow(null);
    await expect(
      openCheckoutPopup({
        win: opener.win,
        options: { fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL) },
      }),
    ).rejects.toBeInstanceOf(CheckoutPopupBlockedError);
  });

  it("closes the window and rethrows when the merchant's server call fails", async () => {
    const opener = stubWindow();
    const boom = new Error("the shop's server said no");
    await expect(
      openCheckoutPopup({
        win: opener.win,
        options: { fetchCheckoutUrl: () => Promise.reject(boom) },
      }),
    ).rejects.toBe(boom);
    expect(opener.popup?.closed).toBe(true);
  });

  it("refuses a checkout url that is not absolute http(s), and navigates nowhere", async () => {
    for (const url of [
      "javascript:alert(1)",
      "/orders/1",
      "data:text/html,x",
      "",
    ]) {
      const opener = stubWindow();
      await expect(
        openCheckoutPopup({
          win: opener.win,
          options: { fetchCheckoutUrl: () => Promise.resolve(url) },
        }),
      ).rejects.toBeInstanceOf(TypeError);
      expect(opener.popup?.assigned).toEqual([]);
      expect(opener.popup?.closed).toBe(true);
    }
  });

  it("reports a vpay:complete from the merchant's own return page", async () => {
    const opener = stubWindow();
    const seen: { session: string; status: string }[] = [];
    await openCheckoutPopup({
      win: opener.win,
      options: {
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
        onComplete: (event) => seen.push(event),
      },
    });
    opener.post(
      { type: "vpay:complete", session: "cs_123", status: "complete" },
      SHOP_ORIGIN,
      opener.popup,
    );
    expect(seen).toEqual([{ session: "cs_123", status: "complete" }]);
    expect(opener.popup?.closed).toBe(true);
  });

  it("ignores a vpay:complete from any other origin", async () => {
    const opener = stubWindow();
    const seen: unknown[] = [];
    await openCheckoutPopup({
      win: opener.win,
      options: {
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
        onComplete: (event) => seen.push(event),
      },
    });
    opener.post(
      { type: "vpay:complete", session: "cs_123", status: "complete" },
      "https://attacker.example",
      opener.popup,
    );
    expect(seen).toEqual([]);
    expect(opener.popup?.closed).toBe(false);
  });

  it("ignores a vpay:complete from a different window on the right origin", async () => {
    const opener = stubWindow();
    const seen: unknown[] = [];
    await openCheckoutPopup({
      win: opener.win,
      options: {
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
        onComplete: (event) => seen.push(event),
      },
    });
    opener.post(
      { type: "vpay:complete", session: "cs_123", status: "complete" },
      SHOP_ORIGIN,
      stubPopup(),
    );
    expect(seen).toEqual([]);
  });

  it("ignores a message whose session or status is not a string", async () => {
    const opener = stubWindow();
    const seen: unknown[] = [];
    await openCheckoutPopup({
      win: opener.win,
      options: {
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
        onComplete: (event) => seen.push(event),
      },
    });
    for (const data of [
      { type: "vpay:complete", session: 1, status: "complete" },
      { type: "vpay:complete", session: "cs_123" },
      { type: "vpay:resize", height: 10 },
      null,
      "vpay:complete",
    ]) {
      opener.post(data, SHOP_ORIGIN, opener.popup);
    }
    expect(seen).toEqual([]);
  });

  it("pins the origin the caller named rather than its own", async () => {
    const opener = stubWindow();
    const seen: unknown[] = [];
    await openCheckoutPopup({
      win: opener.win,
      options: {
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
        completionOrigin: "https://returns.example",
        onComplete: (event) => seen.push(event),
      },
    });
    opener.post(
      { type: "vpay:complete", session: "cs_9", status: "complete" },
      SHOP_ORIGIN,
      opener.popup,
    );
    expect(seen).toEqual([]);
    opener.post(
      { type: "vpay:complete", session: "cs_9", status: "complete" },
      "https://returns.example",
      opener.popup,
    );
    expect(seen).toEqual([{ session: "cs_9", status: "complete" }]);
  });

  it("drops its message listener once the checkout has completed", async () => {
    const opener = stubWindow();
    await openCheckoutPopup({
      win: opener.win,
      options: {
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
        onComplete: () => undefined,
      },
    });
    expect(opener.listeners).toBe(1);
    opener.post(
      { type: "vpay:complete", session: "cs_1", status: "complete" },
      SHOP_ORIGIN,
      opener.popup,
    );
    expect(opener.listeners).toBe(0);
  });

  it("destroy() drops the listener and leaves the window open", async () => {
    const opener = stubWindow();
    const handle = await openCheckoutPopup({
      win: opener.win,
      options: { fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL) },
    });
    handle.destroy();
    expect(opener.listeners).toBe(0);
    expect(opener.popup?.closed).toBe(false);
  });

  it("close() closes the window and focus() forwards to it", async () => {
    const opener = stubWindow();
    const handle = await openCheckoutPopup({
      win: opener.win,
      options: { fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL) },
    });
    handle.focus();
    expect(opener.popup?.focusCalls).toBe(1);
    expect(handle.closed).toBe(false);
    handle.close();
    expect(handle.closed).toBe(true);
  });
});

describe("openCheckoutPopup: the payer closing the window", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("calls onCancel when the window goes without a completion", async () => {
    const opener = stubWindow();
    let cancelled = 0;
    await openCheckoutPopup({
      win: opener.win,
      options: {
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
        onCancel: () => {
          cancelled += 1;
        },
      },
    });
    vi.advanceTimersByTime(2000);
    expect(cancelled).toBe(0);
    if (opener.popup !== null) {
      opener.popup.closed = true;
    }
    vi.advanceTimersByTime(1000);
    expect(cancelled).toBe(1);
    // …and only once: the poll is cleared, not merely guarded.
    vi.advanceTimersByTime(5000);
    expect(cancelled).toBe(1);
  });

  it("does not call onCancel for a close the merchant asked for", async () => {
    const opener = stubWindow();
    let cancelled = 0;
    const handle = await openCheckoutPopup({
      win: opener.win,
      options: {
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
        onCancel: () => {
          cancelled += 1;
        },
      },
    });
    handle.close();
    vi.advanceTimersByTime(5000);
    expect(cancelled).toBe(0);
  });

  it("does not call onCancel after a completion closed the window", async () => {
    const opener = stubWindow();
    let cancelled = 0;
    let completed = 0;
    await openCheckoutPopup({
      win: opener.win,
      options: {
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
        onComplete: () => {
          completed += 1;
        },
        onCancel: () => {
          cancelled += 1;
        },
      },
    });
    opener.post(
      { type: "vpay:complete", session: "cs_1", status: "complete" },
      SHOP_ORIGIN,
      opener.popup,
    );
    vi.advanceTimersByTime(5000);
    expect(completed).toBe(1);
    expect(cancelled).toBe(0);
  });

  it("starts no timer at all when the merchant passes no onCancel", async () => {
    const opener = stubWindow();
    const spy = vi.spyOn(globalThis, "setInterval");
    await openCheckoutPopup({
      win: opener.win,
      options: { fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL) },
    });
    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });
});

/**
 * Issue #65: vpay's page posts `vpay:complete` to its opener (the primary
 * channel) and the merchant's `success_url` page may post it too (the
 * fallback). Every message from vpay's page below is the exact shape the
 * checkout app's `CompleteMessage` declares
 * (`frontends/apps/checkout/src/lib/frame.ts`) and both its controllers post:
 * `{type: 'vpay:complete', session: <cs id>, status: <session status>}`.
 */
describe("openCheckoutPopup: vpay's page first, the return page as fallback", () => {
  const CHECKOUT_ORIGIN = "https://checkout.vpay.example";

  function fromVpay(session: string, status = "complete"): unknown {
    return { type: "vpay:complete", session, status };
  }

  async function open(
    opener: StubOpener,
    seen: { session: string; status: string }[],
    checkoutOrigin: string | null = CHECKOUT_ORIGIN,
  ) {
    return openCheckoutPopup({
      win: opener.win,
      checkoutOrigin: checkoutOrigin ?? undefined,
      options: {
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
        onComplete: (event) => seen.push(event),
      },
    });
  }

  it("completes on vpay's own message from the configured checkout origin and this window", async () => {
    const opener = stubWindow();
    const seen: { session: string; status: string }[] = [];
    await open(opener, seen);
    opener.post(fromVpay("cs_123"), CHECKOUT_ORIGIN, opener.popup);
    expect(seen).toEqual([{ session: "cs_123", status: "complete" }]);
    expect(opener.popup?.closed).toBe(true);
    expect(opener.listeners).toBe(0);
  });

  it("passes on the status vpay's page sent, including one that is not a success", async () => {
    const opener = stubWindow();
    const seen: { session: string; status: string }[] = [];
    await open(opener, seen);
    opener.post(fromVpay("cs_123", "expired"), CHECKOUT_ORIGIN, opener.popup);
    expect(seen).toEqual([{ session: "cs_123", status: "expired" }]);
  });

  it("fires once when vpay's page reports first and the return page follows", async () => {
    const opener = stubWindow();
    const seen: { session: string; status: string }[] = [];
    await open(opener, seen);
    opener.post(fromVpay("cs_123", "complete"), CHECKOUT_ORIGIN, opener.popup);
    opener.post(
      { type: "vpay:complete", session: "cs_123", status: "paid" },
      SHOP_ORIGIN,
      opener.popup,
    );
    expect(seen).toEqual([{ session: "cs_123", status: "complete" }]);
  });

  it("fires once when the return page reports first and vpay's page follows", async () => {
    const opener = stubWindow();
    const seen: { session: string; status: string }[] = [];
    await open(opener, seen);
    opener.post(
      { type: "vpay:complete", session: "cs_123", status: "paid" },
      SHOP_ORIGIN,
      opener.popup,
    );
    opener.post(fromVpay("cs_123", "complete"), CHECKOUT_ORIGIN, opener.popup);
    expect(seen).toEqual([{ session: "cs_123", status: "paid" }]);
  });

  it("drops a session already reported even on a later popup that reused the opener, and not a different one", async () => {
    // The first checkout completes through vpay's page. The merchant then
    // opens a second checkout from the same page, and the first session's
    // late fallback reaches the second call's listener from the second
    // call's own window — right origin, right source. It must not complete
    // the second checkout; the second session's own message must.
    const opener = stubWindow();
    const first: { session: string; status: string }[] = [];
    await open(opener, first);
    opener.post(fromVpay("cs_1"), CHECKOUT_ORIGIN, opener.popup);
    expect(first).toEqual([{ session: "cs_1", status: "complete" }]);

    opener.popup = stubPopup();
    const second: { session: string; status: string }[] = [];
    await open(opener, second);
    opener.post(
      { type: "vpay:complete", session: "cs_1", status: "complete" },
      SHOP_ORIGIN,
      opener.popup,
    );
    expect(second).toEqual([]);
    expect(opener.popup?.closed).toBe(false);
    expect(opener.listeners).toBe(1);

    opener.post(fromVpay("cs_2"), CHECKOUT_ORIGIN, opener.popup);
    expect(second).toEqual([{ session: "cs_2", status: "complete" }]);
    expect(first).toEqual([{ session: "cs_1", status: "complete" }]);
  });

  it("does not share its record of completed sessions between opener windows", async () => {
    const one = stubWindow();
    const two = stubWindow();
    const seenOne: { session: string; status: string }[] = [];
    const seenTwo: { session: string; status: string }[] = [];
    await open(one, seenOne);
    await open(two, seenTwo);
    one.post(fromVpay("cs_same"), CHECKOUT_ORIGIN, one.popup);
    two.post(fromVpay("cs_same"), CHECKOUT_ORIGIN, two.popup);
    expect(seenOne).toHaveLength(1);
    expect(seenTwo).toHaveLength(1);
  });

  it("ignores vpay's message from an origin that is neither the checkout's nor the shop's", async () => {
    const opener = stubWindow();
    const seen: { session: string; status: string }[] = [];
    await open(opener, seen);
    for (const origin of [
      "https://checkout.attacker.example",
      "http://checkout.vpay.example",
      "https://checkout.vpay.example:8443",
      "null",
    ]) {
      opener.post(fromVpay("cs_123"), origin, opener.popup);
    }
    expect(seen).toEqual([]);
    expect(opener.popup?.closed).toBe(false);
  });

  it("ignores vpay's message from another window on the checkout origin", async () => {
    // Another tab or popup on vpay's own origin — a second checkout, say —
    // is not this popup, and its completion is not this checkout's.
    const opener = stubWindow();
    const seen: { session: string; status: string }[] = [];
    await open(opener, seen);
    opener.post(fromVpay("cs_123"), CHECKOUT_ORIGIN, stubPopup());
    expect(seen).toEqual([]);
    // …and the record was not touched: this popup's own message still lands.
    opener.post(fromVpay("cs_123"), CHECKOUT_ORIGIN, opener.popup);
    expect(seen).toEqual([{ session: "cs_123", status: "complete" }]);
  });

  it("ignores vpay's message entirely when no checkout origin is configured, and still takes the fallback", async () => {
    const opener = stubWindow();
    const seen: { session: string; status: string }[] = [];
    await open(opener, seen, null);
    opener.post(fromVpay("cs_123"), CHECKOUT_ORIGIN, opener.popup);
    expect(seen).toEqual([]);
    opener.post(fromVpay("cs_123"), SHOP_ORIGIN, opener.popup);
    expect(seen).toEqual([{ session: "cs_123", status: "complete" }]);
  });

  it("ignores a malformed message from the checkout origin without spending the session", async () => {
    const opener = stubWindow();
    const seen: { session: string; status: string }[] = [];
    await open(opener, seen);
    for (const data of [
      { type: "vpay:complete", session: "", status: "complete" },
      { type: "vpay:complete", session: "cs_123" },
      { type: "vpay:complete", session: "cs_123", status: 1 },
      { type: "vpay:complete", status: "complete" },
      { type: "vpay:redirect", url: "https://rail.example" },
      { type: "vpay:resize", height: 10 },
      { session: "cs_123", status: "complete" },
      "vpay:complete",
      null,
      undefined,
    ]) {
      opener.post(data, CHECKOUT_ORIGIN, opener.popup);
    }
    expect(seen).toEqual([]);
    expect(opener.popup?.closed).toBe(false);
    opener.post(fromVpay("cs_123"), CHECKOUT_ORIGIN, opener.popup);
    expect(seen).toEqual([{ session: "cs_123", status: "complete" }]);
  });
});

describe("Stripe.openCheckoutPopup: the checkout origin comes from loadStripe", () => {
  afterEach(() => {
    delete (globalThis as { window?: unknown }).window;
  });

  it("pins vpay's message to checkoutBaseUrl's origin", async () => {
    const opener = stubWindow();
    (globalThis as { window?: unknown }).window = opener.win;
    const stripe = await loadStripe("pk_test_x", {
      baseUrl: "https://api.vpay.example",
      checkoutBaseUrl: "https://checkout.vpay.example/base/",
      fetch: () => Promise.reject(new Error("no request expected")),
    });
    const seen: { session: string; status: string }[] = [];
    await stripe.openCheckoutPopup({
      fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
      onComplete: (event) => seen.push(event),
    });
    opener.post(
      { type: "vpay:complete", session: "cs_123", status: "complete" },
      "https://elsewhere.vpay.example",
      opener.popup,
    );
    expect(seen).toEqual([]);
    opener.post(
      { type: "vpay:complete", session: "cs_123", status: "complete" },
      "https://checkout.vpay.example",
      opener.popup,
    );
    expect(seen).toEqual([{ session: "cs_123", status: "complete" }]);
  });

  it("leaves the primary channel off when loadStripe had no checkoutBaseUrl", async () => {
    const opener = stubWindow();
    (globalThis as { window?: unknown }).window = opener.win;
    const stripe = await loadStripe("pk_test_x", {
      baseUrl: "https://api.vpay.example",
      fetch: () => Promise.reject(new Error("no request expected")),
    });
    const seen: { session: string; status: string }[] = [];
    await stripe.openCheckoutPopup({
      fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
      onComplete: (event) => seen.push(event),
    });
    opener.post(
      { type: "vpay:complete", session: "cs_123", status: "complete" },
      "https://checkout.vpay.example",
      opener.popup,
    );
    expect(seen).toEqual([]);
  });
});

describe("notifyCheckoutOpener", () => {
  it("posts vpay:complete to the opener's origin and closes the window", () => {
    const posted: { data: unknown; origin: string }[] = [];
    let closed = false;
    const win = {
      location: { origin: SHOP_ORIGIN },
      opener: {
        postMessage: (data: unknown, origin: string) =>
          posted.push({ data, origin }),
      },
      close: () => {
        closed = true;
      },
    } as unknown as Window;
    expect(
      notifyCheckoutOpener({ session: "cs_7", status: "complete", win }),
    ).toBe(true);
    expect(posted).toEqual([
      {
        data: { type: "vpay:complete", session: "cs_7", status: "complete" },
        origin: SHOP_ORIGIN,
      },
    ]);
    expect(closed).toBe(true);
  });

  it("never posts to '*', even when the caller names another origin", () => {
    const posted: string[] = [];
    const win = {
      location: { origin: SHOP_ORIGIN },
      opener: {
        postMessage: (_data: unknown, origin: string) => posted.push(origin),
      },
      close: () => undefined,
    } as unknown as Window;
    notifyCheckoutOpener({
      session: "cs_7",
      status: "complete",
      targetOrigin: "https://other.example",
      win,
    });
    expect(posted).toEqual(["https://other.example"]);
    expect(posted).not.toContain("*");
  });

  it("does nothing and answers false when the page has no opener", () => {
    let closed = false;
    const win = {
      location: { origin: SHOP_ORIGIN },
      opener: null,
      close: () => {
        closed = true;
      },
    } as unknown as Window;
    expect(
      notifyCheckoutOpener({ session: "cs_7", status: "complete", win }),
    ).toBe(false);
    expect(closed).toBe(false);
  });

  it("leaves the window open when the caller asks it to", () => {
    let closed = false;
    const win = {
      location: { origin: SHOP_ORIGIN },
      opener: { postMessage: () => undefined },
      close: () => {
        closed = true;
      },
    } as unknown as Window;
    notifyCheckoutOpener({
      session: "cs_7",
      status: "complete",
      close: false,
      win,
    });
    expect(closed).toBe(false);
  });

  it("refuses to post without a session id", () => {
    const win = {
      location: { origin: SHOP_ORIGIN },
      opener: { postMessage: () => undefined },
      close: () => undefined,
    } as unknown as Window;
    expect(() =>
      notifyCheckoutOpener({ session: "", status: "complete", win }),
    ).toThrow(TypeError);
  });
});

describe("the round trip: a popup and the return page that closes it", () => {
  it("delivers the return page's message to the opener's onComplete", async () => {
    const opener = stubWindow();
    const seen: { session: string; status: string }[] = [];
    await openCheckoutPopup({
      win: opener.win,
      options: {
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
        onComplete: (event) => seen.push(event),
      },
    });
    // The return page, running *inside* the popup: its `opener` is the
    // shop's window, and its own origin is the shop's.
    const returnPage = {
      location: { origin: SHOP_ORIGIN },
      opener: {
        postMessage: (data: unknown, origin: string) =>
          opener.post(data, origin, opener.popup),
      },
      close: () => {
        if (opener.popup !== null) {
          opener.popup.closed = true;
        }
      },
    } as unknown as Window;
    expect(
      notifyCheckoutOpener({
        session: "cs_round",
        status: "complete",
        win: returnPage,
      }),
    ).toBe(true);
    expect(seen).toEqual([{ session: "cs_round", status: "complete" }]);
  });
});

describe("Stripe.openCheckoutPopup", () => {
  it("refuses when there is no browser window to open one from", async () => {
    const stripe = await loadStripe("pk_test_x", {
      baseUrl: "https://api.vpay.example",
      fetch: () => Promise.reject(new Error("no request expected")),
    });
    await expect(
      stripe.openCheckoutPopup({
        fetchCheckoutUrl: () => Promise.resolve(HOSTED_URL),
      }),
    ).rejects.toBeInstanceOf(TypeError);
  });

  it("refuses options with no fetchCheckoutUrl", async () => {
    const opener = stubWindow();
    await expect(
      openCheckoutPopup({
        win: opener.win,
        options: {} as unknown as { fetchCheckoutUrl: () => Promise<string> },
      }),
    ).rejects.toBeInstanceOf(TypeError);
  });
});
