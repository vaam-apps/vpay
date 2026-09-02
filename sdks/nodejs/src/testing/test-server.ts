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
