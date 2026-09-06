/**
 * Checkout in a **popup**: a top-level window the merchant's page opens, and
 * the `postMessage` protocol that tells the opener the payer is finished.
 *
 * This is the third of the three surfaces a merchant can integrate against,
 * and it is the one that is not an iframe:
 *
 * | Surface  | Where vpay's page runs | How the merchant learns it finished |
 * |---|---|---|
 * | hosted   | the payer's own tab, after a full navigation | `success_url`, then the merchant's own webhook |
 * | embedded | an `<iframe>` on the merchant's page | `vpay:complete` from the frame ([embedded.ts](./embedded.ts)) |
 * | popup    | a separate top-level window | `vpay:complete` from the merchant's **own return page**, via `window.opener` |
 *
 * ## Why the message comes from the merchant's page and not from vpay's
 *
 * A popup is not framed: inside it `window.parent === window`, so vpay's
 * checkout page — whose child channel posts to `window.parent` and returns
 * `null` when there is no framer
 * (`frontends/apps/checkout/src/lib/frame.ts`) — has nobody to talk to and
 * deliberately says nothing. A popup integration therefore loads the
 * **hosted** session (`session.url`), exactly as a redirect integration
 * does, and the completion signal is emitted by the page vpay sends the
 * payer back to: the merchant's own `success_url`, which runs in the popup,
 * calls {@link notifyCheckoutOpener}, and closes itself.
 *
 * That has two consequences worth stating rather than discovering:
 *
 * 1. **The origin this side pins is the merchant's, not vpay's.** The
 *    message is sent by the merchant's return page, so
 *    {@link OpenCheckoutPopupOptions.completionOrigin} defaults to the
 *    opener's own origin. Pinning vpay's checkout origin here would accept
 *    nothing at all.
 * 2. **A completion message is still only a cue.** It says the popup reached
 *    the return page; it does not say money moved. The order is settled by
 *    the merchant's signature-verified webhook, and every one of this
 *    package's completion paths says so.
 *
 * ## The window is opened before the URL is known, and that is deliberate
 *
 * `window.open` succeeds only while the browser still considers itself
 * inside the user gesture that triggered it. Awaiting the merchant's server
 * call first and opening afterwards is the single most common way a popup
 * integration gets blocked, so {@link VpayStripe.openCheckoutPopup} opens a
 * blank window **synchronously** and navigates it once
 * {@link OpenCheckoutPopupOptions.fetchCheckoutUrl} resolves. A blocked
 * window is reported as {@link CheckoutPopupBlockedError} so the merchant
 * can fall back to a full-page redirect, which is a thing no browser blocks.
 */
import type { EmbeddedCheckoutCompleteEvent } from "./types.js";

/** The message type vpay's surfaces use to report completion. Shared with `embedded.ts`. */
const COMPLETE_MESSAGE_TYPE = "vpay:complete";

/** How often {@link openCheckoutPopup} asks whether the payer closed the window. */
const CLOSE_POLL_INTERVAL_MS = 500;

/** The popup's default size — big enough for vpay's checkout page on a laptop. */
const DEFAULT_WIDTH = 480;
const DEFAULT_HEIGHT = 720;

/**
 * The browser refused to open the window.
 *
 * Its own class, not a `TypeError`, because it is the one failure here that
 * is **not** an integration mistake: the merchant's code can be perfectly
 * correct and the payer's browser still say no. A merchant is expected to
 * catch this one and navigate the current tab instead.
 */
export class CheckoutPopupBlockedError extends Error {
  constructor() {
    super(
      "openCheckoutPopup: the browser refused to open a window. Call this " +
        "directly from a click handler, or fall back to a full-page redirect.",
    );
    this.name = "CheckoutPopupBlockedError";
  }
}

/** Options for {@link VpayStripe.openCheckoutPopup}. */
export interface OpenCheckoutPopupOptions {
  /**
   * Returns the **hosted** Checkout Session's `url` — the merchant's own
   * server call to `POST /v1/checkout/sessions` with `ui_mode: 'hosted'`,
   * proxied through its page. Called once; a rejection propagates, and the
   * window opened for it is closed first.
   */
  fetchCheckoutUrl: () => Promise<string>;
  /**
   * The single origin a `vpay:complete` message is accepted from — the
   * merchant's own page, since that is what sends it (see this module's
   * header). Defaults to the opener's own origin, which is right whenever
   * `success_url` is on the same origin as the page that opened the popup.
   */
  completionOrigin?: string | undefined;
  /**
   * Called when the merchant's return page reports the payer is finished.
   * **Not** proof of payment — re-read the order from the merchant's own
   * server, which the webhook is what writes.
   */
  onComplete?: ((event: EmbeddedCheckoutCompleteEvent) => void) | undefined;
  /**
   * Called when the window closed without a completion message — the payer
   * dismissed it. Not a failure, and not a cancellation of the charge: the
   * PaymentIntent is untouched and may still settle.
   */
  onCancel?: (() => void) | undefined;
  /** Popup width in CSS pixels. Default 480. */
  width?: number | undefined;
  /** Popup height in CSS pixels. Default 720. */
  height?: number | undefined;
  /**
   * The window name. Default `vpay-checkout`. Reusing a name reuses the
   * window, which is what stops a second click opening a second checkout.
   */
  windowName?: string | undefined;
}

