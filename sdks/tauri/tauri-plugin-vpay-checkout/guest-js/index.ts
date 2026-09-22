/**
 * `@vaam-apps/vpay-tauri-checkout` — the guest-JS half of
 * `tauri-plugin-vpay-checkout`.
 *
 * One state machine, three hosts. Inside a Tauri v2 app the checkout page
 * opens in the payer's **own browser** — a partial Custom Tab on Android, a
 * detented `SFSafariViewController` on iOS, the default browser on desktop
 * — and outside Tauri the same API falls back to a `window.open` popup with
 * the origin-and-source-pinned `vpay:complete` protocol
 * `@vaam-apps/vpay-stripe-js` already speaks.
 *
 * The rule that shapes everything here: **no host decides anything about
 * money.** A host reports one of two things — the payer dismissed the
 * window, or an incoming deep link matched a stop URL — and the outcome
 * comes from polling `GET /v1/browser/payment_intents/{id}` afterwards,
 * never from a URL, a message payload or an event (D1). And a result from
 * `start` is a UI fact: a merchant's server fulfils from the
 * signature-verified webhook, not from this.
 *
 * See README.md for what has and has not actually been run.
 */
export { VpayCheckout } from "./checkout.js";

export {
  CheckoutController,
  DEFAULT_DISMISSAL_POLL_INTERVAL_MS,
  DEFAULT_DISMISSAL_POLL_TIMEOUT_MS,
  DEFAULT_STOP_URL_POLL_INTERVAL_MS,
  DEFAULT_STOP_URL_POLL_TIMEOUT_MS,
} from "./controller.js";
export type {
  CheckoutPreflight,
  CheckoutPreflightReady,
  PollBudget,
} from "./controller.js";

export { VPAY_CLIENT_ERROR_CODES } from "./errors.js";
export type { VpayError } from "./errors.js";

export { defaultHost } from "./host.js";
export { tauriHost } from "./host-tauri.js";
export type { TauriHostDeps } from "./host-tauri.js";
export { webHost } from "./host-web.js";

export { redacted } from "./redaction.js";

export { StopUrlSpec } from "./stop-url.js";

export { systemClock } from "./types.js";
export type {
  CheckoutHost,
  CheckoutStopUrl,
  CheckoutWindowEvent,
  CheckoutWindowOutcome,
  ShowCheckoutRequest,
  VpayCheckoutOptions,
  VpayCheckoutResult,
  VpayClock,
} from "./types.js";
