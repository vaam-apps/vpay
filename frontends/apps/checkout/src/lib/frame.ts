/**
 * The child half of the iframe↔parent protocol (D8).
 *
 * `@vpay/stripe-js`'s `initEmbeddedCheckout` is the parent half and lane 5
 * owns it. This is what runs *inside* the frame, and it holds exactly one
 * rule that matters more than the rest:
 *
 * > Every `postMessage` names its target origin, and that origin is the
 * > single origin that framed this page. Never `'*'`.
 *
 * `'*'` would broadcast the message to whatever document ends up in the
 * parent slot — including one that navigated there after this frame loaded.
 * `vpay:complete` carries a session id; `vpay:redirect` carries a URL a
 * parent is being asked to navigate to. Neither is for an audience.
 *
 * The mirror of that rule is on the way in: a `message` event whose
 * `event.origin` is not the framer is dropped without being read. Any page
 * that can get a handle to this frame's window can post to it.
 */
import type { CheckoutSessionStatus } from './types';

/** `{type:'vpay:resize', height}` — the parent sizes the iframe to the content. */
export interface ResizeMessage {
  type: 'vpay:resize';
  height: number;
}

/** `{type:'vpay:complete', session, status}` — the payment reached a terminal state. */
export interface CompleteMessage {
  type: 'vpay:complete';
  /** The checkout session's id. Never its `client_secret`. */
  session: string;
  status: CheckoutSessionStatus;
}

/**
 * `{type:'vpay:redirect', url}` — the rail wants a top-level navigation.
 *
 * An iframe may not navigate the top-level browsing context it is sandboxed
 * in, and even where it may, doing so from a frame is exactly the behaviour
 * browsers are steadily removing. So the child *asks*, and the parent —
 * which is the merchant's own page — performs the navigation.
 */
export interface RedirectMessage {
  type: 'vpay:redirect';
  url: string;
}

export type ChildMessage = ResizeMessage | CompleteMessage | RedirectMessage;

export interface FrameChannel {
  /** The origin every message is sent to and accepted from. */
  readonly parentOrigin: string;
  post(message: ChildMessage): void;
  /** Reports the document height to the parent. Debounced by the caller, not here. */
  postHeight(height: number): void;
  /** Stops the resize observer and removes the message listener. */
  dispose(): void;
}

export interface FrameChannelOptions {
  /** The framed window — `window` in the browser, a stub in tests. */
  win: Window;
  /** The single allowed origin, from {@link import('./origins.js').resolveParentOrigin}. */
  parentOrigin: string;
  /** Element whose height is reported. Usually `document.documentElement`. */
  observe?: Element | undefined;
  /** Called for a message that passed the origin check. */
  onMessage?: ((data: unknown) => void) | undefined;
}

/**
 * Opens the channel.
 *
 * Returns `null` when the window has no parent other than itself — a page
 * that is not framed has nobody to talk to, and posting to `window.parent`
 * when `parent === self` would deliver the message to this very document.
 */
export function createFrameChannel(options: FrameChannelOptions): FrameChannel | null {
  const { win, parentOrigin } = options;
  const parent: Window | null = win.parent;
  if (parent === null || parent === win) {
    return null;
  }

  const post = (message: ChildMessage): void => {
    // The target origin is the second argument, always, and always the
    // resolved framer. `postMessage(message, '*')` does not appear in this
    // repository's checkout code; `frame.test.ts` asserts that every call
    // this channel makes names `parentOrigin`.
    parent.postMessage(message, parentOrigin);
  };

  const listener = (event: MessageEvent): void => {
    if (event.origin !== parentOrigin) {
      // Dropped unread. Not logged either: the payload is attacker-supplied.
      return;
    }
    options.onMessage?.(event.data);
  };
  win.addEventListener('message', listener);

  // `ResizeObserver` is a global in `lib.dom`, not a member of `Window`, so
  // it is read off the injected window through a narrow cast rather than
  // reached for on `globalThis`: in a test the framed window is a stub, and
  // observing the *test runner's* document instead would prove nothing.
  const ResizeObserverCtor = (win as unknown as { ResizeObserver?: typeof ResizeObserver })
    .ResizeObserver;
  let observer: ResizeObserver | null = null;
  const observed = options.observe;
  if (observed !== undefined && typeof ResizeObserverCtor === 'function') {
    const created = new ResizeObserverCtor((entries) => {
      const entry = entries[0];
      if (entry === undefined) {
        return;
      }
      post({ type: 'vpay:resize', height: Math.ceil(entry.contentRect.height) });
    });
    created.observe(observed);
    observer = created;
  }

  // The parent creates the iframe at `height: 0` and grows it only when a
  // `vpay:resize` arrives, so a channel that waited for a layout *change*
  // would render an embedded checkout as an empty box. This first message
  // is what makes the frame visible at all. `ResizeObserver` also fires
  // once on `observe()` in a browser; posting here as well costs one
  // duplicate message and removes the dependency on that behaviour.
  if (observed !== undefined) {
    post({ type: 'vpay:resize', height: Math.ceil(observed.getBoundingClientRect().height) });
  }

  return {
    parentOrigin,
    post,
    postHeight(height: number): void {
      post({ type: 'vpay:resize', height: Math.ceil(height) });
    },
    dispose(): void {
      observer?.disconnect();
      win.removeEventListener('message', listener);
    },
  };
}
