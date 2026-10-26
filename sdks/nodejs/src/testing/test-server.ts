/**
 * A real `node:http` server for tests, per the SDK task's decisive-tests
 * requirement: assert the raw bytes on the wire, never a mocked `fetch`.
 *
 * Not a test double reachable from a shipping process — this file is
 * excluded from `dist` (see tsconfig.build.json) and only ever imported
 * from `*.test.ts` files.
 *
 * Concurrency note: each test starts its own server on its own ephemeral
 * port, so the suite is order-independent. It is **not** safe under
 * `--sequence.concurrent`, though: the tests that manipulate time do so with
 * `vi.useFakeTimers`, which patches a module-global `Date` shared by every
 * test in the file, and the `servers` array those files close in `afterEach`
 * is likewise file-global. Run these files sequentially (vitest's default).
 */
import {
  createServer,
  type IncomingMessage,
  type ServerResponse,
} from "node:http";
import type { AddressInfo } from "node:net";
import type { HeaderPairs } from "../wasm.js";
import {
  loadResponderModule,
  newTestIdentity,
  type TestIdentity,
  type WasmResponder,
} from "./responder.js";

export interface RecordedRequest {
  method: string;
  url: string;
  headers: Record<string, string | string[] | undefined>;
  body: string;
}

export type TestHandler = (
  request: RecordedRequest,
  response: ServerResponse,
) => void;

export interface TestServer {
  url: string;
  requests: RecordedRequest[];
  close(): Promise<void>;
}

/** Starts an HTTP server on an ephemeral loopback port, recording every request it receives. */
export function startTestServer(handler: TestHandler): Promise<TestServer> {
  const requests: RecordedRequest[] = [];

  const server = createServer((req: IncomingMessage, res: ServerResponse) => {
    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      const record: RecordedRequest = {
        method: req.method ?? "",
        url: req.url ?? "",
        headers: req.headers,
        body: Buffer.concat(chunks).toString("utf8"),
      };
      requests.push(record);
      handler(record, res);
    });
  });

  return new Promise<TestServer>((resolve, reject) => {
    // Rejects the startup promise if `listen` fails (a port already in use,
    // say). Detached once listening: after that point an `error` event is a
    // live-connection problem, and calling `reject` on an already-settled
    // promise would swallow it silently.
    const onStartupError = (err: Error): void => reject(err);
    server.once("error", onStartupError);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", onStartupError);
      const address = server.address() as AddressInfo;
      resolve({
        url: `http://127.0.0.1:${address.port}`,
        requests,
        close: () =>
          new Promise<void>((res, rej) => {
            // `close` alone waits for every open connection to end, which
            // never happens for a deliberately stalled response — destroy
            // them so a test that stalls the body can still tear down.
            server.closeAllConnections();
            server.close((err) => (err ? rej(err) : res()));
          }),
      });
    });
  });
}

// ---------------------------------------------------------------- sealed

/**
 * What a handler answers a sealed request with. `writeHead` + `end` is all the
 * resource tests use, and it is what `jsonResponse` writes to a plain
 * `ServerResponse`; the sealed server seals the pair when `end` is called.
 */
export interface SealedResponder {
  writeHead(status: number, headers?: Record<string, string>): void;
  end(body?: string): void;
}

export type SealedHandler = (
  request: RecordedRequest,
  response: SealedResponder,
) => void;

export interface SealedTestServer extends TestServer {
  /** The keys this server and its merchant use: hand them to a client. */
  identity: TestIdentity;
  /** The options a client needs to talk to this server. */
  clientOptions: {
    baseUrl: string;
    clientId: string;
    privateKey: string;
    serverKeys: { current: string };
  };
  /**
   * Requests that reached the handler after verifying, as the handler saw them
   * (the opened payload as `body`). `requests` holds those too, plus any request
   * that was refused (as it arrived); `wire` holds every request as it arrived.
   */
  opened: RecordedRequest[];
  /** Every request exactly as it arrived on the socket. */
  wire: RecordedRequest[];
}

export interface SealedServerOptions {
  identity?: TestIdentity;
  /** The audience the server expects. Default: this server's own origin. */
  audience?: string;
  /** Registers no merchant key at all, so every request is refused unsigned. */
  registerMerchant?: boolean;
}

