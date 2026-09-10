/**
 * The child half of the protocol: who a message is sent to, and whose
 * messages are read.
 *
 * The window is a stub rather than jsdom's, and deliberately: in jsdom
 * `window.parent === window`, so a framed page cannot be expressed at all
 * there, and the property under test is precisely *which object* receives
 * the `postMessage` and *what second argument* it is given. The
 * `MessageEvent` is the platform's own.
 */
import { describe, expect, it, vi } from "vitest";

import { createFrameChannel, type ChildMessage } from "./frame";

const PARENT = "https://shop.example";

interface Harness {
  win: Window;
  sent: { message: ChildMessage; origin: string }[];
  deliver(event: MessageEvent): void;
  listenerCount(): number;
}

function stubWindow(peer: "parent" | "opener" = "parent"): Harness {
  const sent: { message: ChildMessage; origin: string }[] = [];
  const listeners = new Set<(event: MessageEvent) => void>();
  const other = {
    postMessage: (message: ChildMessage, origin: string) => {
      sent.push({ message, origin });
    },
  };
  const win = {
    // A popup's `window.parent` IS the popup: that is exactly the fact that
    // made the old channel silent in one, so the stub models it.
    parent: peer === "opener" ? undefined : other,
    opener: peer === "opener" ? other : null,
    addEventListener: (_type: string, fn: (event: MessageEvent) => void) =>
      listeners.add(fn),
    removeEventListener: (_type: string, fn: (event: MessageEvent) => void) =>
      listeners.delete(fn),
  } as unknown as Window;
  if (peer === "opener") {
    (win as unknown as { parent: Window }).parent = win;
  }
  return {
    win,
    sent,
    deliver: (event) => {
      for (const fn of listeners) {
        fn(event);
      }
    },
    listenerCount: () => listeners.size,
  };
}

describe("createFrameChannel", () => {
  it("is null when the page is not framed, so nothing can post to itself", () => {
    const self = {} as Window;
    (self as unknown as { parent: Window }).parent = self;
    expect(createFrameChannel({ win: self, parentOrigin: PARENT })).toBeNull();
  });

  it('names the framer as the target of every message — never "*"', () => {
    const h = stubWindow();
    const channel = createFrameChannel({ win: h.win, parentOrigin: PARENT });
    channel?.post({
      type: "vpay:complete",
      session: "cs_1",
      status: "complete",
    });
    channel?.post({ type: "vpay:redirect", url: "https://rail.example/pay" });
    channel?.postHeight(412);

    expect(h.sent).toHaveLength(3);
    for (const call of h.sent) {
      expect(call.origin).toBe(PARENT);
      expect(call.origin).not.toBe("*");
    }
    expect(h.sent[2]?.message).toEqual({ type: "vpay:resize", height: 412 });
  });

  it("posts a first height as soon as it is observed, since the parent starts the frame at 0", () => {
    const h = stubWindow();
    const observed = {
      getBoundingClientRect: () => ({ height: 220 }) as DOMRect,
    } as unknown as Element;
    createFrameChannel({ win: h.win, parentOrigin: PARENT, observe: observed });
    expect(h.sent).toEqual([
      { message: { type: "vpay:resize", height: 220 }, origin: PARENT },
    ]);
  });

  it("reports a layout change through ResizeObserver", () => {
    const h = stubWindow();
    // An array, not a `let`: TypeScript's flow analysis narrows a `let`
    // assigned only inside a constructor back to `null` at the call site.
    const callbacks: ResizeObserverCallback[] = [];
    class FakeResizeObserver {
      constructor(cb: ResizeObserverCallback) {
        callbacks.push(cb);
      }
      observe(): void {}
      disconnect(): void {}
      unobserve(): void {}
    }
    (h.win as unknown as { ResizeObserver: unknown }).ResizeObserver =
      FakeResizeObserver;
    const observed = {
      getBoundingClientRect: () => ({ height: 100 }) as DOMRect,
    } as unknown as Element;
    createFrameChannel({ win: h.win, parentOrigin: PARENT, observe: observed });
    h.sent.length = 0;
    expect(callbacks).toHaveLength(1);
    callbacks[0]?.(
      [{ contentRect: { height: 512.4 } } as ResizeObserverEntry],
      {} as ResizeObserver,
    );
    expect(h.sent).toEqual([
      { message: { type: "vpay:resize", height: 513 }, origin: PARENT },
    ]);
  });

  it("reads a message from the framer", () => {
    const h = stubWindow();
    const onMessage = vi.fn();
    createFrameChannel({ win: h.win, parentOrigin: PARENT, onMessage });
    h.deliver(
      new MessageEvent("message", { data: { type: "ping" }, origin: PARENT }),
    );
    expect(onMessage).toHaveBeenCalledWith({ type: "ping" });
  });

  it("IGNORES a message from any other origin, without reading its payload", () => {
    const h = stubWindow();
    const onMessage = vi.fn();
    createFrameChannel({ win: h.win, parentOrigin: PARENT, onMessage });
    for (const origin of [
      "https://evil.example",
      "null",
      "http://shop.example",
      "",
    ]) {
      h.deliver(
        new MessageEvent("message", { data: { type: "vpay:steal" }, origin }),
      );
    }
    expect(onMessage).not.toHaveBeenCalled();
  });

  it("removes its listener on dispose", () => {
    const h = stubWindow();
    const channel = createFrameChannel({ win: h.win, parentOrigin: PARENT });
    expect(h.listenerCount()).toBe(1);
    channel?.dispose();
    expect(h.listenerCount()).toBe(0);
  });
});

