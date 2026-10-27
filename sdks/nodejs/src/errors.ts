/**
 * Error taxonomy for `@vaam-apps/vpay-sdk`, per docs/flows/merchant-auth.md's
 * "Errors" section.
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
 * vpay refused the call before any handler saw it: the answer is not sealed.
 *
 * That is the signed transport's one coarse refusal — a `401` whose body is
 * the same for **every** cause (a key nobody registered, a disabled client, a
 * replayed message, the wrong audience, a request that was not signed at all),
 * so that nothing in it helps someone guessing. The cause is in vpay's logs
 * and in the operator's registration of this merchant's key, not here;
 * `docs/flows/merchant-auth.md`, "When the answer is a plain 401", lists what
 * to check, in order. A proxy's own error page lands here too: it is not
 * sealed either.
 *
 * Never retried automatically: the same call would be refused for the same
 * reason, and a replay refusal in particular must not be answered with another
 * replay.
 */
export class VpayUnsignedResponseError extends VpayError {
  /** The HTTP status the unsigned answer came with. */
  readonly status: number;

  constructor(status: number) {
    super(
      `vpay answered ${status} without a signature: the call was refused before it was handled ` +
        "(unregistered or disabled key, replayed message, wrong audience, or not vpay at all)",
    );
    this.status = status;
  }
}

/**
 * The answer claims to be sealed and does not verify under either pinned
 * server key, or does not answer this request. The response is not trusted and
 * nothing in it is read. Almost always a stale `serverKeys` after vpay rotated
 * its response key without a `next` having been pinned first, or a client
 * pointed at the wrong deployment.
 */
export class VpayUnverifiedResponseError extends VpayError {
  constructor() {
    super(
      "the answer does not verify under the pinned server keys (serverKeys.current / serverKeys.next)",
    );
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

/**
 * A request parameter this SDK refuses to put on the wire, caught before
 * anything is sent — no token is minted and no request is built. The twin
 * of `sdks/rust`'s `Error::InvalidParams { param, message }`, and its
 * `message` is that variant's `Display` text: `invalid request parameter
 * {param}: {detail}`.
 *
 * Thrown for an amount outside `0..=2^53-1`, a value the form encoder cannot
 * put on the wire exactly (a non-safe-integer number, a `Date`, a `Map`, a
 * class instance), a path `id` that is empty or only dots, and a URL beside
 * `outOfBand` on `invoices.pay`.
 *
 * **Not a `TypeError`, since 2026-10-21.** Every one of these was a bare
 * `TypeError` until then, which the one narrowing this package documents —
 * `instanceof VpayError` — missed. It cannot be both: a class has one
 * superclass, and making `VpayError` extend `TypeError` would make every
 * error this package throws a `TypeError`, a network failure included.
 */
export class VpayInvalidParamsError extends VpayError {
  /**
   * The offending parameter's wire name — `amount`, `unit_amount`, `id`,
   * `success_url`, or a nested form path such as `metadata[order_id]`.
   */
  readonly param: string;

  constructor(param: string, detail: string) {
    super(`invalid request parameter ${param}: ${detail}`);
    this.param = param;
  }
}

/** A transport-level failure — DNS, TLS, connection refused, timeout. */
export class VpayTransportError extends VpayError {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
  }
}

/**
 * An answer's body exceeded {@link MAX_RESPONSE_BYTES}. Reading stopped at the
 * bound and the connection was destroyed; nothing of the body was kept or
 * opened. The honest server never sends one this large (a page of 100 objects
 * is a few hundred kilobytes), so this is a misbehaving peer or proxy, and an
 * unbounded read would let it make this process hold whatever it liked.
 *
 * A {@link VpayTransportError}, so a caller that handles transport failures
 * already handles this one.
 */
export class VpayResponseTooLargeError extends VpayTransportError {
  /** The bound that was exceeded, in bytes. */
  readonly limit: number;
  constructor(limit: number) {
    super(`the response body exceeded ${limit} bytes and was not read further`);
    this.limit = limit;
  }
}

/**
 * The SDK was configured incorrectly — caught before any request is sent.
 *
 * Almost always at construction (a missing `clientId`, an unreadable or
 * non-Ed25519 `privateKey`, a `serverKeys` entry that is not 32 bytes). The
 * one exception is `@vaam-apps/vpay-sdk/stripe`'s host binding: an HTTP client
 * asked to seal a request addressed somewhere other than its `baseUrl` throws
 * this *per request*, because that is when the destination becomes knowable.
 * Still before anything is sent — nothing is signed.
 *
 * Also thrown when the signing callback fails or returns a signature that does
 * not verify under the registered public key: refused here, with the cause
 * attached, instead of surfacing at vpay as the coarse 401.
 */
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
