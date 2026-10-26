/**
 * The HTTP leg of a sealed call, on `node:http` and `node:https`.
 *
 * Not `fetch`: a sealed `GET` carries the signed message as its **body** (the
 * request is a COSE_Sign1 even when the operation has no payload — that is how
 * the signature reaches the server), and `fetch` refuses a body on `GET`. The
 * Rust SDK's `reqwest` and stripe-node's own Node client have no such rule, and
 * neither does `http.request`.
 *
 * One attempt, no retry, no redirect following: a redirect would re-send a
 * signed body to a host the signature was not addressed to.
 */
import { request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
import type { Agent as HttpAgent } from "node:http";
import type { Agent as HttpsAgent } from "node:https";
import { VpayTransportError } from "./errors.js";
import type { HeaderPairs } from "./wasm.js";

export interface TransportRequest {
  method: string;
  url: string;
  headers: HeaderPairs;
  body: Uint8Array;
  timeoutMs: number;
}

export interface TransportResponse {
  status: number;
  /** Every response header, in order, duplicates preserved. */
  headers: HeaderPairs;
  body: Uint8Array;
}

/** What {@link sendSealed} needs from the caller's environment. */
export interface TransportOptions {
  /** A custom agent (a proxy, a pool). Defaults to the global agent. */
  agent?: HttpAgent | HttpsAgent | undefined;
}

/** Sends one request and reads the whole answer. Rejects with {@link VpayTransportError}. */
export function sendSealed(
  request: TransportRequest,
  options: TransportOptions = {},
): Promise<TransportResponse> {
  return new Promise<TransportResponse>((resolve, reject) => {
    const signal = AbortSignal.timeout(request.timeoutMs);
    // A request that outlives its timeout is aborted, and `http.request` then
    // reports a generic `AbortError`. The reason on the signal is the
    // `TimeoutError` a caller can tell from a connection failure, so that is
    // what is attached as the cause.
    const fail = (err: unknown): void =>
      reject(
        new VpayTransportError("request failed", {
          cause: signal.aborted ? (signal.reason as unknown) : err,
        }),
      );

    let url: URL;
    try {
      url = new URL(request.url);
    } catch (err) {
      fail(err);
      return;
    }
    const send = url.protocol === "https:" ? httpsRequest : httpRequest;
    const headers: Record<string, string | string[]> = {};
    for (const [name, value] of request.headers) {
      const key = name.toLowerCase();
      const existing = headers[key];
      headers[key] =
        existing === undefined
          ? value
          : Array.isArray(existing)
            ? [...existing, value]
            : [existing, value];
    }

    // Explicit, because Node omits the length of a `GET` body unless told (it
    // assumes none), and the sealed message would then be read by the server as
    // the start of a second, malformed request.
    headers["content-length"] = String(request.body.byteLength);

    const req = send(
      url,
      {
        method: request.method,
        headers,
        agent: options.agent,
        signal,
      },
      (res) => {
        const chunks: Buffer[] = [];
        res.on("data", (chunk: Buffer) => chunks.push(chunk));
        res.on("error", fail);
        // `aborted` fires when the peer severs a response mid-body; `end`
        // would then never come, and the caller would wait for the timeout.
        res.on("aborted", () => fail(new Error("response aborted")));
        res.on("end", () => {
          const pairs: HeaderPairs = [];
          for (let i = 0; i + 1 < res.rawHeaders.length; i += 2) {
            pairs.push([res.rawHeaders[i]!, res.rawHeaders[i + 1]!]);
          }
          resolve({
            status: res.statusCode ?? 0,
            headers: pairs,
            body: Buffer.concat(chunks),
          });
        });
      },
    );
    req.on("error", fail);
    req.end(request.body);
  });
}
