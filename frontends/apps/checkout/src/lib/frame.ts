/**
 * The child half of the iframe↔parent protocol (D8).
 *
 * `@vaam-apps/vpay-stripe-js`'s `initEmbeddedCheckout` is the parent half and lane 5
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
import type { CheckoutSessionStatus } from "./types";

/** `{type:'vpay:resize', height}` — the parent sizes the iframe to the content. */
export interface ResizeMessage {
  type: "vpay:resize";
  height: number;
}

/** `{type:'vpay:complete', session, status}` — the payment reached a terminal state. */
export interface CompleteMessage {
  type: "vpay:complete";
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
  type: "vpay:redirect";
  url: string;
}

export type ChildMessage = ResizeMessage | CompleteMessage | RedirectMessage;

/**
 * Which window on the other end of the channel.
 *
 * `parent` is the classic embedded case: `/e/{id}` inside the merchant's own
 * iframe. `opener` is a **popup** — `/c/{id}` opened with `window.open` from
 * the merchant's page (2026-09-06). A popup is not a frame: `window.parent`
 * is the popup itself, so a channel that only ever looked at `parent` said
 * nothing to the merchant at all.
 *
 * The two differ in more than which handle is posted to, which is why this
 * is a value rather than an implementation detail:
 *
 * - **No `vpay:resize` to an opener.** A popup owns its own window size;
 *   telling the merchant's page how tall this document is would be telling
 *   it about a window it does not lay out.
 * - **No `vpay:redirect` to an opener.** A popup *is* a top-level browsing
 *   context and may navigate itself, and asking the opener to navigate would
 *   send the merchant's own page to Orange Money out from under the payer.
 *   `CheckoutController` reads {@link FrameChannel.peer} to decide.
 */
export type ChannelPeer = "parent" | "opener";

export interface FrameChannel {
  /** Which window is on the other end. */
  readonly peer: ChannelPeer;
  /** The origin every message is sent to and accepted from. */
  readonly parentOrigin: string;
  post(message: ChildMessage): void;
  /** Reports the document height to the parent. Debounced by the caller, not here. */
  postHeight(height: number): void;
  /** Stops the resize observer and removes the message listener. */
  dispose(): void;
}

export interface FrameChannelOptions {
  /** The framed or popped-up window — `window` in the browser, a stub in tests. */
  win: Window;
  /** Which window to talk to. Defaults to `parent`, the embedded case. */
  peer?: ChannelPeer | undefined;
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
 * Returns `null` when there is nobody on the other end: a page that is not
 * framed has no parent but itself (posting to `window.parent` when
 * `parent === self` would deliver the message to this very document), and a
 * page that was not opened by a script has no opener.
 */
export function createFrameChannel(
  options: FrameChannelOptions,
): FrameChannel | null {
  const { win, parentOrigin } = options;
  const peer: ChannelPeer = options.peer ?? "parent";
  // `Window.opener` is `any` in `lib.dom` — it is whatever the opener chose
  // to leave there for same-origin openers — so it is narrowed here rather
  // than trusted. Nothing is ever *read* off it: the only thing this channel
  // does with the handle is `postMessage` to a pinned origin.
  const opener: unknown = win.opener;
  const peerWindow: Window | null =
    peer === "opener"
      ? typeof opener === "object" && opener !== null
        ? (opener as Window)
        : null
      : win.parent;
  if (peerWindow === null || peerWindow === undefined || peerWindow === win) {
    return null;
  }

  const post = (message: ChildMessage): void => {
    // The target origin is the second argument, always, and always the
    // resolved framer or opener. `postMessage(message, '*')` does not appear
    // in this repository's checkout code; `frame.test.ts` asserts that every
    // call this channel makes names `parentOrigin`, on both peers.
    peerWindow.postMessage(message, parentOrigin);
  };

  const listener = (event: MessageEvent): void => {
    if (event.origin !== parentOrigin) {
      // Dropped unread. Not logged either: the payload is attacker-supplied.
      return;
    }
    options.onMessage?.(event.data);
  };
  win.addEventListener("message", listener);

  // `ResizeObserver` is a global in `lib.dom`, not a member of `Window`, so
  // it is read off the injected window through a narrow cast rather than
  // reached for on `globalThis`: in a test the framed window is a stub, and
  // observing the *test runner's* document instead would prove nothing.
  const ResizeObserverCtor = (
    win as unknown as { ResizeObserver?: typeof ResizeObserver }
  ).ResizeObserver;
  let observer: ResizeObserver | null = null;
  // No height reporting to an opener: see {@link ChannelPeer}.
  const observed = peer === "parent" ? options.observe : undefined;
  if (observed !== undefined && typeof ResizeObserverCtor === "function") {
    const created = new ResizeObserverCtor((entries) => {
      const entry = entries[0];
      if (entry === undefined) {
        return;
      }
      post({
        type: "vpay:resize",
        height: Math.ceil(entry.contentRect.height),
      });
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
    post({
      type: "vpay:resize",
      height: Math.ceil(observed.getBoundingClientRect().height),
    });
  }

  return {
    peer,
    parentOrigin,
    post,
    postHeight(height: number): void {
      post({ type: "vpay:resize", height: Math.ceil(height) });
    },
    dispose(): void {
      observer?.disconnect();
      win.removeEventListener("message", listener);
    },
  };
}