describe("a popup, where the peer is the opener", () => {
  it("opens a channel where the old parent-only one was silent", () => {
    const h = stubWindow("opener");
    // The whole defect this fixes: in a popup `window.parent === window`,
    // so asking for the parent channel answers null and vpay says nothing
    // to the merchant at all.
    expect(createFrameChannel({ win: h.win, parentOrigin: PARENT })).toBeNull();
    const channel = createFrameChannel({
      win: h.win,
      peer: "opener",
      parentOrigin: PARENT,
    });
    expect(channel?.peer).toBe("opener");
  });

  it('names the opener’s origin as the target of every message — never "*"', () => {
    const h = stubWindow("opener");
    const channel = createFrameChannel({
      win: h.win,
      peer: "opener",
      parentOrigin: PARENT,
    });
    channel?.post({
      type: "vpay:complete",
      session: "cs_1",
      status: "complete",
    });
    expect(h.sent).toEqual([
      {
        message: { type: "vpay:complete", session: "cs_1", status: "complete" },
        origin: PARENT,
      },
    ]);
    expect(h.sent.every((entry) => entry.origin !== "*")).toBe(true);
  });

  it("is null when there is no opener", () => {
    const h = stubWindow("parent");
    expect(
      createFrameChannel({ win: h.win, peer: "opener", parentOrigin: PARENT }),
    ).toBeNull();
  });

  it("reports no height to an opener, which does not lay this window out", () => {
    const h = stubWindow("opener");
    const observed = {
      getBoundingClientRect: () => ({ height: 480 }),
    } as unknown as Element;
    createFrameChannel({
      win: h.win,
      peer: "opener",
      parentOrigin: PARENT,
      observe: observed,
    });
    // A frame gets a first-paint `vpay:resize` because the parent creates it
    // at height 0. A popup sizes itself.
    expect(h.sent).toEqual([]);
  });

  it("still drops a message from any origin but the opener’s", () => {
    const h = stubWindow("opener");
    const seen: unknown[] = [];
    createFrameChannel({
      win: h.win,
      peer: "opener",
      parentOrigin: PARENT,
      onMessage: (data) => seen.push(data),
    });
    h.deliver(
      new MessageEvent("message", {
        origin: "https://evil.example",
        data: { x: 1 },
      }),
    );
    expect(seen).toEqual([]);
    h.deliver(new MessageEvent("message", { origin: PARENT, data: { x: 2 } }));
    expect(seen).toEqual([{ x: 2 }]);
  });

  it("still defaults to the parent when no peer is named", () => {
    const h = stubWindow("parent");
    expect(createFrameChannel({ win: h.win, parentOrigin: PARENT })?.peer).toBe(
      "parent",
    );
  });
});