/** The handle {@link VpayStripe.openCheckoutPopup} resolves to. */
export interface CheckoutPopup {
  /** Whether the payer's window has gone. */
  readonly closed: boolean;
  /** Brings the window forward — what a merchant's "continue payment" button calls. */
  focus(): void;
  /** Closes the window. `onCancel` does **not** fire for a close the merchant asked for. */
  close(): void;
  /** Drops the `message` listener and the close poll. Leaves the window alone. */
  destroy(): void;
}

/**
 * True only for an absolute `http:`/`https:` URL.
 *
 * The value is the rail-facing `session.url` vpay minted, arriving through
 * the merchant's own server — but it is still a string this package is
 * about to navigate a window to, and `javascript:` in a window this page
 * opened runs in **that** window with a live `window.opener` handle back to
 * the merchant's document.
 */
function isSafeTopLevelUrl(url: string): boolean {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  return parsed.protocol === "http:" || parsed.protocol === "https:";
}

/** Reads a `vpay:complete` payload, or `undefined` if it is not one. */
function parseCompleteEvent(
  data: Record<string, unknown>,
): EmbeddedCheckoutCompleteEvent | undefined {
  const session = data["session"];
  const status = data["status"];
  if (typeof session !== "string" || typeof status !== "string") {
    return undefined;
  }
  return { session, status };
}

/** `window`, or `undefined` where there is none (Node, SSR, a worker). */
function browserWindow(): Window | undefined {
  return typeof window === "undefined" ? undefined : window;
}

/** `left`/`top` that centre the popup over the opener, when it can be measured. */
function windowFeatures(win: Window, width: number, height: number): string {
  const screenX = typeof win.screenX === "number" ? win.screenX : 0;
  const screenY = typeof win.screenY === "number" ? win.screenY : 0;
  const outerWidth =
    typeof win.outerWidth === "number" ? win.outerWidth : width;
  const outerHeight =
    typeof win.outerHeight === "number" ? win.outerHeight : height;
  const left = Math.max(0, Math.round(screenX + (outerWidth - width) / 2));
  const top = Math.max(0, Math.round(screenY + (outerHeight - height) / 2));
  return [
    "popup=yes",
    `width=${width}`,
    `height=${height}`,
    `left=${left}`,
    `top=${top}`,
    // Withheld deliberately: `location=yes` keeps the address bar visible,
    // which is the payer's only way to see whose page is asking for their
    // money. A popup checkout that hides it is a phishing lesson.
    "location=yes",
    "resizable=yes",
    "scrollbars=yes",
  ].join(",");
}

/** Everything {@link openCheckoutPopup} needs, resolved by the caller. */
export interface CheckoutPopupConfig {
  /** The window doing the opening — injectable so a test can supply its own. */
  win: Window;
  options: OpenCheckoutPopupOptions;
}

/**
 * Opens the window, navigates it, and wires the completion channel.
 *
 * Exported for `VpayStripe.openCheckoutPopup` and for this package's own
 * tests; merchants call the method on the `Stripe` object.
 *
 * # Errors
 *
 * Rejects with {@link CheckoutPopupBlockedError} when the browser refused
 * the window, with a `TypeError` when `fetchCheckoutUrl` did not answer an
 * absolute `http(s)` URL, and with whatever `fetchCheckoutUrl` itself threw
 * — unwrapped, because that is the merchant's own server call failing and a
 * message invented on top of it would hide the fault.
 */
