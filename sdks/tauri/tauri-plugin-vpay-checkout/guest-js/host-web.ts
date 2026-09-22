/**
 * The plain-browser host: a `window.open` popup and the origin-and-source
 * pinned `vpay:complete` protocol.
 *
 * The port of
 * `sdks/flutter/vpay_checkout_flutter/lib/src/platform/web_checkout_platform.dart`,
 * with the rules of `sdks/stripe-js/src/popup.ts`. There is no native
 * window outside Tauri, and the *only* two signals available to a popup
 * opener are the ones that module documents — a cross-origin popup's
 * `location` cannot be read from the opener at all, which is exactly why
 * that protocol exists:
 *
 * - a `{ type: 'vpay:complete', … }` `postMessage` from the popup's own
 *   page, sent by the merchant's own `success_url`/`cancel_url` page once
 *   it lands there (`notifyCheckoutOpener` in `@vaam-apps/vpay-stripe-js`)
 *   — never by vpay's checkout page itself, which does not know it is
 *   inside a popup; or
 * - the popup's `closed` property going true with no such message having
 *   arrived, watched on a 500 ms poll.
 *
 * `stopUrls` and `allowInsecureUrl` are **inapplicable here** and this host
 * says so rather than pretending. There is no navigation for a stop URL to
 * match against, and `allowInsecureUrl` exists so a *native* host can be
 * told the demo stack's `http://` URL is expected rather than a mistake
 * (D6) — a browser's own address bar already shows the payer whatever
 * scheme `window.open` navigated to, and there is no separate host-side
 * check to relax.
 *
 * Note which origin is pinned: the **merchant's own**, not vpay's. The
 * message is sent by the merchant's return page, so pinning vpay's checkout
 * origin here would accept nothing at all —
 * `OpenCheckoutPopupOptions.completionOrigin`'s default, restated.
 */
import type {
  CheckoutHost,
  CheckoutWindowEvent,
  CheckoutWindowOutcome,
  ShowCheckoutRequest,
} from "./types.js";

/**
 * The `postMessage` payload's `type`, exactly as `popup.ts`'s
 * `COMPLETE_MESSAGE_TYPE` spells it — the two packages must agree on this
 * string, since one only ever reads what the other's return-page helper
 * writes.
 */
const COMPLETE_MESSAGE_TYPE = "vpay:complete";

/** How often `show` asks whether the payer closed the popup — `popup.ts`'s own value. */
const CLOSE_POLL_INTERVAL_MS = 500;

/** The window name and features, as `web_checkout_platform.dart` spells them. */
const WINDOW_NAME = "vpay-checkout";
const WINDOW_FEATURES = "popup=yes,location=yes,resizable=yes,scrollbars=yes";

/**
 * A {@link CheckoutHost} that opens the session's own `url` in a popup.
 *
 * `win` is injectable so the suite can drive this on Node. It is resolved
 * **lazily**, at `show`/`dismiss` time rather than as a default parameter,
 * so that {@link defaultHost} can be called in Node or during SSR without a
 * `ReferenceError` from a `window` that is not there.
 */
export function webHost(win?: Window): CheckoutHost {
  let popup: Window | null = null;
  let listener: ((event: MessageEvent) => void) | null = null;
  let poll: ReturnType<typeof setInterval> | undefined;
  let settled = false;
  let target: Window | null = null;

  function teardown(): void {
    if (poll !== undefined) {
      clearInterval(poll);
      poll = undefined;
    }
    if (listener !== null && target !== null) {
      target.removeEventListener("message", listener);
    }
    listener = null;
    popup = null;
  }

  function reportOnce(
    outcome: CheckoutWindowOutcome,
    onEvent: (event: CheckoutWindowEvent) => void,
  ): void {
    if (settled) {
      return;
    }
    settled = true;
    popup?.close();
    teardown();
    // `reachedUrl` is always `null` here, and not because it is unknown: a
    // cross-origin popup's location is unreadable from the opener, and D1
    // forbids reading an outcome off a URL even where one is available.
    onEvent({ outcome, reachedUrl: null });
  }

  return {
    // The `CheckoutHost` contract is a promise and nothing in here awaits:
    // `window.open` is synchronous, and yielding before the `message`
    // listener is wired is exactly the race this host must not have. The
    // method stays `async` so that a refused popup *rejects* rather than
    // throwing synchronously at a caller that did not `await`.
    // eslint-disable-next-line @typescript-eslint/require-await
    async show(
      request: ShowCheckoutRequest,
      onEvent: (event: CheckoutWindowEvent) => void,
    ): Promise<void> {
      teardown();
      settled = false;
      const resolved: Window | undefined =
        win ?? (globalThis as { window?: Window }).window;
      if (resolved === undefined) {
        throw new Error(
          "webHost: there is no window to open a checkout popup from.",
        );
      }
      target = resolved;

      const opened = resolved.open(request.url, WINDOW_NAME, WINDOW_FEATURES);
      if (opened === null) {
        // The one failure here that is **not** an integration mistake: the
        // merchant's code can be perfectly correct and the payer's browser
        // still say no. A fixed message, and no URL in it (D6) —
        // `VpayCheckout.start` replaces it with `platform_window_failed`
        // and never reads this text, but a merchant calling `webHost`
        // directly does.
        throw new Error(
          "webHost: the browser refused to open the checkout popup. Call VpayCheckout.start from a click handler, or use a full-page redirect instead.",
        );
      }
      popup = opened;

      const onMessage = (event: MessageEvent): void => {
        // The whole security boundary of this file. `message` is a global
        // event: without this check any page holding a handle to this
        // document could report a completed checkout.
        if (event.origin !== resolved.location.origin) {
          return;
        }
        // …and it must be *this* window, not merely something else on the
        // same origin — another tab, another popup, an iframe of the
        // merchant's own.
        if (event.source !== opened) {
          return;
        }
        const data: unknown = event.data;
        if (typeof data !== "object" || data === null) {
          return;
        }
        if (
          (data as Record<string, unknown>)["type"] !== COMPLETE_MESSAGE_TYPE
        ) {
          return;
        }
        // D1: the payload's own `session`/`status` are **not** read. A
        // `vpay:complete` says the popup reached the return page; it does
        // not say money moved, and only the poll that follows decides.
        reportOnce("stopUrlReached", onEvent);
      };
      listener = onMessage;
      resolved.addEventListener("message", onMessage);

      poll = setInterval(() => {
        if (popup === null || popup.closed) {
          reportOnce("dismissed", onEvent);
        }
      }, CLOSE_POLL_INTERVAL_MS);
    },

    // Async for the same reason `show` is; see above.
    // eslint-disable-next-line @typescript-eslint/require-await
    async dismiss(): Promise<void> {
      // Deliberately does not report an outcome. Closing the window makes
      // the close poll observe it on its next tick, which is the same
      // `dismissed` a payer's own close produces — one path, not two.
      popup?.close();
    },
  };
}
