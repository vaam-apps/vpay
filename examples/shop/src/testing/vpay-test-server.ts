/**
 * A real `node:http` server standing in for vpay, for this package's tests.
 *
 * The same shape as `sdks/nodejs/src/testing/test-server.ts`, and for the
 * same reason: the assertions worth making here are about the **bytes on the
 * wire** — the `Idempotency-Key` header, the form-encoded amount, the
 * `success_url` with its literal `{CHECKOUT_SESSION_ID}` — and a mocked
 * `fetch` would let a change to the SDK's encoder pass unnoticed. The real
 * `VpayClient` mints a real assertion, exchanges a real token, and posts a
 * real body to this server.
 *
 * **Test-only.** Nothing under `src/app` or `src/server` imports it; the test
 * beside this file proves that.
 */
import { createServer, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";

export interface RecordedRequest {
  method: string;
  url: string;
  headers: Record<string, string | string[] | undefined>;
  body: string;
  /** The body decoded as `application/x-www-form-urlencoded`. */
  form: URLSearchParams;
}

export interface VpayTestServer {
  url: string;
  /** Every request, in order, including the token exchange. */
  requests: RecordedRequest[];
  /** Requests to a path, token exchange excluded. */
  requestsTo(method: string, path: string): RecordedRequest[];
  close(): Promise<void>;
}

export type RouteHandler = (
  request: RecordedRequest,
  response: ServerResponse,
) => void;

export interface VpayTestServerOptions {
  /** Keyed by `"<METHOD> <pathname>"`, e.g. `"POST /v1/payment_intents"`. */
  routes: Record<string, RouteHandler>;
}

function json(response: ServerResponse, status: number, body: unknown): void {
  const text = JSON.stringify(body);
  response.writeHead(status, {
    "Content-Type": "application/json",
    "Content-Length": String(Buffer.byteLength(text)),
  });
  response.end(text);
}

/** Answers a route with `body` as JSON and `status`. */
export function reply(status: number, body: unknown): RouteHandler {
  return (_request, response) => json(response, status, body);
}

export function startVpayTestServer(
  options: VpayTestServerOptions,
): Promise<VpayTestServer> {
  const requests: RecordedRequest[] = [];
  const routes: Record<string, RouteHandler> = {
    // Every test needs the token exchange, and none of them is about it.
    "POST /v1/oauth/token": reply(200, {
      access_token: "test-access-token",
      token_type: "Bearer",
      expires_in: 3600,
    }),
    ...options.routes,
  };

  const server = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      const body = Buffer.concat(chunks).toString("utf8");
      const url = req.url ?? "";
      const record: RecordedRequest = {
        method: req.method ?? "",
        url,
        headers: req.headers,
        body,
        form: new URLSearchParams(body),
      };
      requests.push(record);
      const pathname = new URL(url, "http://127.0.0.1").pathname;
      const handler = routes[`${record.method} ${pathname}`];
      if (handler === undefined) {
        json(res, 404, {
          error: { type: "invalid_request_error", code: "resource_missing" },
        });
        return;
      }
      handler(record, res);
    });
  });

  return new Promise<VpayTestServer>((resolve, reject) => {
    const onStartupError = (err: Error): void => reject(err);
    server.once("error", onStartupError);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", onStartupError);
      const address = server.address() as AddressInfo;
      resolve({
        url: `http://127.0.0.1:${address.port}`,
        requests,
        requestsTo(method: string, path: string): RecordedRequest[] {
          return requests.filter(
            (request) =>
              request.method === method &&
              new URL(request.url, "http://127.0.0.1").pathname === path,
          );
        },
        close: () =>
          new Promise<void>((res, rej) => {
            server.closeAllConnections();
            server.close((err) => (err ? rej(err) : res()));
          }),
      });
    });
  });
}
