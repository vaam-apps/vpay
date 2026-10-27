/**
 * `@vaam-apps/vpay-sdk/stripe` — an `httpClient` for the **official Stripe
 * Node SDK**, so `stripe-node` can talk to vpay's `/v1` API.
 *
 * vpay never accepts an API key or a token ([ADR-0010](../../../docs/adr/0010-merchant-auth-private-key-jwt.md),
 * superseded by the signed transport): every `/v1` call is a COSE_Sign1 message
 * signed with the merchant's Ed25519 key, and every answer is one signed by
 * vpay (`docs/signed-transport.md`). The request is not a header on an
 * otherwise ordinary call — the **body is the message** — so stripe-node's
 * `authenticator` seam, which can only add headers, cannot carry it. The seam
 * that can is `httpClient`: stripe-node hands it the finished request
 * (`host`, `port`, `path`, `method`, `headers`, `requestData`, `protocol`,
 * `timeout`) and takes back something that looks like a response. This module
 * is that client. It seals the request, sends it, opens the answer, and returns
 * the verified JSON as the response body, so everything above it — resource
 * methods, typed results, `Stripe.errors.*` — behaves as it does against Stripe:
 *
 * ```js
 * import Stripe from "stripe";
 * import { createStripeHttpClient } from "@vaam-apps/vpay-sdk/stripe";
 *
 * const stripe = new Stripe("unused", {      // a non-empty string; never sent
 *   httpClient: createStripeHttpClient({
 *     baseUrl: "http://localhost:8080",
 *     clientId: "acme-cameroon",
 *     privateKey: readFileSync("./merchant.seed", "utf8"),
 *     serverKeys: { current: process.env.VPAY_SERVER_KEY_CURRENT },
 *   }),
 *   host: "localhost", port: "8080", protocol: "http",
 * });
 * ```
 *
 * The `host`/`port`/`protocol` in that snippet are **not optional garnish** —
 * see {@link createStripeHttpClient}'s "Where a request may be sent".
 *
 * This module deliberately imports **nothing** from `stripe`. `stripe` is an
 * optional peer dependency, and the client is typed structurally, so
 * `@vaam-apps/vpay-sdk` type-checks and builds with `stripe` absent.
 * `src/stripe-http-client.test.ts` pins the structure against the real
 * `Stripe.HttpClientInterface`, so a divergence in a future `stripe` release
 * fails the build rather than the merchant's deployment.
 *
 * See the README's "Using the official Stripe SDK" section for what does and
 * does not carry over from real Stripe.
 */
import { randomUUID } from "node:crypto";
import {
  VpayConfigError,
  VpayInvalidParamsError,
  VpayResponseTooLargeError,
  VpayUnsignedResponseError,
  VpayUnverifiedResponseError,
} from "./errors.js";
import { performSealedCall } from "./sealed-call.js";
import {
  resolveMerchantAuth,
  type MerchantAuthOptions,
  type ResolvedMerchantAuth,
} from "./sealer.js";

/**
 * Configuration for {@link createStripeHttpClient}.
 *
 * **The same options `VpayClient` takes** ({@link MerchantAuthOptions}): both
 * entry points seal through the same {@link resolveMerchantAuth}, so a knob one
 * of them offers and the other does not would be a bug rather than a feature.
 * `timeoutMs` is the one that narrows: stripe-node passes its own `timeout` to
 * every request, and that is what governs the call; `timeoutMs` here is the
 * fallback for a request that arrives without one.
 */
export interface StripeHttpClientOptions extends MerchantAuthOptions {
  /** Default 30000. Used only when stripe-node passes no `timeout` of its own. */
  timeoutMs?: number | undefined;
}

/** The request headers stripe-node passes (`RequestHeaders`). */
export type StripeRequestHeaders = Record<
  string,
  string | number | string[] | undefined
>;

/**
 * What stripe-node reads off the object `makeRequest` resolves with
 * (`HttpClientResponseInterface`).
 */
export interface VpayStripeHttpResponse {
  getStatusCode(): number;
  getHeaders(): Record<string, string | string[]>;
  getRawResponse(): unknown;
  toStream(streamCompleteCallback: () => void): never;
  toJSON(): Promise<unknown>;
}

