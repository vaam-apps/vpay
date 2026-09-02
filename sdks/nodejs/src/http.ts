/**
 * The `/v1` resource HTTP layer: request encoding, headers, the single
 * 401-triggered re-auth retry, and error-envelope mapping
 * (docs/flows/merchant-auth.md, "Headers"/"Errors"/"Re-authentication").
 */
import { randomUUID } from "node:crypto";
import type { TokenManager } from "./auth.js";
import {
  VpayApiError,
  VpayTransportError,
  VpayUnexpectedResponseError,
  boundedBodyPrefix,
} from "./errors.js";
import { encodeForm, type FormValue } from "./form.js";
import type { RequestOptions } from "./types.js";

export interface HttpClientOptions {
  baseUrl: string;
  tokenManager: TokenManager;
  fetchImpl: typeof fetch;
  timeoutMs: number;
  userAgent: string;
}

type HttpMethod = "GET" | "POST";

function isErrorEnvelope(value: unknown): value is {
  error: { type: string; code?: string; message: string; param?: string };
} {
  if (typeof value !== "object" || value === null || !("error" in value)) {
    return false;
  }
  const err = (value as { error: unknown }).error;
  return (
    typeof err === "object" &&
    err !== null &&
    typeof (err as Record<string, unknown>)["type"] === "string" &&
    typeof (err as Record<string, unknown>)["message"] === "string"
  );
}

/** Thin wrapper over `fetch` implementing the merchant-auth wire contract for `/v1`. */
export class HttpClient {
  readonly #options: HttpClientOptions;

  constructor(options: HttpClientOptions) {
    this.#options = options;
  }

  async request<T>(
    method: HttpMethod,
    path: string,
    params?: Record<string, FormValue> | undefined,
    requestOptions?: RequestOptions | undefined,
  ): Promise<T> {
    let url = `${this.#options.baseUrl}${path}`;
    let body: string | undefined;

    const headers: Record<string, string> = {
      Accept: "application/json",
      "User-Agent": this.#options.userAgent,
    };

    if (method === "GET") {
      if (params) {
        const query = encodeForm(params);
        if (query.length > 0) {
          url += `?${query}`;
        }
      }
    } else {
      body = params ? encodeForm(params) : "";
      headers["Content-Type"] = "application/x-www-form-urlencoded";
      headers["Idempotency-Key"] =
        requestOptions?.idempotencyKey ?? randomUUID();
    }

    // `headers` and `body` are built once, above the retry loop, and the
    // 401 re-auth below reuses them untouched — only `Authorization` is
    // rewritten. The second attempt therefore carries a byte-identical
    // `Idempotency-Key` and body, so a re-auth can never double-create
    // (docs/flows/merchant-auth.md, "Headers"/"Re-authentication").
    let attempt = 0;
    for (;;) {
      attempt += 1;
      const token = await this.#options.tokenManager.getToken();
      headers["Authorization"] = `Bearer ${token}`;

      let status: number;
      let ok: boolean;
      let text: string;
      try {
        const init: RequestInit = {
          method,
          headers,
          signal: AbortSignal.timeout(this.#options.timeoutMs),
        };
        if (body !== undefined) {
          init.body = body;
        }
        const response = await this.#options.fetchImpl(url, init);
        status = response.status;
        ok = response.ok;
        // Read the body inside the same `try`. `fetch` resolves as soon as the
        // response headers arrive, so a stalled or truncated body — or the
        // timeout signal firing mid-stream — rejects *here*, not at the call
        // above. Outside this block that rejection escapes as a raw
        // `DOMException: TimeoutError` instead of a `VpayTransportError`.
        text = await response.text();
      } catch (err) {
        throw new VpayTransportError("request failed", { cause: err });
      }

      if (status === 401 && attempt === 1) {
        this.#options.tokenManager.invalidate();
        continue;
      }

      return this.#mapResponse<T>(status, ok, text);
    }
  }

  #mapResponse<T>(status: number, ok: boolean, text: string): T {
    if (ok) {
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