export async function openCheckoutPopup(
  config: CheckoutPopupConfig,
): Promise<CheckoutPopup> {
  const { win, options } = config;
  if (typeof options?.fetchCheckoutUrl !== "function") {
    throw new TypeError(
      "openCheckoutPopup: options.fetchCheckoutUrl must be a function returning a Promise<string>",
    );
  }
  const width = options.width ?? DEFAULT_WIDTH;
  const height = options.height ?? DEFAULT_HEIGHT;

  // Synchronously, before any `await`. See this module's header.
  const opened = win.open(
    "",
    options.windowName ?? "vpay-checkout",
    windowFeatures(win, width, height),
  );
  if (opened === null) {
    throw new CheckoutPopupBlockedError();
  }
  // Rebound to a non-nullable const because `onMessage` below is a hoisted
  // function declaration: TypeScript creates it before the check above runs
  // and would not carry the narrowing into it.
  const popup: Window = opened;

  let url: string;
  try {
    url = await options.fetchCheckoutUrl();
  } catch (cause) {
    popup.close();
    throw cause;
  }
  if (typeof url !== "string" || !isSafeTopLevelUrl(url)) {
    popup.close();
    throw new TypeError(
      "openCheckoutPopup: fetchCheckoutUrl must resolve to an absolute http(s) checkout URL",
    );
  }

  const completionOrigin = options.completionOrigin ?? win.location.origin;

  let finished = false;
  let timer: ReturnType<typeof setInterval> | undefined;

  const stop = (): void => {
    win.removeEventListener("message", onMessage);
    if (timer !== undefined) {
      clearInterval(timer);
      timer = undefined;
    }
  };

  function onMessage(event: MessageEvent): void {
    // The whole security boundary of this file. Everything below trusts the
    // sender: `message` is a global event, and without this check any page
    // holding a handle to this document could report a completed checkout.
    if (event.origin !== completionOrigin) {
      return;
    }
    // …and it must be *this* window, not merely something else on the same
    // origin — another tab, another popup, an iframe of the merchant's own.
    if (event.source !== popup) {
      return;
    }
    const data: unknown = event.data;
    if (typeof data !== "object" || data === null) {
      return;
    }
    const message = data as Record<string, unknown>;
    if (message["type"] !== COMPLETE_MESSAGE_TYPE) {
      return;
    }
    const payload = parseCompleteEvent(message);
    if (payload === undefined) {
      return;
    }
    finished = true;
    stop();
    // Closed by this side, so the payer is not left staring at a window
    // that has already done its job. The merchant's return page closes
    // itself too; whichever wins, `close()` on a closed window is a no-op.
    popup.close();
    options.onComplete?.(payload);
  }

  win.addEventListener("message", onMessage);

  if (options.onCancel !== undefined) {
    // A closed window fires no event at the opener, in any browser. Polling
    // `closed` is the only way to notice, and it is why this is opt-in: a
    // merchant that does not pass `onCancel` gets no timer at all.
    timer = setInterval(() => {
      if (!popup.closed) {
        return;
      }
      stop();
      if (!finished) {
        options.onCancel?.();
      }
    }, CLOSE_POLL_INTERVAL_MS);
  }

  popup.location.assign(url);

  return {
    get closed(): boolean {
      return popup.closed;
    },
    focus(): void {
      popup.focus();
    },
    close(): void {
      // `finished` first, so the close poll does not report a cancellation
      // for a close the merchant asked for.
      finished = true;
      stop();
      popup.close();
    },
    destroy(): void {
      finished = true;
      stop();
    },
  };
}

/** Options for {@link notifyCheckoutOpener}. */
export interface NotifyCheckoutOpenerOptions {
  /** The `cs_…` that completed — vpay substitutes it into `success_url` (D5). */
  session: string;
  /** What the merchant's page knows of the outcome. A label, never authority. */
  status: string;
  /**
   * The origin to post to: the merchant's own page that opened the popup.
   * Defaults to this page's origin, which is right whenever `success_url`
   * and the shop are the same origin.
   *
   * **Never `'*'`, and there is no option to make it one.** The message
   * names a checkout session, and a broadcast would hand that id to
   * whatever document happens to hold the opener slot.
   */
  targetOrigin?: string | undefined;
  /** Close this window after posting. Default `true`. */
  close?: boolean | undefined;
  /** The window to read `opener` from — injectable so a test can supply one. */
  win?: Window | undefined;
}

/**
 * The popup half: tells the opener the payer is finished, then closes.
 *
 * Called from the merchant's own `success_url` page. Returns `false` — and
 * does nothing at all — when there is no opener, which is exactly what
 * happens when the same page is reached by an ordinary redirect. That is
 * the property that lets one return page serve the hosted and the popup
 * integrations without branching on a query parameter.
 *
 * ```ts
 * import { notifyCheckoutOpener } from '@vaam-apps/vpay-stripe-js';
 *
 * // On /orders/{id}/return, which vpay substituted the session id into.
 * notifyCheckoutOpener({ session: sessionId, status: 'complete' });
 * ```
 */
export function notifyCheckoutOpener(
  options: NotifyCheckoutOpenerOptions,
): boolean {
  const win = options.win ?? browserWindow();
  if (win === undefined) {
    return false;
  }
  const opener = win.opener as Window | null | undefined;
  // `opener === win` cannot happen in a browser, but a stub window in a test
  // could be its own opener, and posting to ourselves would fire the
  // listener this function exists to feed from the wrong side.
  if (
    opener === null ||
    opener === undefined ||
    opener === win ||
    typeof opener.postMessage !== "function"
  ) {
    return false;
  }
  if (typeof options.session !== "string" || options.session.length === 0) {
    throw new TypeError("notifyCheckoutOpener: options.session is required");
  }
  const targetOrigin = options.targetOrigin ?? win.location.origin;
  opener.postMessage(
    {
      type: COMPLETE_MESSAGE_TYPE,
      session: options.session,
      status: typeof options.status === "string" ? options.status : "",
    },
    targetOrigin,
  );
  if (options.close !== false) {
    win.close();
  }
  return true;
}
