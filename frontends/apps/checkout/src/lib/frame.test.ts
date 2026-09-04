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
import { describe, expect, it, vi } from 'vitest';

import { createFrameChannel, type ChildMessage } from './frame';

const PARENT = 'https://shop.example';

interface Harness {
  win: Window;
  sent: { message: ChildMessage; origin: string }[];
  deliver(event: MessageEvent): void;
  listenerCount(): number;
}

function stubWindow(): Harness {
  const sent: { message: ChildMessage; origin: string }[] = [];
  const listeners = new Set<(event: MessageEvent) => void>();
  const parent = {
    postMessage: (message: ChildMessage, origin: string) => {
      sent.push({ message, origin });
    },
  };
  const win = {
    parent,
    addEventListener: (_type: string, fn: (event: MessageEvent) => void) => listeners.add(fn),
    removeEventListener: (_type: string, fn: (event: MessageEvent) => void) => listeners.delete(fn),
  } as unknown as Window;
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

describe('createFrameChannel', () => {
  it('is null when the page is not framed, so nothing can post to itself', () => {
    const self = {} as Window;
    (self as unknown as { parent: Window }).parent = self;
    expect(createFrameChannel({ win: self, parentOrigin: PARENT })).toBeNull();
  });

  it('names the framer as the target of every message — never "*"', () => {
    const h = stubWindow();
    const channel = createFrameChannel({ win: h.win, parentOrigin: PARENT });
    channel?.post({ type: 'vpay:complete', session: 'cs_1', status: 'complete' });
    channel?.post({ type: 'vpay:redirect', url: 'https://rail.example/pay' });
    channel?.postHeight(412);

    expect(h.sent).toHaveLength(3);
    for (const call of h.sent) {
      expect(call.origin).toBe(PARENT);
      expect(call.origin).not.toBe('*');
    }
    expect(h.sent[2]?.message).toEqual({ type: 'vpay:resize', height: 412 });
  });

  it('posts a first height as soon as it is observed, since the parent starts the frame at 0', () => {
    const h = stubWindow();
    const observed = {
      getBoundingClientRect: () => ({ height: 220 }) as DOMRect,
    } as unknown as Element;
    createFrameChannel({ win: h.win, parentOrigin: PARENT, observe: observed });
    expect(h.sent).toEqual([{ message: { type: 'vpay:resize', height: 220 }, origin: PARENT }]);
  });

  it('reports a layout change through ResizeObserver', () => {
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
    (h.win as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;
    const observed = {
      getBoundingClientRect: () => ({ height: 100 }) as DOMRect,
    } as unknown as Element;
    createFrameChannel({ win: h.win, parentOrigin: PARENT, observe: observed });
    h.sent.length = 0;
    expect(callbacks).toHaveLength(1);
    callbacks[0]?.([{ contentRect: { height: 512.4 } } as ResizeObserverEntry], {} as ResizeObserver);
    expect(h.sent).toEqual([{ message: { type: 'vpay:resize', height: 513 }, origin: PARENT }]);
  });

  it('reads a message from the framer', () => {
    const h = stubWindow();
    const onMessage = vi.fn();
    createFrameChannel({ win: h.win, parentOrigin: PARENT, onMessage });
    h.deliver(new MessageEvent('message', { data: { type: 'ping' }, origin: PARENT }));
    expect(onMessage).toHaveBeenCalledWith({ type: 'ping' });
  });

  it('IGNORES a message from any other origin, without reading its payload', () => {
    const h = stubWindow();
    const onMessage = vi.fn();
    createFrameChannel({ win: h.win, parentOrigin: PARENT, onMessage });
    for (const origin of ['https://evil.example', 'null', 'http://shop.example', '']) {
      h.deliver(new MessageEvent('message', { data: { type: 'vpay:steal' }, origin }));
    }
    expect(onMessage).not.toHaveBeenCalled();
  });

  it('removes its listener on dispose', () => {
    const h = stubWindow();
    const channel = createFrameChannel({ win: h.win, parentOrigin: PARENT });
    expect(h.listenerCount()).toBe(1);
    channel?.dispose();
    expect(h.listenerCount()).toBe(0);
  });
});
