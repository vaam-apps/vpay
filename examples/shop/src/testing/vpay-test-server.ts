/**
 * A real `node:http` server standing in for vpay, for this package's tests.
 *
 * It speaks vpay's signed `/v1`: each request is opened under the shop's
 * registered Ed25519 key, the handler is given the plain request, and what it
 * answers is sealed under the server's key — cratestack's own server envelope,
 * through `sdks/nodejs`'s test server (`src/testing/test-server.ts`), which is
 * imported by path because it is test code that package does not publish.
 *
 * The assertions worth making here are about the **bytes the shop hands vpay**
 * — the `Idempotency-Key` header, the form-encoded amount, the `success_url`
 * with its literal `{CHECKOUT_SESSION_ID}` — and a mocked `fetch` would let a
 * change to the SDK's encoder pass unnoticed. The real `VpayClient` signs a
 * real message and posts it to this server, which verifies it.
 *
 * **Test-only.** Nothing under `src/app` or `src/server` imports it; the test
 * beside this file proves that.
 */
import {
  startSealedServer,
  type SealedResponder,
  type SealedTestServer,
} from "../../../../sdks/nodejs/src/testing/test-server";

export interface RecordedRequest {
  method: string;
  url: string;
  headers: Record<string, string | string[] | undefined>;
  /** The opened payload: the form text of a write. */
  body: string;
  /** The body decoded as `application/x-www-form-urlencoded`. */
  form: URLSearchParams;
}

export interface VpayTestServer {
  url: string;
  /** What a `VpayClient` talking to this server is built from. */
  clientOptions: SealedTestServer["clientOptions"];
  /** Every request that verified, in order. */
  requests: RecordedRequest[];
  /** Requests to a path. */
  requestsTo(method: string, path: string): RecordedRequest[];
  close(): Promise<void>;
}

export type RouteHandler = (
  request: RecordedRequest,
  response: SealedResponder,
) => void;

export interface VpayTestServerOptions {
  /** Keyed by `"<METHOD> <pathname>"`, e.g. `"POST /v1/payment_intents"`. */
  routes: Record<string, RouteHandler>;
}

function json(response: SealedResponder, status: number, body: unknown): void {
  response.writeHead(status, { "Content-Type": "application/json" });
  response.end(JSON.stringify(body));
}

/** Answers a route with `body` as JSON and `status`. */
export function reply(status: number, body: unknown): RouteHandler {
  return (_request, response) => json(response, status, body);
}

export async function startVpayTestServer(
  options: VpayTestServerOptions,
): Promise<VpayTestServer> {
  const requests: RecordedRequest[] = [];
  const server = await startSealedServer((plain, res) => {
    const record: RecordedRequest = {
      ...plain,
      form: new URLSearchParams(plain.body),
    };
    requests.push(record);
    const pathname = new URL(record.url, "http://127.0.0.1").pathname;
    const handler = options.routes[`${record.method} ${pathname}`];
    if (handler === undefined) {
      json(res, 404, {
        error: { type: "invalid_request_error", code: "resource_missing" },
      });
      return;
    }
    handler(record, res);
  });
  return {
    url: server.url,
    clientOptions: server.clientOptions,
    requests,
    requestsTo(method: string, path: string): RecordedRequest[] {
      return requests.filter(
        (request) =>
          request.method === method &&
          new URL(request.url, "http://127.0.0.1").pathname === path,
      );
    },
    close: () => server.close(),
  };
}
