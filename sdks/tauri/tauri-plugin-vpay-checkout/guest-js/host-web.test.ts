/**
 * The plain-browser host against a hand-rolled `Window`.
 *
 * Not jsdom, deliberately. What is under test is this host's *rules* — the
 * two checks on a `message`, the one event, the 500 ms close poll, the
 * teardown — and a hand-rolled window makes each of those an assertion
 * about the host rather than about jsdom's fidelity to `window.open`, which
 * jsdom does not implement at all.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import { webHost } from "./host-web.js";
import type { CheckoutWindowEvent, ShowCheckoutRequest } from "./types.js";

const ORIGIN = "https://shop.example";

const REQUEST: ShowCheckoutRequest = {
  url: "https://checkout.example/c/cs_123?key=pk_test_1#cs_123_secret_abcdef",
  stopUrls: [],
  allowInsecureUrl: false,
};

interface FakePopup {
  closed: boolean;
  closes: number;
  close(): void;
}

function fakePopup(): FakePopup {
  return {
    closed: false,
    closes: 0,
    close(): void {
      this.closes += 1;
      this.closed = true;
    },
  };
}

interface FakeWindow {
  opens: { url: string; name: string; features: string }[];
  listeners: Map<string, Set<(event: MessageEvent) => void>>;
  /** Delivers a `message` to whatever this window currently listens with. */
  emit(event: Partial<MessageEvent>): void;
  asWindow: Window;
}

function fakeWindow(popup: FakePopup | null): FakeWindow {
  const opens: { url: string; name: string; features: string }[] = [];
  const listeners = new Map<string, Set<(event: MessageEvent) => void>>();
  const win = {
    location: { origin: ORIGIN },
    open(url: string, name: string, features: string): unknown {
      opens.push({ url, name, features });
      return popup;
    },
    addEventListener(type: string, handler: (event: MessageEvent) => void) {
      const set = listeners.get(type) ?? new Set();
      set.add(handler);
      listeners.set(type, set);
    },
    removeEventListener(type: string, handler: (event: MessageEvent) => void) {
      listeners.get(type)?.delete(handler);
    },
  };
  return {
    opens,
    listeners,
    emit(event) {
      for (const handler of listeners.get("message") ?? []) {
        handler(event as MessageEvent);
      }
    },
    asWindow: win as unknown as Window,
  };
}

