/**
 * Embedded Checkout: vpay's own checkout page, framed on the merchant's
 * page, and the `postMessage` protocol between the two
 * (`docs/plans/2026-09-04-step9-hosted-checkout.md`, D6 and D8).
 *
 * Three rules hold in this file, and each one is a test:
 *
 * 1. **The secret rides in the fragment, never the query string** (D6). A
 *    query string reaches the checkout app's server logs, its access log,
 *    and every proxy in between; a fragment is never sent to a server at
 *    all. The publishable key *is* in the query, because the page's own
 *    server needs it to look the tenant's `frame-ancestors` up before any
 *    script runs, and a publishable key is public by name.
 * 2. **`event.origin` is compared against one pinned value**, derived from
 *    `checkoutBaseUrl`. `message` is a global event: without this check any
 *    page the merchant frames — an ad, an analytics pixel, an attacker's
 *    popup holding a reference to `window.opener` — can post
 *    `{type:'vpay:redirect', url}` and steer the merchant's top-level
 *    navigation. The check is the whole security boundary of this file.
 * 3. **This side posts nothing.** There is therefore no target origin to
 *    get wrong, and in particular no `postMessage(…, '*')` — the mistake
 *    D8 names. The child learns the parent's origin from the CSP
 *    `frame-ancestors` vpay served it, not from a handshake.
 */
import type {
  EmbeddedCheckout,
  EmbeddedCheckoutCompleteEvent,
} from "./types.js";

/** The three messages vpay's checkout page sends its framer (D8). */
const MESSAGE_TYPES = {
  resize: "vpay:resize",
  complete: "vpay:complete",
  redirect: "vpay:redirect",
} as const;

/**
 * What the frame is allowed to do.
 *
 * `allow-same-origin` is required, not a relaxation: without it the frame
 * runs in an opaque origin, its `/v1/browser` requests carry `Origin: null`
 * and vpay's CORS layer refuses them. What the list *withholds* is the
 * point — `allow-top-navigation` is absent, so the page physically cannot
 * navigate the merchant's tab, which is why the redirect rail's hand-off is
 * a `vpay:redirect` message the parent acts on rather than something the
 * frame does itself.
 */
const FRAME_SANDBOX = "allow-scripts allow-same-origin allow-forms";

/**
 * Everything {@link createEmbeddedCheckout} needs, resolved by the caller.
 *
 * A plain record rather than a reference back to the `Stripe` object: this
 * module must not be able to reach a `client_secret` for a *payment intent*,
 * and taking only what it needs is how that stays true by construction.
 */
export interface EmbeddedCheckoutConfig {
  /** `pk_…`, encoded into the frame's query string. */
  publishableKey: string;
  /** The checkout app's origin, trailing slashes already stripped. */
  checkoutBaseUrl: string;
  /** `cs_…`, parsed out of the session's client secret. */
  sessionId: string;
  /** `cs_…_secret_…`, placed in the frame's URL fragment. */
  clientSecret: string;
  /** The merchant's completion callback, if any. */
  onComplete: ((event: EmbeddedCheckoutCompleteEvent) => void) | undefined;
}

/**
 * True only for an absolute `http:`/`https:` URL.
 *
 * Duplicated in spirit from `client.ts`'s `isSafeRedirectUrl` and applied
 * for a stronger reason: a `vpay:redirect` message asks the *parent* to
 * navigate, so a `javascript:` URL would execute in the **merchant's**
 * origin, on the merchant's own document, with the merchant's cookies. The
 * origin check above should already make that unreachable; this is the
 * second lock on the same door.
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

/** The `origin` half of a URL — what `event.origin` is compared against. */
export function originOf(url: string): string {
  return new URL(url).origin;
}

/**
 * Builds the frame's `src`: `{checkoutBaseUrl}/e/{cs_id}?key={pk}#{secret}`.
 *
 * The id is percent-encoded (it is interpolated into a path, and
 * `client.ts` encodes an intent id for the same reason); the key is
 * percent-encoded; the fragment is **verbatim**, because it is the exact
 * byte string the page hands back to `/v1/browser` and an encoding this
 * package applied would have to be undone by an agreement nobody wrote
 * down.
 */
