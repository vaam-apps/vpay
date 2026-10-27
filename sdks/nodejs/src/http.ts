/**
 * The `/v1` resource HTTP layer: request encoding, the `Idempotency-Key`, and
 * error-envelope mapping (docs/flows/merchant-auth.md, "Headers"/"Errors").
 *
 * Every request is sealed (`sealed-call.ts`); there is no token, no
 * `Authorization` header and no re-authentication retry.
 */
import { randomUUID } from "node:crypto";
import {
  VpayApiError,
  VpayUnexpectedResponseError,
  boundedBodyPrefix,
} from "./errors.js";
import { encodeForm, type FormValue } from "./form.js";
import { performSealedCall } from "./sealed-call.js";
import type { ResolvedMerchantAuth } from "./sealer.js";
import type { RequestOptions } from "./types.js";

/**
 * `DELETE` since 2026-09-06 (S4a) — `DELETE /v1/customers/{id}` is the one
 * this API has. It is handled as a write below, not as a read: it carries an
 * `Idempotency-Key` and, deliberately, **no body** (the sealed message still
 * has an empty payload; the server sees a bodiless `DELETE`).
 *
 * The header is the server's contract (every write under `/v1` carries one)
 * and it is the verb where a replay is most confusing without it — the second
 * call would otherwise answer `404` for a deletion that succeeded, which a
 * merchant retrying a timed-out request cannot tell from "somebody else
 * deleted it".
 */
type HttpMethod = "GET" | "POST" | "DELETE";

function isErrorEnvelope(value: unknown): value is {
  error: { type: string; code?: string; message: string; param?: string };
} {
  if (typeof value !== "object" || value === null || !("error" in value)) {
    return false;
  }
  const err = value.error;
  return (
    typeof err === "object" &&
    err !== null &&
    typeof (err as Record<string, unknown>)["type"] === "string" &&
    typeof (err as Record<string, unknown>)["message"] === "string"
  );
}

/** Thin wrapper over the sealed call implementing the `/v1` wire contract. */
export class HttpClient {
  readonly #auth: ResolvedMerchantAuth;

  constructor(auth: ResolvedMerchantAuth) {
    this.#auth = auth;
  }

  async request<T>(
    method: HttpMethod,
    path: string,
    params?: Record<string, FormValue>,
    requestOptions?: RequestOptions,
  ): Promise<T> {
    let query: string | undefined;
    let body: string | undefined;
    let idempotencyKey: string | undefined;

    if (method === "GET") {
      if (params) {
        const encoded = encodeForm(params);
        if (encoded.length > 0) {
          query = encoded;
        }
      }
    } else if (method === "DELETE") {
      idempotencyKey = requestOptions?.idempotencyKey ?? randomUUID();
    } else {
      body = params ? encodeForm(params) : "";
      idempotencyKey = requestOptions?.idempotencyKey ?? randomUUID();
    }

    const { status, text } = await performSealedCall(this.#auth, {
      method,
      path,
      body,
      query,
      idempotencyKey,
    });
    return this.#mapResponse<T>(status, text);
  }

  #mapResponse<T>(status: number, text: string): T {
    if (status >= 200 && status < 300) {
      try {
        return (text.length > 0 ? JSON.parse(text) : undefined) as T;
      } catch {
        throw new VpayUnexpectedResponseError(status, boundedBodyPrefix(text));
      }
    }

    let parsed: unknown;
    try {
      parsed = text.length > 0 ? JSON.parse(text) : undefined;
    } catch {
      parsed = undefined;
    }

    if (isErrorEnvelope(parsed)) {
      throw new VpayApiError(status, parsed.error);
    }
    throw new VpayUnexpectedResponseError(status, boundedBodyPrefix(text));
  }
}