/**
 * What {@link createStripeHttpClient} returns: assignable to
 * `Stripe.StripeConfig["httpClient"]` (`HttpClientInterface`).
 */
export interface VpayStripeHttpClient {
  getClientName(): string;
  makeRequest(
    host: string,
    port: string,
    path: string,
    method: string,
    headers: StripeRequestHeaders,
    requestData: string,
    protocol: string,
    timeout: number,
  ): Promise<VpayStripeHttpResponse>;
}

/** The origin a client is bound to, normalised for comparison. */
interface BoundOrigin {
  /** Lower-cased, without the brackets an IPv6 literal wears in a URL. */
  host: string;
  /** Always explicit: the URL's port, or the scheme's default. */
  port: string;
  /** Lower-cased, without the trailing colon `URL.protocol` carries. */
  protocol: string;
}

function defaultPortFor(protocol: string): string {
  return protocol === "http" ? "80" : "443";
}

/**
 * `api.vpay.example`, `API.Vpay.Example.` and `[::1]` all have to compare
 * equal to the same configured host, because a merchant writes `baseUrl` and
 * stripe-node's `host` by hand and nothing makes them agree on case or on
 * bracket notation. Everything else about the value is left alone: this is a
 * comparison, not a validator.
 */
function normaliseHost(host: string): string {
  const lowered = host.trim().toLowerCase();
  const unbracketed =
    lowered.startsWith("[") && lowered.endsWith("]")
      ? lowered.slice(1, -1)
      : lowered;
  return unbracketed.endsWith(".") ? unbracketed.slice(0, -1) : unbracketed;
}

function normaliseProtocol(protocol: string): string {
  const lowered = protocol.trim().toLowerCase();
  return lowered.endsWith(":") ? lowered.slice(0, -1) : lowered;
}

function parseBoundOrigin(baseUrl: string): BoundOrigin {
  let url: URL;
  try {
    url = new URL(baseUrl);
  } catch (err) {
    throw new VpayConfigError(`baseUrl is not an absolute URL: ${baseUrl}`, {
      cause: err,
    });
  }
  const protocol = normaliseProtocol(url.protocol);
  return {
    host: normaliseHost(url.hostname),
    port: url.port === "" ? defaultPortFor(protocol) : url.port,
    protocol,
  };
}

function describeOrigin(origin: BoundOrigin): string {
  return `${origin.protocol}://${origin.host}:${origin.port}`;
}

/**
 * Refuses to seal a request addressed somewhere other than {@link BoundOrigin}.
 *
 * See {@link createStripeHttpClient}'s "Where a request may be sent" for why
 * this is a refusal and not a warning.
 */
function assertRequestIsBound(
  host: string,
  port: string,
  protocol: string,
  bound: BoundOrigin,
): void {
  const effectiveProtocol = normaliseProtocol(protocol);
  const effectiveHost = normaliseHost(host);
  const effectivePort =
    port === undefined || port === ""
      ? defaultPortFor(effectiveProtocol)
      : String(port);

  if (
    effectiveHost === bound.host &&
    effectivePort === bound.port &&
    effectiveProtocol === bound.protocol
  ) {
    return;
  }
  throw new VpayConfigError(
    `refusing to seal a request to ${describeOrigin({ host: effectiveHost, port: effectivePort, protocol: effectiveProtocol })}: ` +
      `this client signs for ${describeOrigin(bound)} (its baseUrl) and will not send a ` +
      `signed request anywhere else. Set stripe-node's host, port and protocol to match baseUrl — ` +
      `with them omitted stripe-node addresses api.stripe.com:443.`,
  );
}

/** The `Idempotency-Key` stripe-node set, if it set one (header names are case-insensitive). */
function idempotencyKeyOf(headers: StripeRequestHeaders): string | undefined {
  for (const [name, value] of Object.entries(headers)) {
    if (name.toLowerCase() === "idempotency-key" && value !== undefined) {
      return Array.isArray(value) ? value[0] : String(value);
    }
  }
  return undefined;
}

class SealedResponse implements VpayStripeHttpResponse {
  readonly #status: number;
  readonly #text: string;
  readonly #headers: Record<string, string>;
  /** What stripe-node annotates and exposes as `lastResponse`. */
  readonly #raw: { statusCode: number; headers: Record<string, string> };