function pairsOf(headers: IncomingMessage["rawHeaders"]): HeaderPairs {
  const out: HeaderPairs = [];
  for (let i = 0; i + 1 < headers.length; i += 2) {
    out.push([headers[i]!, headers[i + 1]!]);
  }
  return out;
}

/**
 * A `node:http` server that behaves like vpay's `/v1` for sealed callers: it
 * opens each request under the merchant's registered key, hands the handler
 * the plain request (the form body, the path, the bound `Idempotency-Key`),
 * and seals what the handler answers under the server's key. A request that
 * does not verify — or that repeats one that did — gets the unsigned coarse
 * `401` and never reaches the handler, as it would not at vpay.
 */
export async function startSealedServer(
  handler: SealedHandler,
  options: SealedServerOptions = {},
): Promise<SealedTestServer> {
  const identity = options.identity ?? newTestIdentity();
  const wasm = loadResponderModule();
  const requests: RecordedRequest[] = [];
  const opened: RecordedRequest[] = [];
  const wire: RecordedRequest[] = [];
  // Assigned once the port is known (the audience is the origin); the request
  // handler below only runs after that.
  const state: { responder?: WasmResponder } = {};

  const server = createServer((req: IncomingMessage, res: ServerResponse) => {
    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      const body = Buffer.concat(chunks);
      const url = new URL(req.url ?? "/", "http://placeholder.invalid");
      const record: RecordedRequest = {
        method: req.method ?? "",
        url: req.url ?? "",
        headers: req.headers,
        body: body.toString("base64"),
      };
      wire.push(record);

      const refuse = (): void => {
        requests.push(record);
        res.writeHead(401, { "Content-Type": "application/json" });
        res.end(
          JSON.stringify({ code: "UNAUTHORIZED", message: "unauthorized" }),
        );
      };
      if (!url.pathname.startsWith("/v1/") || state.responder === undefined) {
        res.writeHead(404, { "Content-Type": "application/json" });
        res.end("{}");
        return;
      }
      const query = url.search.length > 1 ? url.search.slice(1) : undefined;
      state.responder
        .openRequest(
          req.method ?? "",
          url.pathname.slice("/v1".length),
          query,
          pairsOf(req.rawHeaders),
          body,
        )
        .then((request) => {
          // What the handler sees is what a vpay router sees after the layer has
          // opened the message: the payload as the body, the payload's own type
          // as `Content-Type` (none for a bodiless `GET` or `DELETE`), and the answer type
          // the client asked for as `Accept` — not the envelope's headers.
          const bodiless = req.method === "GET" || req.method === "DELETE";
          const seen: IncomingMessage["headers"] = { ...req.headers };
          delete seen["content-type"];
          delete seen["content-length"];
          if (!bodiless) {
            seen["content-type"] = "application/x-www-form-urlencoded";
          }
          seen["accept"] = "application/json";
          const plain: RecordedRequest = {
            method: record.method,
            url: record.url,
            headers: seen,
            body: request.payload,
          };
          opened.push(plain);
          requests.push(plain);
          let status = 200;
          handler(plain, {
            writeHead: (code) => {
              status = code;
            },
            end: (text = "") => {
              request
                .sealResponse(status, text)
                .then((sealed) => {
                  const headers: Record<string, string> = {};
                  for (const [name, value] of sealed.headers) {
                    headers[name] = value;
                  }
                  res.writeHead(status, headers);
                  res.end(Buffer.from(sealed.body));
                })
                .catch(() => {
                  res.writeHead(500);
                  res.end();
                });
            },
          });
        }, refuse);
    });
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolve());
  });
  const port = (server.address() as AddressInfo).port;
  const origin = `http://127.0.0.1:${port}`;
  state.responder = new wasm.Responder(
    identity.serverSeed,
    options.registerMerchant === false ? [] : [identity.merchantPublic],
    options.audience ?? origin,
  );

  return {
    url: origin,
    requests,
    opened,
    wire,
    identity,
    clientOptions: {
      baseUrl: origin,
      clientId: "merchant_a",
      privateKey: identity.merchantSeed,
      serverKeys: { current: identity.serverPublic },
    },
    close: () =>
      new Promise<void>((res, rej) => {
        server.closeAllConnections();
        server.close((err) => (err ? rej(err) : res()));
      }),
  };
}
