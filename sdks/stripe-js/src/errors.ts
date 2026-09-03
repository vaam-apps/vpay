/**
 * The error half of `@vpay/stripe-js`.
 *
 * Two rules hold everywhere in this package and are what this module exists
 * to enforce in one place:
 *
 * 1. **Nothing rejects.** Every method on {@link import('./types.js').Stripe}
 *    resolves with a `PaymentIntentResult`, so a merchant's `await` never
 *    needs a `try`/`catch`. That is Stripe.js's own contract and it is the
 *    reason a payer never sees an unhandled rejection in the console.
 * 2. **No error message ever carries a credential.** The request URL holds
 *    both `key` and `client_secret` in its query string, so a message built
 *    from a `fetch` failure (`err.message`, `String(err)`, `err.cause`) can
 *    leak the secret into a console line, a Sentry breadcrumb or a
 *    `window.onerror` report. Every message below is a **fixed string** —
 *    never interpolated from a thrown value or a URL.
 *
 * The `type`/`code` vocabulary is vpay's server vocabulary, not an invented
 * one: `invalid_request_error` + `invalid_request` is what
 * `vpay_core::Category::InvalidRequest` renders, and `resource_missing` is
 * what `Category::NotFound` renders. Codes the *client* originates (there
 * are three) are the ones the server can never send, so a merchant reading
 * `error.code` can always tell which side produced the failure.
 */

/**
 * Stripe.js's `StripeError`, narrowed to the members vpay can populate.
 *
 * `type` is `string` rather than Stripe's closed `StripeErrorType` union so
 * that a `type` a future vpay version introduces still deserialises here
 * instead of being a compile error at the merchant. See `src/compat.test.ts`
 * for what that costs in assignability, pinned at the type level.
 *
 * Every optional member is written `?: T | undefined`, not `?: T`, so a
 * consumer compiling with `exactOptionalPropertyTypes` (as this repo does)
 * can pass through a value that is legitimately `undefined`.
 */
export interface StripeError {
  /** e.g. `invalid_request_error`, `api_error`, `api_connection_error`. */
  type: string;
  /** Machine-readable code, e.g. `resource_missing`. */
  code?: string | undefined;
  /** Human-readable detail. Never contains a `client_secret` or a publishable key. */
  message?: string | undefined;
  /** The offending request parameter, when one can be named. */
  param?: string | undefined;
}

/**
 * The four codes this package originates rather than reads off the wire.
 *
 * vpay's server has no such codes, so `error.code` alone distinguishes "the
 * browser could not finish" from "the API refused".
 */
export const CLIENT_ERROR_CODES = {
  /** {@link import('./types.js').Stripe.waitForPaymentIntent} ran out of time. */
  pollingTimeout: "polling_timeout",
  /** A redirect was required but there is no `window` to navigate. */
  redirectUnavailable: "redirect_unavailable",
  /** A response that is not the documented envelope — a proxy's HTML 502, say. */
  unexpectedResponse: "unexpected_response",
  /**
   * `next_action.redirect_to_url.url` was not an absolute `http:`/`https:`
   * URL, so it was refused rather than handed to `location.assign`.
   */
  invalidRedirect: "invalid_redirect",
} as const;

/** Builds a `{ error }` result body. Not exported from the package. */
export function stripeError(
  type: string,
  code: string | undefined,
  message: string,
  param?: string,
): StripeError {
  const error: StripeError = { type, message };
  if (code !== undefined) {
    error.code = code;
  }
  if (param !== undefined) {
    error.param = param;
  }
  return error;
}

/**
 * A `fetch` that never reached the API: DNS, TLS, connection refused, CORS,
 * an aborted navigation.
 *
 * Deliberately carries no `code` and a fixed message. The thrown value is
 * *not* interpolated: in a browser a `fetch` rejection is
 * `TypeError: Failed to fetch`, but its `cause` and some engines' messages
 * include the request URL — which is exactly the string holding the
 * `client_secret`.
 */
export function connectionError(): StripeError {
  return stripeError(
    "api_connection_error",
    undefined,
    "Could not reach the vpay API.",
  );
}

/** A response that is not vpay's documented error envelope. Carries the status, never the body. */
export function unexpectedResponseError(status: number): StripeError {
  return stripeError(
    "api_error",
    CLIENT_ERROR_CODES.unexpectedResponse,
    `The vpay API returned an unexpected response (HTTP ${status}).`,
  );
}

/**
 * Reads `{ "error": { "type", "code", "message", "param" } }` —
 * `vpay_api::error_envelope_with_param` — out of a parsed response body.
 *
 * `type` is the only member the renderer always writes that this parser
 * insists on; `code` is always written too, but a version skew that dropped
 * it should surface as a typed error rather than as
 * `unexpected_response`. Returns `undefined` for anything else, which the
 * caller turns into {@link unexpectedResponseError}.
 */
export function parseErrorEnvelope(body: unknown): StripeError | undefined {
  if (typeof body !== "object" || body === null || !("error" in body)) {
    return undefined;
  }
  const raw: unknown = (body as { error: unknown }).error;
  if (typeof raw !== "object" || raw === null) {
    return undefined;
  }
  const fields = raw as Record<string, unknown>;
  const type = fields["type"];
  if (typeof type !== "string") {
    return undefined;
  }
  const error: StripeError = { type };
  for (const key of ["code", "message", "param"] as const) {
    const value = fields[key];
    if (typeof value === "string") {
      error[key] = value;
    }
  }
  return error;
}