export function embeddedFrameSrc(config: EmbeddedCheckoutConfig): string {
  const path = `${config.checkoutBaseUrl}/e/${encodeURIComponent(config.sessionId)}`;
  const query = `key=${encodeURIComponent(config.publishableKey)}`;
  return `${path}?${query}#${config.clientSecret}`;
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

/**
 * The `document` and `window` this handle drives, or a `TypeError`.
 *
 * `@vaam-apps/vpay-stripe-js` runs under Node in its own tests and in SSR frameworks
 * at a merchant, where neither binding exists. An embedded checkout with no
 * DOM cannot be silently degraded into anything useful, so it is refused
 * with the same shape as `loadStripe`'s argument checks.
 */
function requireBrowser(): { win: Window; doc: Document } {
  const win = typeof window === "undefined" ? undefined : window;
  if (win === undefined || typeof win.document !== "object") {
    throw new TypeError(
      "initEmbeddedCheckout: there is no browser window to mount into",
    );
  }
  return { win, doc: win.document };
}

/**
 * Builds one embedded-checkout handle: an `<iframe>`, a `message`
 * listener pinned to the checkout origin, and the three-verb lifecycle
 * `@stripe/stripe-js`'s `StripeEmbeddedCheckout` defines.
 *
 * The listener is attached here rather than in `mount`, so a page that
 * posts a `vpay:complete` between two `mount` calls is not lost, and it is
 * detached in `destroy` and nowhere else — `unmount` leaves the handle
 * usable, which is Stripe's own semantics.
 */
export function createEmbeddedCheckout(
  config: EmbeddedCheckoutConfig,
): EmbeddedCheckout {
  const { win, doc } = requireBrowser();
  const allowedOrigin = originOf(config.checkoutBaseUrl);

  const frame = doc.createElement("iframe");
  frame.setAttribute("src", embeddedFrameSrc(config));
  // Named, because a frame with no accessible name is announced as
  // "frame" and a payer using a screen reader cannot tell it from the
  // merchant's own chrome.
  frame.setAttribute("title", "Checkout");
  frame.setAttribute("sandbox", FRAME_SANDBOX);
  frame.style.border = "0";
  frame.style.width = "100%";
  frame.style.display = "block";
  // Zero until the page says otherwise. The page owns its own height — it
  // is the only side that knows how tall its content is — and sends
  // `vpay:resize` on every layout change. A page that never sends one
  // renders nothing, deliberately: an iframe silently stuck at a height
  // this package guessed would be a worse failure to diagnose.
  frame.style.height = "0px";

  let mounted = false;
  let destroyed = false;

  const onMessage = (event: MessageEvent): void => {
    // Rule 2. Everything below this line trusts the sender.
    if (event.origin !== allowedOrigin) {
      return;
    }
    // …and trusts *this* frame, not merely something else served from the
    // same checkout origin: two embedded checkouts on one merchant page
    // would otherwise resize and complete each other.
    if (event.source !== frame.contentWindow) {
      return;
    }
    const data: unknown = event.data;
    if (typeof data !== "object" || data === null) {
      return;
    }
    const message = data as Record<string, unknown>;
    switch (message["type"]) {
      case MESSAGE_TYPES.resize: {
        const height = message["height"];
        if (
          typeof height === "number" &&
          Number.isFinite(height) &&
          height >= 0
        ) {
          frame.style.height = `${height}px`;
        }
        return;
      }
      case MESSAGE_TYPES.complete: {
        const payload = parseCompleteEvent(message);
        if (payload !== undefined) {
          config.onComplete?.(payload);
        }
        return;
      }
      case MESSAGE_TYPES.redirect: {
        const url = message["url"];
        if (typeof url !== "string" || !isSafeTopLevelUrl(url)) {
          return;
        }
        // The parent navigates, because the frame may not (see
        // `FRAME_SANDBOX`). `window.top` rather than `window`: a merchant
        // may itself be framed, and sending only its own frame to the
        // rail's page would put Orange's page inside two frames, which its
        // own `frame-ancestors` refuses.
        win.top?.location.assign(url);
        return;
      }
      default:
        return;
    }
  };

  win.addEventListener("message", onMessage);

  return {
    mount(location: string | HTMLElement): void {
      if (destroyed) {
        throw new TypeError(
          "EmbeddedCheckout.mount: this checkout has been destroyed; create a new one",
        );
      }
      if (mounted) {
        throw new TypeError(
          "EmbeddedCheckout.mount: this checkout is already mounted; call unmount() first",
        );
      }
      const target =
        typeof location === "string" ? doc.querySelector(location) : location;
      if (target === null) {
        throw new TypeError(
          "EmbeddedCheckout.mount: no element matches the given selector",
        );
      }
      target.appendChild(frame);
      mounted = true;
    },
    unmount(): void {
      // Idempotent on purpose, and legal after `destroy`: a merchant's
      // cleanup path (a React effect teardown, say) should not have to
      // know which of the two already ran.
      frame.remove();
      mounted = false;
    },
    destroy(): void {
      if (destroyed) {
        return;
      }
      destroyed = true;
      frame.remove();
      mounted = false;
      win.removeEventListener("message", onMessage);
    },
  };
}
