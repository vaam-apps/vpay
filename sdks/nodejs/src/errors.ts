/**
 * Error taxonomy for `@vpay/sdk`, per docs/flows/merchant-auth.md's
 * "Errors" section and the token-endpoint error shape it documents.
 *
 * Every error extends {@link VpayError} so callers can `catch (err) { if
 * (err instanceof VpayError) ... }` once and still narrow further by
 * `err.name` or an `instanceof` check on a specific subclass.
 */

/** Base class for every error this SDK throws. */
export class VpayError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = new.target.name;
  }
}

/**
 * A non-2xx response from a `/v1` resource route, shaped like
 * `vpay_api::error_envelope`: `{ "error": { "type", "code", "message",
 * "param" } }`.
 */
export class VpayApiError extends VpayError {
  /** The HTTP status code of the response. */
  readonly status: number;
  /** Stripe-shaped error type, e.g. `invalid_request_error`. */
  readonly type: string;
  /** Machine-readable error code, when the server sent one. */
  readonly code: string | undefined;
  /** The offending request parameter, when the server named one. */
  readonly param: string | undefined;

  constructor(
    status: number,
    body: {
      type: string;
      code?: string | undefined;
      message: string;
      param?: string | undefined;
    },
  ) {
    super(body.message);
    this.status = status;
    this.type = body.type;
    this.code = body.code;
    this.param = body.param;
  }
}

/**
 * A `400`/`401` from the OAuth2 token endpoint:
 * `{ "error": "invalid_client", "error_description": "…" }`
 * (`TokenErrorResponse` in `authkestra-op`). Never retried automatically —
 * see docs/flows/merchant-auth.md, "Re-authentication".
 */
export class VpayAuthError extends VpayError {
  /** The OAuth2 `error` code, e.g. `invalid_client`. */
  readonly error: string;
  /** Human-readable detail, when the server sent one. */
  readonly errorDescription: string | undefined;

  constructor(error: string, errorDescription: string | undefined) {
    super(errorDescription ? `${error}: ${errorDescription}` : error);
    this.error = error;
    this.errorDescription = errorDescription;
  }
}

/**
 * A response that is not the Stripe-shaped error envelope this SDK expects —
 * a proxy's HTML 502, an empty body, or JSON with an unrecognised shape.
 * Carries a bounded prefix of the body so the cause is diagnosable without
 * risking an unbounded string in logs.
 */
export class VpayUnexpectedResponseError extends VpayError {
  /** The HTTP status code of the response. */
  readonly status: number;
  /** At most the first 500 **bytes** of the response body, decoded as UTF-8. */
  readonly bodyPrefix: string;

  constructor(status: number, bodyPrefix: string) {
    super(
      `unexpected response (status ${status}): ${bodyPrefix.slice(0, 200)}`,
    );
    this.status = status;
    this.bodyPrefix = bodyPrefix;
  }
}

/** A transport-level failure — DNS, TLS, connection refused, timeout. */
export class VpayTransportError extends VpayError {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
  }
}

/** The SDK was configured incorrectly — caught before any request is sent. */
export class VpayConfigError extends VpayError {}

/** A webhook signature failed verification — see {@link verifyWebhook}. */
export class WebhookSignatureError extends VpayError {}

const BODY_PREFIX_LIMIT_BYTES = 500;

/**
 * Truncates a response body to a bounded prefix for
 * {@link VpayUnexpectedResponseError}.
 *
 * The bound is on **bytes**, not `String.prototype.length`: a body of
 * multi-byte characters (a UTF-8 error page, a JSON body of CJK text) is up
 * to four times its code-unit count on the wire, and the point of the bound
 * is to keep an unbounded response out of a log line. `TextDecoder` is used
 * in streaming mode and never flushed, so a multi-byte character straddling
 * the cut is dropped rather than emitted as a replacement character.
 */
export function boundedBodyPrefix(body: string): string {
  const bytes = Buffer.from(body, "utf8");
  if (bytes.length <= BODY_PREFIX_LIMIT_BYTES) {
    return body;
  }
  return new TextDecoder("utf-8").decode(
    bytes.subarray(0, BODY_PREFIX_LIMIT_BYTES),
    { stream: true },
  );
}
