/**
 * A real `node:http` server standing in for vpay's `/v1/browser` surface.
 *
 * **This is not a test double reachable from a shipping process.** ADR-0006
 * and AGENTS.md's rule 1 forbid a mock, fake or stub being linked into
 * `vpay-server` or `vpay-worker-bin`; this file is TypeScript in an SDK
 * package, excluded from `dist` by `tsconfig.build.json`, and imported only
 * from `*.test.ts`. Nothing in the Rust workspace can reach it, and no
 * shipping byte of `@vpay/stripe-js` can either.
 *
 * It is a real HTTP server on a real socket, not a patched `fetch`, for the
 * same reason `@vpay/sdk` uses one: the assertions that matter here are
 * about **bytes on the wire** — the exact form encoding of
 * `payment_method_data[mtn_momo][msisdn]`, the query string carrying `key`
 * and `client_secret`, the absence of an `Idempotency-Key` header. A mocked
 * `fetch` would assert the arguments this package passes to a function it
 * also controls, which is a tautology.
 *
 * Each test starts its own server on its own ephemeral port, so the suite is
 * order-independent. It is not safe under `--sequence.concurrent`: the
 * polling tests use `vi.useFakeTimers`, which patches a module-global
 * `Date`.
 */
import {
  createServer,
  type IncomingMessage,
  type ServerResponse,
} from "node:http";
import type { AddressInfo } from "node:net";

export interface RecordedRequest {
  method: string;
  /** The raw request target, query string included — asserted on verbatim. */
  url: string;
  headers: Record<string, string | string[] | undefined>;
  /** The raw request body, undecoded — asserted on verbatim. */
  body: string;
}

export type StubHandler = (
  request: RecordedRequest,
  response: ServerResponse,
) => void;

export interface BrowserStub {
  /** Origin to hand to `loadStripe` as `baseUrl`. */
  url: string;
  requests: RecordedRequest[];
  close(): Promise<void>;
}

/** Starts a stub on an ephemeral loopback port, recording every request it receives. */
export function startBrowserStub(handler: StubHandler): Promise<BrowserStub> {
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

  return new Promise<BrowserStub>((resolve, reject) => {
    // Rejects startup if `listen` fails. Detached once listening: after that
    // an `error` is a live-connection problem, and rejecting an already
    // settled promise would swallow it.
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
            server.closeAllConnections();
            server.close((err) => (err ? rej(err) : res()));
          }),
      });
    });
  });
}

/** Writes a JSON response, the way `axum`'s `Json` does. */
export function json(res: ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

/**
 * `vpay_api::error_envelope_with_param`, reproduced key for key: `type` and
 * `code` always present, `param` present only when the error names one.
 */
export function errorEnvelope(
  type: string,
  code: string,
  message: string,
  param?: string,
): { error: Record<string, string> } {
  const error: Record<string, string> = { type, code, message };
  if (param !== undefined) {
    error["param"] = param;
  }
  return { error };
}

/**
 * The uniform 404 every credential failure on the browser surface renders —
 * `ApiError::NotFound { resource: "payment intent", id }` through
 * `Category::NotFound` (404, `invalid_request_error`, `resource_missing`,
 * no `param`).
 */
export function notFoundEnvelope(id: string): {
  error: Record<string, string>;
} {
  return errorEnvelope(
    "invalid_request_error",
    "resource_missing",
    `No such payment intent: ${id}`,
  );
}

export const SAMPLE_SECRET_SUFFIX = "a".repeat(32);

/** A `PaymentIntentWithSecret` body: the twelve documented keys plus `client_secret`. */
export function samplePaymentIntent(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    id: "pi_123",
    object: "payment_intent",
    amount: 5000,
    currency: "xaf",
    status: "requires_payment_method",
    payment_method_types: ["mtn_momo"],
    next_action: null,
    last_payment_error: null,
    metadata: {},
    description: null,
    created: 1_700_000_000,
    livemode: false,
    client_secret: `pi_123_secret_${SAMPLE_SECRET_SUFFIX}`,
    ...overrides,
  };
}

/**
 * The uniform 404 the browser surface renders for a checkout session —
 * the session's counterpart to {@link notFoundEnvelope}, and equally
 * byte-identical across every credential failure (unknown key, wrong
 * secret, another merchant's session, unknown id).
 */
export function sessionNotFoundEnvelope(id: string): {
  error: Record<string, string>;
} {
  return errorEnvelope(
    "invalid_request_error",
    "resource_missing",
    `No such checkout session: ${id}`,
  );
}

/**
 * A `checkout.session` body as `GET /v1/browser/checkout/sessions/{id}`
 * renders it — the object pinned in §"The wire contract" of
 * `docs/plans/2026-09-04-step9-hosted-checkout.md`, `client_secret`
 * included and **`payment_intent` expanded**.
 *
 * The expansion is the integrator's ruling of 2026-09-04 on a contract the
 * plan left readable two ways: on the browser routes `payment_intent` is
 * the whole intent (with its `client_secret` on the session read, without
 * it on the return read), and on `/v1` it stays the `pi_…` string. The
 * fixture therefore carries a live *confirm* credential, which is a fact
 * about the route rather than a convenience of the stub.
 *
 * The route also carries the merchant's display name, which this package
 * does not model and this stub does not send: a fixture is not the place to
 * pin a field nothing reads.
 */
export function sampleCheckoutSession(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    id: "cs_123",
    object: "checkout.session",
    livemode: false,
    payment_intent: samplePaymentIntent(),
    ui_mode: "embedded",
    status: "open",
    payment_status: "unpaid",
    success_url: null,
    cancel_url: null,
    return_url: "https://shop.example/thanks",
    url: null,
    expires_at: 1_700_086_400,
    created: 1_700_000_000,
    client_secret: `cs_123_secret_${SAMPLE_SECRET_SUFFIX}`,
    ...overrides,
  };
}
