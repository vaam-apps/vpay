/**
 * The public entry point: `new VpayCheckout({…}).start(sessionUrl)` — parse,
 * pre-flight, show, watch, resolve, answer.
 *
 * The port of
 * `sdks/flutter/vpay_checkout_flutter/lib/src/vpay_checkout.dart`. It reuses
 * `@vaam-apps/vpay-stripe-js` for both reads rather than porting a second
 * HTTP client, which is why there is no `browser-client.ts` here.
 */
import { loadStripe, type Stripe } from "@vaam-apps/vpay-stripe-js";

import {
  CheckoutController,
  type CheckoutPreflightReady,
} from "./controller.js";
import {
  invalidSessionUrlError,
  platformWindowError,
  type VpayError,
} from "./errors.js";
import { defaultHost } from "./host.js";
import {
  systemClock,
  type CheckoutHost,
  type CheckoutWindowEvent,
  type VpayCheckoutOptions,
  type VpayCheckoutResult,
} from "./types.js";

/** The separator `vpay_core::ids::client_secret` joins an id and its suffix with. */
const SECRET_SEPARATOR = "_secret_";

/**
 * `{base}/c/{cs_id}?key={pk}#{cs_secret}` (D6), split into the one thing
 * this package needs out of it: the fragment.
 *
 * The query's `key` is deliberately **not** re-parsed — {@link VpayCheckout}
 * already holds the publishable key it was constructed with, and deriving a
 * second copy from the URL would be two things that could disagree.
 */
interface ParsedSessionUrl {
  clientSecret: string;
  /**
   * A label for an `unresolved` result when the pre-flight itself never
   * ran — best-effort only. `retrieveCheckoutSession` is what actually
   * validates the secret's shape; this is not a second validator, only a
   * display string.
   */
  bestEffortSessionId: string;
}

function parseSessionUrl(url: string): ParsedSessionUrl | null {
  const hash = url.indexOf("#");
  if (hash === -1 || hash === url.length - 1) {
    return null;
  }
  const clientSecret = url.slice(hash + 1);
  const separator = clientSecret.indexOf(SECRET_SEPARATOR);
  return {
    clientSecret,
    bestEffortSessionId:
      separator === -1 ? "" : clientSecret.slice(0, separator),
  };
}

/** True only for an absolute `https:` URL — D6's rule, before `loadStripe` sees it. */
function isHttps(value: string): boolean {
  try {
    return new URL(value).protocol === "https:";
  } catch {
    return false;
  }
}

/**
 * vpay's Tauri checkout entry point. Constructed once per deployment (base
 * URL + publishable key), the way `sdks/stripe-js`'s `loadStripe` answers a
 * `Stripe` bound to one.
 */
export class VpayCheckout {
  readonly #stripe: Promise<Stripe>;
  readonly #controller: Promise<CheckoutController>;
  readonly #host: CheckoutHost;
  readonly #allowInsecureBaseUrl: boolean;

