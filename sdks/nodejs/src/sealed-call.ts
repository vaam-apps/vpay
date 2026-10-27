/**
 * One sealed call, start to finish: seal, send, open.
 *
 * The single place the two entry points (`VpayClient`'s resources and
 * `@vaam-apps/vpay-sdk/stripe`'s HTTP client) go through, so that both bind the
 * same bytes and map the same failures to the same errors.
 */
import {
  VpayConfigError,
  VpayInvalidParamsError,
  VpayTransportError,
  VpayUnexpectedResponseError,
  VpayUnsignedResponseError,
  VpayUnverifiedResponseError,
} from "./errors.js";
import type { ResolvedMerchantAuth } from "./sealer.js";
import { sendSealed } from "./transport.js";
import { isWasmFailure, type WasmSealedCall } from "./wasm.js";

export interface SealedCallSpec {
  method: "GET" | "POST" | "DELETE";
  /** The path **under `/v1`**, parameters in place: `/payment_intents/pi_1/confirm`. */
  path: string;
  /** The form-encoded body of a write. Absent for a read and for a `DELETE`. */
  body?: string | undefined;
  /** The canonical query string of a read, without the `?`. */
  query?: string | undefined;
  /** The `Idempotency-Key` of a write, bound by the signature as sent. */
  idempotencyKey?: string | undefined;
}

/** An answer that verified. */
export interface OpenedAnswer {
  /** The HTTP status, which the signature also binds. */
  status: number;
  /** The verified JSON text. */
  text: string;
  /**
   * The response headers as they arrived, lower-cased, last one winning.
   * **Not authenticated**: the signature covers the payload, the status and the
   * request it answers, not these. Good for a `request-id` in a log line,
   * nothing a decision should rest on.
   */
  headers: Record<string, string>;
}

function mapFailure(err: unknown): Error {
  if (!isWasmFailure(err)) {
    return new VpayConfigError("sealing the call failed", { cause: err });
  }
  switch (err.kind) {
    case "unbound":
      return new VpayInvalidParamsError("path", err.message);
    case "unsigned":
      return new VpayUnsignedResponseError(err.status ?? 0);
    case "unverified":
      return new VpayUnverifiedResponseError();
    default:
      return new VpayConfigError(err.message);
  }
}

/**
 * Seals `spec`, sends it, and opens the answer. Exactly one attempt: a sealed
 * request is never retried here (a retry is a new message with a new `cti`,
 * which is only safe for a call whose `Idempotency-Key` the caller chose).
 */
export async function performSealedCall(
  auth: ResolvedMerchantAuth,
  spec: SealedCallSpec,
  timeoutMs: number = auth.timeoutMs,
): Promise<OpenedAnswer> {
  let sealed: WasmSealedCall;
  try {
    sealed = await auth.sealer.sealCall(
      spec.method,
      spec.path,
      spec.body,
      spec.query,
      spec.idempotencyKey,
    );
  } catch (err) {
    throw mapFailure(err);
  }

  const query =
    spec.query !== undefined && spec.query.length > 0 ? `?${spec.query}` : "";
  const response = await sendSealed(
    {
      method: spec.method,
      url: `${auth.baseUrl}/v1${spec.path}${query}`,
      headers: [...sealed.headers, ["user-agent", auth.userAgent]],
      body: sealed.body,
      timeoutMs,
    },
    { agent: auth.agent },
  );

  try {
    const text = await sealed.openResponse(
      response.status,
      response.headers,
      response.body,
    );
    const headers: Record<string, string> = {};
    for (const [name, value] of response.headers) {
      headers[name.toLowerCase()] = value;
    }
    return { status: response.status, text, headers };
  } catch (err) {
    if (isWasmFailure(err) && err.kind === "seal") {
      // The module could not read the answer at all (an unreadable header, a
      // payload type vpay did not advertise): the response, not the request.
      throw new VpayUnexpectedResponseError(response.status, err.message);
    }
    if (err instanceof VpayTransportError) throw err;
    throw mapFailure(err);
  }
}
