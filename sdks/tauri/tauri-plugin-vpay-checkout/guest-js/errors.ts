/**
 * The error half of `@vaam-apps/vpay-tauri-checkout` — the TypeScript port of
 * `sdks/flutter/vpay_checkout_flutter/lib/src/errors.dart`, which is itself
 * the Dart port of `sdks/stripe-js/src/errors.ts`. Two rules hold everywhere
 * in this package and this module exists to enforce them in one place:
 *
 * 1. **Nothing rejects out of a poll.** {@link VpayCheckout.start} answers a
 *    {@link VpayCheckoutResult}, never a rejected promise — the same
 *    contract Stripe.js keeps, for the same reason: a payer-facing failure
 *    is data, not a control-flow surprise the caller must remember to catch.
 * 2. **No error message ever carries a credential.** Every message below is
 *    a fixed string. Nothing thrown by a host, and nothing read off a URL,
 *    is interpolated into one. The session URL this package is handed
 *    carries the session's `client_secret` in its fragment (D6), and every
 *    request the reused `@vaam-apps/vpay-stripe-js` client makes holds `key`
 *    and `client_secret` in its query string, so a message built from a
 *    thrown value (`err.message`, `String(err)`, `err.cause`) is a
 *    credential in whatever renders it.
 *
 * The one value any message here interpolates is a **checkout session id**
 * (`cs_…`), in {@link embeddedSessionNotSupportedError} — a public
 * identifier the merchant's own server minted and already holds, not a
 * secret, and the Flutter package interpolates it in exactly the same place
 * for exactly the same reason: without it the merchant cannot tell which of
 * several sessions was misconfigured. `errors.test.ts` pins both halves of
 * that: the id is present, and no secret ever is.
 *
 * The `type`/`code` vocabulary is vpay's server vocabulary rather than an
 * invented one, so `error.code` alone tells a merchant which side produced
 * the failure.
 */
import type { StripeError } from "@vaam-apps/vpay-stripe-js";

/**
 * vpay's error envelope (`vpay_api::error_envelope_with_param`), narrowed to
 * the members this package can populate.
 *
 * Deliberately an alias of `@vaam-apps/vpay-stripe-js`'s own `StripeError`
 * rather than a second declaration of the same four fields: this package
 * hands the errors that client produced straight through to its caller, so a
 * structurally-identical-but-separate interface would be two shapes that
 * could drift while both compiling.
 */
export type VpayError = StripeError;

/**
 * The five codes this package originates rather than reads off the wire.
 *
 * vpay's server has no such codes, so `error.code` alone distinguishes "the
 * checkout window could not finish" from "the API refused".
 */
export const VPAY_CLIENT_ERROR_CODES = {
  /**
   * A polling budget elapsed with the intent still not terminal, for a
   * caller that awaits one poll outcome directly. The
   * {@link CheckoutController} flow maps that case to a `pending` result
   * instead (D4 — a still-processing payment is not a failure), which is
   * why nothing in this package's own flow produces this code today. It is
   * declared because it is part of the vocabulary the Flutter package and
   * `sdks/stripe-js` already publish, and a consumer switching on
   * `error.code` should see one list, not three.
   */
  pollingTimeout: "polling_timeout",
  /**
   * A response that is not the documented envelope — a proxy's HTML 502,
   * or (the case this package produces) a 200 that described a *different*
   * payment intent from the one that was asked about.
   */
  unexpectedResponse: "unexpected_response",
  /** D2: an `embedded` session was named at the pre-flight. Refused before any window opens. */
  embeddedSessionNotSupported: "embedded_session_not_supported",
  /**
   * The checkout window never opened, or closed without ever reporting
   * which of its two signals occurred — a browser that refused the popup,
   * an Android `Activity` that would not start, a host that dropped the
   * event. Distinct from every status this package reads off an intent: it
   * means the *window* failed, so nothing was observed either way.
   */
  platformWindowFailed: "platform_window_failed",
  /** An integration mistake caught before any request is made. */
  invalidRequest: "invalid_request",
} as const;

/** Builds a {@link VpayError}, omitting the members that have no value. */
function vpayError(
  type: string,
  code: string | undefined,
  message: string,
  param?: string,
): VpayError {
  const error: VpayError = { type, message };
  if (code !== undefined) {
    error.code = code;
  }
  if (param !== undefined) {
    error.param = param;
  }
  return error;
}

/**
 * The session URL carried no fragment, so there is no session
 * `client_secret` to pre-flight with.
 *
 * The offending URL is **not** quoted: a URL of this shape carries the
 * secret in the very fragment whose absence is being reported, and a
 * caller that passed a *nearly* right URL would have the secret echoed into
 * whatever renders `error.message`.
 */
export function invalidSessionUrlError(): VpayError {
  return vpayError(
    "invalid_request_error",
    VPAY_CLIENT_ERROR_CODES.invalidRequest,
    'sessionUrl must carry a client secret in its fragment (vpay_api renders "{base}/c/{id}?key=…#{client_secret}").',
    "sessionUrl",
  );
}

/**
 * D2: an `embedded` checkout session was named at the pre-flight. A typed
 * error, refused before any window opens — never a window that opens and
 * then refuses itself.
 */
export function embeddedSessionNotSupportedError(sessionId: string): VpayError {
  return vpayError(
    "invalid_request_error",
    VPAY_CLIENT_ERROR_CODES.embeddedSessionNotSupported,
    `Checkout session ${sessionId} is ui_mode "embedded"; @vaam-apps/vpay-tauri-checkout only opens a hosted session in the payer's browser.`,
  );
}

/**
 * The checkout window could not be opened, or ended without reporting an
 * outcome.
 *
 * A **fixed** message on purpose. The value this replaces is whatever the
 * host threw: a Tauri command rejection is a string the Rust side built and
 * can quote the URL it failed to parse, and a refused `window.open` is this
 * package's own `Error`. That URL carries the session's `client_secret` in
 * its fragment (D6), so nothing from the thrown value is interpolated here.
 */
export function platformWindowError(): VpayError {
  return vpayError(
    "api_error",
    VPAY_CLIENT_ERROR_CODES.platformWindowFailed,
    "The checkout window could not be opened, or closed without reporting an outcome.",
  );
}

/**
 * A response that is not vpay's documented envelope. Carries the status,
 * never the body.
 */
export function unexpectedResponseError(status: number): VpayError {
  return vpayError(
    "api_error",
    VPAY_CLIENT_ERROR_CODES.unexpectedResponse,
    `The vpay API returned an unexpected response (HTTP ${status}).`,
  );
}

// There is deliberately no `pollingTimeoutError()` factory to go with
// `VPAY_CLIENT_ERROR_CODES.pollingTimeout`. Nothing in this package's flow
// produces that code — an elapsed budget is a `pending` result, not an
// error (D4) — and a factory nobody calls would read as a path that exists.