  /**
   * # Errors
   *
   * Throws a `TypeError` — the only thing in this package that throws —
   * for the three integration mistakes visible on a merchant's first page
   * load: a blank publishable key, a blank base URL, and a non-`https`
   * base URL without the named opt-in (D6). `start` keeps its
   * never-rejects contract precisely because these were checked once,
   * here.
   *
   * None of the three messages quotes the offending value. A base URL is
   * not itself a secret, but this package has exactly one rule about
   * interpolation and an exception would be the thing the next reader
   * copies.
   */
  constructor(options: VpayCheckoutOptions) {
    if (
      typeof options.publishableKey !== "string" ||
      options.publishableKey.trim().length === 0
    ) {
      throw new TypeError(
        "VpayCheckout: options.publishableKey must be a non-empty string",
      );
    }
    if (
      typeof options.baseUrl !== "string" ||
      options.baseUrl.trim().length === 0
    ) {
      throw new TypeError(
        "VpayCheckout: options.baseUrl must be a non-empty string",
      );
    }
    this.#allowInsecureBaseUrl = options.allowInsecureBaseUrl ?? false;
    if (!this.#allowInsecureBaseUrl && !isHttps(options.baseUrl.trim())) {
      // D6, and `loadStripe` will not do it: it accepts `http://` happily.
      // The demo stack is the only deployment that needs one, and it has
      // to say so by name rather than by having a scheme inferred for it.
      throw new TypeError(
        "VpayCheckout: options.baseUrl must be an absolute https URL; pass allowInsecureBaseUrl: true for a local demo stack",
      );
    }

    this.#stripe = loadStripe(options.publishableKey, {
      baseUrl: options.baseUrl,
      fetch: options.fetch,
    });
    // `loadStripe` cannot reject once the three checks above have passed —
    // its own validation is a subset of theirs — but the promise is held
    // across ticks, so an unconsumed rejection would be an unhandled one
    // at the process level. This marks it handled without swallowing it:
    // the `await` in `start` still sees the rejection.
    void this.#stripe.catch(() => undefined);

    const clock = options.clock ?? systemClock;
    this.#controller = this.#stripe.then(
      (stripe) => new CheckoutController({ stripe, clock }),
    );
    void this.#controller.catch(() => undefined);
    this.#host = options.host ?? defaultHost();
  }

  /**
   * The whole flow. **Never rejects**: every failure is a
   * `kind: "unresolved"` carrying a typed {@link VpayError}.
   *
   * Never reads the outcome off `sessionUrl`, off a stop URL, or off
   * anything a window navigated to (D1) — only the controller's poll of
   * `retrievePaymentIntent` decides.
   */
  async start(sessionUrl: string): Promise<VpayCheckoutResult> {
    const parsed = parseSessionUrl(sessionUrl);
    if (parsed === null) {
      return unresolved("", "", invalidSessionUrlError());
    }

    let controller: CheckoutController;
    try {
      controller = await this.#controller;
    } catch {
      // Unreachable given the constructor's checks; mapped rather than
      // rethrown so the never-rejects contract holds even if a future
      // `loadStripe` grows a check this constructor does not mirror.
      return unresolved(
        parsed.bestEffortSessionId,
        "",
        invalidSessionUrlError(),
      );
    }

    const preflight = await controller.preflight(parsed.clientSecret);
    if (!preflight.ok) {
      return unresolved(parsed.bestEffortSessionId, "", preflight.error);
    }
    return this.#showAndResolve(sessionUrl, preflight.ready, controller);
  }

  /** Closes the checkout window if one is open; a no-op otherwise. */
  async dismiss(): Promise<void> {
    await this.#host.dismiss();
  }

  /**
   * D5/D7: the host shows the URL and reports one of two signals.
   *
   * `onEvent` is wired **before** `show` is awaited, not after. On a Tauri
   * host the event cannot arrive before `invoke` resolves; on the web host
   * `show` completes with a popup already open, and a payer who closes it
   * instantly would have the one event this flow waits for reported into a
   * callback that did not exist yet.
   */
  async #showAndResolve(
    sessionUrl: string,
    ready: CheckoutPreflightReady,
    controller: CheckoutController,
  ): Promise<VpayCheckoutResult> {
    let reported = false;
    let deliver: ((event: CheckoutWindowEvent) => void) | undefined;
    const settled = new Promise<CheckoutWindowEvent>((resolve) => {
      deliver = resolve;
    });
    const onEvent = (event: CheckoutWindowEvent): void => {
      // A host that reports twice is a host bug, and the second report
      // would start a second poll loop against the same intent. Ignored
      // here, so no host has to be trusted for it.
      if (reported) {
        return;
      }
      reported = true;
      deliver?.(event);
    };

    try {
      await this.#host.show(
        {
          url: sessionUrl,
          stopUrls: ready.stopUrls,
          allowInsecureUrl: this.#allowInsecureBaseUrl,
        },
        onEvent,
      );
    } catch {
      // A popup the browser refused, an Android Activity that would not
      // start, a Tauri command that rejected: the window never opened, so
      // no money can have moved. A typed result, and **never** the thrown
      // value's own message — on every host it can quote the URL, and that
      // URL carries the session secret in its fragment (D6).
      return unresolved(
        ready.sessionId,
        ready.paymentIntentId,
        platformWindowError(),
      );
    }

    const event = await settled;
    return event.outcome === "stopUrlReached"
      ? controller.resolveAfterStopUrlReached(ready)
      : controller.resolveAfterDismissal(ready);
  }
}

/** The one `unresolved` shape, built in one place so the three call sites cannot drift. */
function unresolved(
  sessionId: string,
  paymentIntentId: string,
  error: VpayError,
): VpayCheckoutResult {
  return { kind: "unresolved", sessionId, paymentIntentId, error };
}