  constructor(status: number, text: string, headers: Record<string, string>) {
    this.#status = status;
    this.#text = text;
    this.#headers = headers;
    this.#raw = { statusCode: status, headers };
  }

  getStatusCode(): number {
    return this.#status;
  }

  getHeaders(): Record<string, string | string[]> {
    // The body is the verified JSON payload. The headers are the response's
    // own and **not authenticated**; stripe-node reads `request-id` from them
    // for its error messages and nothing else.
    return { ...this.#headers, "content-type": "application/json" };
  }

  getRawResponse(): unknown {
    return this.#raw;
  }

  toStream(): never {
    throw new VpayConfigError(
      "vpay answers are sealed and read whole; streaming responses are not supported",
    );
  }

  toJSON(): Promise<unknown> {
    return Promise.resolve().then(() => JSON.parse(this.#text) as unknown);
  }
}

/**
 * A refusal, presented to stripe-node as the response it is.
 *
 * stripe-node retries **every** rejection from an `httpClient` as a connection
 * error: `maxNetworkRetries` defaults to 2 on Node, so a refused call would go
 * out three times, each a fresh signed message, and the public hook that would
 * exempt a rejection (`HttpClientRuntimeError`) is not exported. A response is
 * different: stripe-node honours `stripe-should-retry: false` before it looks
 * at the status. So a refusal that retrying cannot fix (a `4xx` unsigned
 * answer, an answer that does not verify, a body over the bound) is returned
 * as a Stripe-shaped error response carrying that header, and surfaces as the
 * ordinary `StripeAuthenticationError` / `StripeAPIError` with the cause in
 * `message`, once. A gateway's own `5xx` is left retryable: that is what
 * `maxNetworkRetries` is for, and a keyed write is safe to repeat.
 *
 * A request this client cannot seal at all (no such operation) is the same
 * shape: it was never sent, retrying it cannot change that, and it is the
 * caller's request that is wrong, so it is an `invalid_request_error`
 * (`StripeInvalidRequestError`), as Stripe answers an unrecognised URL.
 */
function refusal(
  status: number,
  code: string,
  message: string,
  retryable: boolean,
  type:
    "authentication_error" | "invalid_request_error" | "api_error" = status ===
  401
    ? "authentication_error"
    : "api_error",
  requestId?: string,
): SealedResponse {
  const body = JSON.stringify({ error: { type, code, message } });
  return new SealedResponse(status, body, {
    ...(retryable ? {} : { "stripe-should-retry": "false" }),
    // The server's own id for the exchange, so the merchant has something to
    // quote to support. Not authenticated, and used for nothing else.
    ...(requestId === undefined ? {} : { "request-id": requestId }),
  });
}

/** Whether a verified payload is already a Stripe error envelope (`{ error: { … } }`). */
function isStripeErrorEnvelope(text: string): boolean {
  try {
    const parsed: unknown = JSON.parse(text);
    return (
      typeof parsed === "object" &&
      parsed !== null &&
      typeof (parsed as Record<string, unknown>)["error"] === "object" &&
      (parsed as Record<string, unknown>)["error"] !== null
    );
  } catch {
    return false;
  }
}

/**
 * A verified `4xx`/`5xx` whose payload is not a Stripe error envelope, put in
 * one.
 *
 * Not every refusal vpay seals comes from its own error renderer: the body
 * bound (`413`) is answered by a layer in front of it, and the signed
 * transport seals that answer as it finds it, in cratestack's own
 * `{ code, message, details }` shape. stripe-node reads an error only from an
 * `error` key, so without this a `413` **resolved**: the merchant's `await
 * paymentIntents.create(…)` returned `{ code: "INTERNAL_ERROR", … }` as though
 * it were a PaymentIntent, and nothing threw. It is now an error that says what
 * happened, never retried when it is a `4xx` (the caller's request) and left
 * retryable when it is a `5xx` (vpay's). stripe-node picks the class from the
 * status alone: `StripeAPIError` for a `413` and a `5xx`, which have no class
 * of their own. The status is the signed
 * one; the sealed body's wording is not repeated (it is `internal error` for a
 * `413`), only its `code`, bounded, when it is a short string.
 */
function unenveloped(
  status: number,
  text: string,
  headers: Record<string, string>,
): SealedResponse {
  const requestId = headers["request-id"] ?? headers["x-request-id"];
  if (status === 413) {
    return refusal(
      413,
      "request_too_large",
      "vpay refused the request: its body is over the limit vpay accepts on /v1 (413)",
      false,
      "invalid_request_error",
      requestId,
    );
  }
  let sealedCode = "";
  try {
    const parsed = JSON.parse(text) as { code?: unknown };
    if (typeof parsed.code === "string" && parsed.code.length <= 64) {
      sealedCode = ` (${parsed.code})`;
    }
  } catch {
    // Not JSON: the status alone is reported.
  }
  return refusal(
    status,
    "unexpected_error_response",
    `vpay answered ${status}${sealedCode} with an error that is not in the Stripe error envelope`,
    status >= 500,
    status >= 500
      ? "api_error"
      : status === 401
        ? "authentication_error"
        : "invalid_request_error",
    requestId,
  );
}

/**
 * What a request this client cannot seal is answered with: a resolved,
 * non-retryable `404` `invalid_request_error` / `unknown_operation`. It is
 * never sent, signed or not, so no message leaves the process.
 */
function unknownOperation(message: string): SealedResponse {
  return refusal(
    404,
    "unknown_operation",
    message,
    false,
    "invalid_request_error",
  );
}

class SealingStripeHttpClient implements VpayStripeHttpClient {
  readonly #auth: ResolvedMerchantAuth;
  readonly #bound: BoundOrigin;

  constructor(auth: ResolvedMerchantAuth, bound: BoundOrigin) {
    this.#auth = auth;
    this.#bound = bound;
  }

  getClientName(): string {
    return "vpay-sealed";
  }

  async makeRequest(
    host: string,
    port: string,
    path: string,
    method: string,
    headers: StripeRequestHeaders,
    requestData: string,
    protocol: string,
    timeout: number,
  ): Promise<VpayStripeHttpResponse> {
    assertRequestIsBound(host, port, protocol, this.#bound);

    const verb = method.toUpperCase();
    if (verb !== "GET" && verb !== "POST" && verb !== "DELETE") {
      return unknownOperation(`vpay's /v1 has no ${verb} operation`);
    }
    const [pathname = "", query] = path.split("?", 2);
    if (!pathname.startsWith("/v1/")) {
      return unknownOperation(
        `refusing to seal ${pathname}: vpay serves /v1/… only`,
      );
    }
    const underV1 = pathname.slice("/v1".length);

    try {
      const {
        status,
        text,
        headers: responseHeaders,
      } = await performSealedCall(
        this.#auth,
        {
          method: verb,
          path: underV1,
          body: verb === "POST" ? requestData : undefined,
          query: verb === "GET" ? query : undefined,
          // stripe-node keys POSTs and nothing else; vpay requires a key on
          // every write, `DELETE` included, so a `DELETE` is given one here.
          // **A retry of a `POST` re-seals with the key stripe-node chose,
          // which is what makes it safe. A retry of a `DELETE` does not:**
          // stripe-node sets no key for it, `makeRequest` runs again with no
          // memory of the first attempt, and this draws a new one. That is
          // acceptable because a `DELETE` is naturally idempotent (the
          // repeat of one that landed is a `404`), and it is why the
          // README tells a caller who cannot tolerate that to set
          // `maxNetworkRetries: 0`.
          idempotencyKey:
            verb === "GET"
              ? undefined
              : (idempotencyKeyOf(headers) ?? randomUUID()),
        },
        Number.isFinite(timeout) && timeout > 0
          ? timeout
          : this.#auth.timeoutMs,
      );
      if (status >= 400 && !isStripeErrorEnvelope(text)) {
        return unenveloped(status, text, responseHeaders);
      }
      return new SealedResponse(status, text, responseHeaders);
    } catch (err) {
      // The sealing module found no such operation in the contract table (a
      // GET on a POST-only path, a path with no route): nothing was sent.
      if (err instanceof VpayInvalidParamsError) {
        return unknownOperation(
          `vpay's /v1 has no ${verb} ${pathname} operation`,
        );
      }
      if (err instanceof VpayUnsignedResponseError) {
        return refusal(
          err.status,
          "unsigned_response",
          err.message,
          err.status >= 500,
        );
      }
      if (err instanceof VpayUnverifiedResponseError) {
        return refusal(502, "unverified_response", err.message, false);
      }
      if (err instanceof VpayResponseTooLargeError) {
        return refusal(502, "response_too_large", err.message, false);
      }
      throw err;
    }
  }
}

/**
 * Builds a stripe-node `httpClient` that seals each request under the
 * merchant's Ed25519 key, sends it, and opens vpay's sealed answer.
 *
 * The signing, the operation table and the verification are the *same* code
 * `VpayClient` and the Rust SDK run (`sdks/rust/wasm`), not a second
 * implementation.
 *
 * # Where a request may be sent
 *
 * **The client is bound to `baseUrl`'s origin and refuses every other one.**
 * stripe-node calls `makeRequest` for *any* request the instance makes;
 * `host`/`port`/`protocol` are configured on the `Stripe` instance, separately
 * from this module, and when they are omitted stripe-node addresses
 * `api.stripe.com:443`. A request signed for vpay and sent to Stripe is not
 * dangerous (the signature binds the audience, and Stripe holds no key that
 * could replay it at vpay), but it is certainly a mistake, and a refusal that
 * names both origins is a better way to learn it than a `401` from a server
 * that was never vpay.
 *
 * So a request whose host, effective port or protocol is not `baseUrl`'s is a
 * {@link VpayConfigError} naming both origins, thrown **before** anything is
 * signed.
 *
 * # What is dropped
 *
 * Everything stripe-node set except the `Idempotency-Key`: `Authorization`
 * (the placeholder API key), `Stripe-Version`, `X-Stripe-Client-User-Agent`,
 * `Content-Type`. The signature binds the payload type itself, and the
 * `Idempotency-Key` as sent.
 *
 * # Failures
 *
 * stripe-node retries any *rejection* from an `httpClient` as a connection
 * error (`maxNetworkRetries` defaults to **2** on Node, not 0), and cannot be
 * told not to, so a refusal is not a rejection. A refused call (the coarse
 * unsigned `401`), an answer that does not verify, or a body over the bound
 * resolves as a Stripe-shaped error response with `stripe-should-retry:
 * false`, and surfaces once: `StripeAuthenticationError` (`code:
 * "unsigned_response"`) for a `401`, `StripeAPIError` (`unverified_response`,
 * `response_too_large`) otherwise, with the cause in `message`. A gateway's
 * unsigned `5xx` stays retryable.
 *
 * A request this client cannot seal, because the contract has no such
 * operation (a `GET` on a `POST`-only path, a path outside `/v1`, a verb `/v1`
 * does not use), is refused the same way, before anything is sent:
 * `StripeInvalidRequestError` with `statusCode: 404` and `code:
 * "unknown_operation"`, once.
 *
 * What does reject is a real transport failure (DNS, TLS, a reset, a timeout)
 * and a {@link VpayConfigError} (a request addressed to another origin, a
 * misconfigured key), reported as `StripeConnectionError` with the
 * original at `err.detail` / `err.raw.exception`, and retried up to
 * `maxNetworkRetries` times. Each retry re-seals the call (a new message). A
 * `POST` keeps the `Idempotency-Key` stripe-node chose, so the retry is the
 * same operation; a `DELETE` has none from stripe-node and gets a new one per
 * attempt (see the comment at the call), so set `maxNetworkRetries: 0` if a
 * repeated `DELETE` answering `404` is not acceptable to you.
 *
 * @throws `VpayConfigError` at construction if `baseUrl` or `clientId` is
 * missing, if `baseUrl` is not an absolute URL, if the key is not an Ed25519
 * key, or if `serverKeys` is missing or malformed.
 */
export function createStripeHttpClient(
  options: StripeHttpClientOptions,
): VpayStripeHttpClient {
  const auth = resolveMerchantAuth(options);
  // Parsed once, here: a `baseUrl` that is not a URL is a configuration
  // mistake the merchant should be told about at construction.
  const bound = parseBoundOrigin(auth.baseUrl);
  return new SealingStripeHttpClient(auth, bound);
}