function complete(
  source: unknown,
  origin = ORIGIN,
  extra: Record<string, unknown> = {},
): Partial<MessageEvent> {
  return {
    origin,
    source: source as MessageEventSource,
    data: {
      type: "vpay:complete",
      session: "cs_123",
      status: "complete",
      ...extra,
    },
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("webHost.show — opening the popup", () => {
  it("opens one popup at the session URL with the documented window features", async () => {
    const popup = fakePopup();
    const win = fakeWindow(popup);

    await webHost(win.asWindow).show(REQUEST, () => undefined);

    expect(win.opens).toEqual([
      {
        url: REQUEST.url,
        name: "vpay-checkout",
        // `location=yes` is asked for deliberately and there is no option
        // to turn it off: the address bar is the payer's only way to see
        // whose page is asking for their money.
        features: "popup=yes,location=yes,resizable=yes,scrollbars=yes",
      },
    ]);
  });

  it("a refused popup rejects show with a fixed message that never quotes the URL", async () => {
    const win = fakeWindow(null);

    const thrown = await webHost(win.asWindow)
      .show(REQUEST, () => undefined)
      .then(
        () => undefined,
        (error: unknown) => error,
      );

    expect(thrown).toBeInstanceOf(Error);
    expect((thrown as Error).message).toContain(
      "refused to open the checkout popup",
    );
    expect((thrown as Error).message).not.toContain("cs_123_secret_abcdef");
  });

  it("rejects with a fixed message where there is no window at all", async () => {
    // Node, SSR, a worker. `defaultHost()` can be called there without
    // throwing, and this is where that decision surfaces.
    await expect(webHost().show(REQUEST, () => undefined)).rejects.toThrow(
      /no window to open a checkout popup from/u,
    );
  });
});

describe("webHost — the vpay:complete protocol", () => {
  it("a message from this page's own origin and from the popup itself resolves stopUrlReached", async () => {
    const popup = fakePopup();
    const win = fakeWindow(popup);
    const seen: CheckoutWindowEvent[] = [];

    await webHost(win.asWindow).show(REQUEST, (event) => seen.push(event));
    win.emit(complete(popup));

    expect(seen).toEqual([{ outcome: "stopUrlReached", reachedUrl: null }]);
  });

  it("a message from a different origin is ignored — the whole security boundary of this host", async () => {
    const popup = fakePopup();
    const win = fakeWindow(popup);
    const seen: CheckoutWindowEvent[] = [];

    await webHost(win.asWindow).show(REQUEST, (event) => seen.push(event));
    win.emit(complete(popup, "https://evil.example"));

    expect(seen).toEqual([]);
  });

  it("a message from another window on the same origin is ignored", async () => {
    // Another tab, another popup, an iframe of the merchant's own can all
    // share this page's origin and still hold a `postMessage` handle to it.
    const popup = fakePopup();
    const win = fakeWindow(popup);
    const seen: CheckoutWindowEvent[] = [];

    await webHost(win.asWindow).show(REQUEST, (event) => seen.push(event));
    win.emit(complete(fakePopup()));

    expect(seen).toEqual([]);
  });

  it("a message that is not a vpay:complete is ignored", async () => {
    const popup = fakePopup();
    const win = fakeWindow(popup);
    const seen: CheckoutWindowEvent[] = [];

    await webHost(win.asWindow).show(REQUEST, (event) => seen.push(event));
    win.emit({
      origin: ORIGIN,
      source: popup as unknown as MessageEventSource,
      data: "hello",
    });
    win.emit({
      origin: ORIGIN,
      source: popup as unknown as MessageEventSource,
      data: { type: "something-else" },
    });

    expect(seen).toEqual([]);
  });

  it("the outcome is never read off the message payload, whatever status or session it carries (D1)", async () => {
    const popup = fakePopup();
    const win = fakeWindow(popup);
    const seen: CheckoutWindowEvent[] = [];

    await webHost(win.asWindow).show(REQUEST, (event) => seen.push(event));
    win.emit(
      complete(popup, ORIGIN, { status: "expired", session: "cs_other" }),
    );

    // `stopUrlReached`, not "expired": the message says the popup reached
    // the return page, and only the poll that follows decides anything.
    expect(seen).toEqual([{ outcome: "stopUrlReached", reachedUrl: null }]);
  });
});

describe("webHost — the close poll and the one-event rule", () => {
  it("the popup's own closed state reports a dismissal on the 500 ms poll", async () => {
    vi.useFakeTimers();
    const popup = fakePopup();
    const win = fakeWindow(popup);
    const seen: CheckoutWindowEvent[] = [];

    await webHost(win.asWindow).show(REQUEST, (event) => seen.push(event));
    expect(seen).toEqual([]);
    popup.closed = true;
    vi.advanceTimersByTime(500);

    expect(seen).toEqual([{ outcome: "dismissed", reachedUrl: null }]);
  });

  it("reports exactly one event, and the teardown drops the listener and the interval", async () => {
    vi.useFakeTimers();
    const popup = fakePopup();
    const win = fakeWindow(popup);
    const seen: CheckoutWindowEvent[] = [];

    await webHost(win.asWindow).show(REQUEST, (event) => seen.push(event));
    win.emit(complete(popup));
    // Everything that could produce a second one, after the first.
    win.emit(complete(popup));
    vi.advanceTimersByTime(5_000);

    expect(seen).toHaveLength(1);
    expect(win.listeners.get("message")?.size ?? 0).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
    // Closed by this side, so the payer is not left staring at a window
    // that has already done its job.
    expect(popup.closes).toBe(1);
  });
});

describe("webHost.dismiss", () => {
  it("closes the popup without inventing an outcome", async () => {
    vi.useFakeTimers();
    const popup = fakePopup();
    const win = fakeWindow(popup);
    const seen: CheckoutWindowEvent[] = [];
    const host = webHost(win.asWindow);

    await host.show(REQUEST, (event) => seen.push(event));
    await host.dismiss();

    // Nothing reported yet: `dismiss` closes the window, and the close
    // poll observes it on its next tick — one path to `dismissed`, not
    // two.
    expect(popup.closed).toBe(true);
    expect(seen).toEqual([]);
    vi.advanceTimersByTime(500);
    expect(seen).toEqual([{ outcome: "dismissed", reachedUrl: null }]);
  });
});
