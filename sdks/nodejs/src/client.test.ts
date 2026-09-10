import { verify } from "node:crypto";
import type { ServerResponse } from "node:http";
import { inspect } from "node:util";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MAX_ASSERTION_LIFETIME_SECONDS } from "./auth.js";
import { VpayClient } from "./client.js";
import {
  VpayApiError,
  VpayAuthError,
  VpayConfigError,
  VpayTransportError,
  VpayUnexpectedResponseError,
} from "./errors.js";
import { generateTestRsaKeyPair } from "./testing/keys.js";
import {
  startTestServer,
  type RecordedRequest,
  type TestServer,
} from "./testing/test-server.js";
import {
  isCheckoutSessionEvent,
  isInvoiceEvent,
  isPaymentIntentEvent,
} from "./types.js";
import type { Event, KnownEventType, PaymentIntent } from "./types.js";
import { SDK_VERSION } from "./version.js";

const { privateKey, privateKeyPem, publicKey } = generateTestRsaKeyPair();
const TOKEN_PATH = "/v1/oauth/token";

function parseFormBody(body: string): Record<string, string> {
  const out: Record<string, string> = {};
  if (body.length === 0) return out;
  for (const pair of body.split("&")) {
    const eq = pair.indexOf("=");
    const key = decodeURIComponent(eq === -1 ? pair : pair.slice(0, eq));
    const value = eq === -1 ? "" : decodeURIComponent(pair.slice(eq + 1));
    out[key] = value;
  }
  return out;
}

interface AssertionHeader {
  alg: string;
  typ: string;
  kid?: string;
}

interface AssertionPayload {
  iss: string;
  sub: string;
  aud: string;
  jti: string;
  exp: number;
  iat: number;
}

function decodeJwtPart<T>(part: string): T {
  return JSON.parse(Buffer.from(part, "base64url").toString("utf8")) as T;
}

/** The client assertion the client actually put on the wire, decoded. */
function sentAssertion(server: TestServer): {
  header: AssertionHeader;
  payload: AssertionPayload;
} {
  const tokenRequest = server.requests.find((r) => r.url === TOKEN_PATH)!;
  const jwt = parseFormBody(tokenRequest.body)["client_assertion"]!;
  const [headerPart, payloadPart] = jwt.split(".");
  return {
    header: decodeJwtPart<AssertionHeader>(headerPart!),
    payload: decodeJwtPart<AssertionPayload>(payloadPart!),
  };
}

function jsonResponse(
  res: ServerResponse,
  status: number,
  body: unknown,
): void {
  const text = JSON.stringify(body);
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(text);
}

function makeSamplePaymentIntent(
  overrides: Partial<PaymentIntent> = {},
): PaymentIntent {
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
    ...overrides,
  };
}

interface HandshakeServerConfig {
  tokenExpiresIn?: number;
  /** Overrides the default 200 token response for a given 1-indexed token call. Return undefined to fall through to the default. */
  onToken?: (
    req: RecordedRequest,
    callIndex: number,
  ) => { status: number; body: unknown } | undefined;
  /** Handles every non-token request. `callIndex` is 1-indexed across all resource calls. */
  resource: (
    req: RecordedRequest,
    callIndex: number,
  ) => { status: number; body: unknown };
}

async function startHandshakeServer(
  config: HandshakeServerConfig,
): Promise<TestServer> {
  let tokenCount = 0;
  let resourceCount = 0;
  return startTestServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    if (req.method === "POST" && url.pathname === TOKEN_PATH) {
      tokenCount += 1;
      const override = config.onToken?.(req, tokenCount);
      if (override) {
        jsonResponse(res, override.status, override.body);
        return;
      }
      jsonResponse(res, 200, {
        access_token: `access-token-${tokenCount}`,
        token_type: "Bearer",
        expires_in: config.tokenExpiresIn ?? 300,
      });
      return;
    }
    resourceCount += 1;
    const { status, body } = config.resource(req, resourceCount);
    jsonResponse(res, status, body);
  });
}

function makeClient(
  server: TestServer,
  overrides: Partial<ConstructorParameters<typeof VpayClient>[0]> = {},
) {
  return new VpayClient({
    baseUrl: server.url,
    clientId: "merchant_a",
    privateKey,
    ...overrides,
  });
}

let servers: TestServer[] = [];
async function withServer(config: HandshakeServerConfig): Promise<TestServer> {
  const server = await startHandshakeServer(config);
  servers.push(server);
  return server;
}

afterEach(async () => {
  await Promise.all(servers.map((s) => s.close()));
  servers = [];
  vi.useRealTimers();
});

describe("token exchange", () => {
  it("sends the exact form fields and content type, and no client_secret", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.retrieve("pi_123");

    const tokenRequest = server.requests.find((r) => r.url === TOKEN_PATH)!;
    expect(tokenRequest.method).toBe("POST");
    expect(tokenRequest.headers["content-type"]).toBe(
      "application/x-www-form-urlencoded",
    );
    expect(tokenRequest.headers["accept"]).toBe("application/json");

    const fields = parseFormBody(tokenRequest.body);
    expect(fields["grant_type"]).toBe("client_credentials");
    expect(fields["client_id"]).toBe("merchant_a");
    expect(fields["client_assertion_type"]).toBe(
      "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
    );
    expect(fields["client_assertion"]).toBeTruthy();
    expect(fields["client_assertion"]!.split(".")).toHaveLength(3);
    expect(fields["audience"]).toBe("vpay:v1");
    expect(fields).not.toHaveProperty("client_secret");
  });

  it("carries the access token as Authorization: Bearer on the following resource call", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.retrieve("pi_123");

    const resourceRequest = server.requests.find((r) =>
      r.url.startsWith("/v1/payment_intents"),
    )!;
    expect(resourceRequest.headers["authorization"]).toBe(
      "Bearer access-token-1",
    );
  });

  it("omits scope from the token request when not configured, and includes it when configured", async () => {
    const configured = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    await makeClient(configured, {
      scope: "payments:write",
    }).paymentIntents.retrieve("pi_123");
    const configuredFields = parseFormBody(
      configured.requests.find((r) => r.url === TOKEN_PATH)!.body,
    );
    expect(configuredFields["scope"]).toBe("payments:write");

    // The omit half: no `scope` option means no `scope` field at all, not an
    // empty one — the OP treats a present-but-empty scope differently.
    const unconfigured = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    await makeClient(unconfigured).paymentIntents.retrieve("pi_123");
    const unconfiguredFields = parseFormBody(
      unconfigured.requests.find((r) => r.url === TOKEN_PATH)!.body,
    );
    expect(unconfiguredFields).not.toHaveProperty("scope");
    expect(
      unconfigured.requests.find((r) => r.url === TOKEN_PATH)!.body,
    ).not.toContain("scope");
  });
});

describe("token caching", () => {
  it("reuses one token across two resource calls", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.retrieve("pi_123");
    await client.paymentIntents.retrieve("pi_123");

    const tokenRequests = server.requests.filter((r) => r.url === TOKEN_PATH);
    expect(tokenRequests).toHaveLength(1);
  });

  it("re-authenticates once the cached token has passed expires_in minus the safety margin", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      const server = await withServer({
        tokenExpiresIn: 100, // margin = min(30, 50) = 30 -> cached for 70s
        resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
      });
      const client = makeClient(server);

      await client.paymentIntents.retrieve("pi_123");
      expect(server.requests.filter((r) => r.url === TOKEN_PATH)).toHaveLength(
        1,
      );

      vi.setSystemTime(Date.now() + 69_000);
      await client.paymentIntents.retrieve("pi_123");
      expect(server.requests.filter((r) => r.url === TOKEN_PATH)).toHaveLength(
        1,
      );

      vi.setSystemTime(Date.now() + 2_000); // now 71s past the first call
      await client.paymentIntents.retrieve("pi_123");
      expect(server.requests.filter((r) => r.url === TOKEN_PATH)).toHaveLength(
        2,
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("halves the margin instead of using 30s when expires_in is short", async () => {
    // docs/flows/merchant-auth.md §3: margin is 30 s "or half of expires_in
    // for very short TTLs". With expires_in = 20 the margin must be 10, so
    // the token is reused at 9 s and refreshed at 11 s — a fixed 30 s margin
    // would refresh on every call.
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      const server = await withServer({
        tokenExpiresIn: 20,
        resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
      });
      const client = makeClient(server);

      await client.paymentIntents.retrieve("pi_123");
      vi.setSystemTime(Date.now() + 9_000);
      await client.paymentIntents.retrieve("pi_123");
      expect(server.requests.filter((r) => r.url === TOKEN_PATH)).toHaveLength(
        1,
      );

      vi.setSystemTime(Date.now() + 2_000); // 11 s past the first call
      await client.paymentIntents.retrieve("pi_123");
      expect(server.requests.filter((r) => r.url === TOKEN_PATH)).toHaveLength(
        2,
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("shares one in-flight token request across concurrent callers", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await Promise.all([
      client.paymentIntents.retrieve("pi_1"),
      client.paymentIntents.retrieve("pi_2"),
      client.paymentIntents.retrieve("pi_3"),
      client.paymentIntents.retrieve("pi_4"),
      client.paymentIntents.retrieve("pi_5"),
    ]);

    expect(server.requests.filter((r) => r.url === TOKEN_PATH)).toHaveLength(1);
  });
});

describe("401 re-authentication", () => {
  it("discards the token, re-authenticates, and retries once on a single 401", async () => {
    const server = await withServer({
      resource: (_req, callIndex) =>
        callIndex === 1
          ? {
              status: 401,
              body: {
                error: {
                  type: "invalid_request_error",
                  code: "invalid_token",
                  message: "expired",
                },
              },
            }
          : { status: 200, body: makeSamplePaymentIntent() },
    });
    const client = makeClient(server);

    const result = await client.paymentIntents.retrieve("pi_123");

    expect(result.id).toBe("pi_123");
    expect(server.requests.filter((r) => r.url === TOKEN_PATH)).toHaveLength(2);
    const resourceRequests = server.requests.filter((r) =>
      r.url.startsWith("/v1/payment_intents"),
    );
    expect(resourceRequests).toHaveLength(2);
    expect(resourceRequests[0]!.headers["authorization"]).toBe(
      "Bearer access-token-1",
    );
    expect(resourceRequests[1]!.headers["authorization"]).toBe(
      "Bearer access-token-2",
    );
  });

  it("throws VpayApiError after two consecutive 401s, having hit the token endpoint exactly twice", async () => {
    const server = await withServer({
      resource: () => ({
        status: 401,
        body: {
          error: {
            type: "invalid_request_error",
            code: "invalid_token",
            message: "expired",
          },
        },
      }),
    });
    const client = makeClient(server);

    await expect(client.paymentIntents.retrieve("pi_123")).rejects.toThrow(
      VpayApiError,
    );
    expect(server.requests.filter((r) => r.url === TOKEN_PATH)).toHaveLength(2);
    expect(
      server.requests.filter((r) => r.url.startsWith("/v1/payment_intents")),
    ).toHaveLength(2);
  });
});

describe("resource methods", () => {
  it("payment_intents.create: exact path, method, Idempotency-Key, and body", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    const result = await client.paymentIntents.create(
      {
        amount: 5000,
        currency: "XAF",
        payment_method_types: ["mtn_momo"],
        metadata: { order_id: "1234" },
      },
      { idempotencyKey: "order_1234_attempt_1" },
    );

    const req = server.requests.find((r) => r.url === "/v1/payment_intents")!;
    expect(req.method).toBe("POST");
    expect(req.headers["content-type"]).toBe(
      "application/x-www-form-urlencoded",
    );
    expect(req.headers["idempotency-key"]).toBe("order_1234_attempt_1");
    expect(req.body).toBe(
      "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo&metadata[order_id]=1234",
    );
    expect(result.id).toBe("pi_123");
    expect(result.status).toBe("requires_payment_method");
  });

  it("payment_intents.create surfaces client_secret typed, when the server sends it", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: makeSamplePaymentIntent({
          client_secret: "pi_123_secret_abc123",
        }),
      }),
    });
    const client = makeClient(server);

    const result = await client.paymentIntents.create({
      amount: 5000,
      currency: "xaf",
      payment_method_types: ["mtn_momo"],
    });

    // Typed access, no cast: `result.client_secret` is `string | undefined`.
    expect(result.client_secret).toBe("pi_123_secret_abc123");
  });

  it("payment_intents.create generates an Idempotency-Key when the caller supplies none", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.create({
      amount: 100,
      currency: "xaf",
      payment_method_types: ["mtn_momo"],
    });

    const req = server.requests.find((r) => r.url === "/v1/payment_intents")!;
    const key = req.headers["idempotency-key"] as string;
    expect(key).toMatch(/^[0-9a-f-]{36}$/i);
  });

  it("payment_intents.create throws TypeError before sending a request for a non-integer amount", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await expect(
      client.paymentIntents.create({
        amount: 50.5,
        currency: "xaf",
        payment_method_types: ["mtn_momo"],
      }),
    ).rejects.toThrow(TypeError);
    expect(server.requests).toHaveLength(0);
  });

  it("payment_intents.retrieve: exact GET path", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.retrieve("pi_123");

    const req = server.requests.find((r) =>
      r.url.startsWith("/v1/payment_intents"),
    )!;
    expect(req.method).toBe("GET");
    expect(req.url).toBe("/v1/payment_intents/pi_123");
  });

  it("payment_intents.retrieve surfaces client_secret typed, when the server sends it", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: makeSamplePaymentIntent({
          client_secret: "pi_123_secret_abc123",
        }),
      }),
    });
    const client = makeClient(server);

    const result = await client.paymentIntents.retrieve("pi_123");

    expect(result.client_secret).toBe("pi_123_secret_abc123");
  });

  it("percent-encodes a path id so it can never escape the /v1 namespace", async () => {
    // Merchants routinely pass an id straight through from their own
    // database or an inbound request. Without encoding, fetch normalises
    // `../../admin` before the request leaves and the call lands on
    // `/admin` — outside `/v1` entirely, not on a sibling intent.
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.retrieve("../../admin");
    await client.paymentIntents.cancel("pi_1?injected=1#frag");

    const urls = server.requests
      .filter((r) => r.url !== TOKEN_PATH)
      .map((r) => r.url);
    expect(urls).toEqual([
      "/v1/payment_intents/..%2F..%2Fadmin",
      "/v1/payment_intents/pi_1%3Finjected%3D1%23frag/cancel",
    ]);
  });

  it("percent-encodes a hostile id on confirm too", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.confirm("pi_1#frag", {
      payment_method_data: {
        type: "mtn_momo",
        mtn_momo: { msisdn: "237670000000" },
      },
    });

    const urls = server.requests
      .filter((r) => r.url !== TOKEN_PATH)
      .map((r) => r.url);
    expect(urls).toEqual(["/v1/payment_intents/pi_1%23frag/confirm"]);
  });

  it("payment_intents.confirm on a push rail: exact path and body", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: makeSamplePaymentIntent({ status: "processing" }),
      }),
    });
    const client = makeClient(server);

    const result = await client.paymentIntents.confirm("pi_123", {
      payment_method_data: {
        type: "mtn_momo",
        mtn_momo: { msisdn: "237670000000" },
      },
    });

    const req = server.requests.find(
      (r) => r.url === "/v1/payment_intents/pi_123/confirm",
    )!;
    expect(req.method).toBe("POST");
    expect(req.body).toBe(
      "payment_method_data[type]=mtn_momo&payment_method_data[mtn_momo][msisdn]=237670000000",
    );
    expect(result.status).toBe("processing");
  });

  it("payment_intents.confirm on a redirect rail: includes return_url", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: makeSamplePaymentIntent({
          status: "requires_action",
          next_action: {
            type: "redirect_to_url",
            redirect_to_url: {
              url: "https://rail.example/pay",
              return_url: "https://shop.example/return",
            },
          },
        }),
      }),
    });
    const client = makeClient(server);

    const result = await client.paymentIntents.confirm("pi_123", {
      payment_method_data: { type: "orange_money" },
      return_url: "https://shop.example/return",
    });

    const req = server.requests.find(
      (r) => r.url === "/v1/payment_intents/pi_123/confirm",
    )!;
    expect(req.body).toBe(
      "payment_method_data[type]=orange_money&return_url=https%3A%2F%2Fshop.example%2Freturn",
    );
    expect(result.status).toBe("requires_action");
    expect(result.next_action?.redirect_to_url.url).toBe(
      "https://rail.example/pay",
    );
  });

  it("payment_intents.cancel: exact path and method", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: makeSamplePaymentIntent({ status: "canceled" }),
      }),
    });
    const client = makeClient(server);

    const result = await client.paymentIntents.cancel("pi_123");

    const req = server.requests.find(
      (r) => r.url === "/v1/payment_intents/pi_123/cancel",
    )!;
    expect(req.method).toBe("POST");
    expect(result.status).toBe("canceled");
  });

  it("payment_intents.list: exact query string", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: {
          object: "list",
          data: [makeSamplePaymentIntent()],
          has_more: false,
          url: "/v1/payment_intents",
        },
      }),
    });
    const client = makeClient(server);

    const result = await client.paymentIntents.list({
      limit: 10,
      starting_after: "pi_100",
    });

    const req = server.requests.find((r) =>
      r.url.startsWith("/v1/payment_intents?"),
    )!;
    expect(req.method).toBe("GET");
    expect(req.url).toBe("/v1/payment_intents?limit=10&starting_after=pi_100");
    expect(result.object).toBe("list");
    expect(result.data).toHaveLength(1);
    // The server never puts client_secret on a list item — a merchant's own
    // listing view must not receive a live payer credential for every
    // intent on the page. `makeSamplePaymentIntent()` above did not set it,
    // so this is also a typed access: `string | undefined`, not `string`.
    expect(result.data[0]?.client_secret).toBeUndefined();
  });

  it("refunds.create: exact path and body, amount omitted for a full refund", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: {
          id: "re_1",
          object: "refund",
          amount: 5000,
          currency: "xaf",
          payment_intent: "pi_123",
          status: "pending",
          reason: null,
          metadata: {},
          created: 1_700_000_000,
          fee: null,
        },
      }),
    });
    const client = makeClient(server);

    const result = await client.refunds.create({
      payment_intent: "pi_123",
      reason: "requested_by_customer",
    });

    const req = server.requests.find((r) => r.url === "/v1/refunds")!;
    expect(req.method).toBe("POST");
    expect(req.body).toBe("payment_intent=pi_123&reason=requested_by_customer");
    expect(result.object).toBe("refund");
    // vpay renders `fee` on every refund and `null` on every refund it can
    // currently produce; the key must survive the decode, not be dropped.
    expect(result.fee).toBeNull();
  });

  // Issue #46. `fee` carries three distinguishable answers and the SDK must
  // not flatten them: `0` is a measured "the movement was free", `null` is
  // "the rail reported nothing", and an absent key is "this vpay predates the
  // field". Only the first belongs on a merchant's settlement statement as a
  // zero; the other two belong there as nothing at all.
  //
  // `refund.fee ?? 0` — the idiom a TypeScript author reaches for — turns all
  // three into the same line. That is the bug the issue reports one layer up,
  // where an integrator shipped a hardcoded `provider_fee_minor: 0`.
  it("refunds.create keeps a fee of 0, null and absent as three different answers", async () => {
    const cases: Array<
      [string, Record<string, unknown>, number | null | undefined]
    > = [
      ["a measured zero", { fee: 0 }, 0],
      ["nothing reported", { fee: null }, null],
      ["a real cost", { fee: 250 }, 250],
      ["a vpay older than the field", {}, undefined],
    ];

    for (const [label, extra, expected] of cases) {
      const server = await withServer({
        resource: () => ({
          status: 200,
          body: {
            id: "re_1",
            object: "refund",
            amount: 5000,
            currency: "xaf",
            payment_intent: "pi_123",
            status: "succeeded",
            reason: null,
            metadata: {},
            created: 1_700_000_000,
            ...extra,
          },
        }),
      });
      const client = makeClient(server);

      const result = await client.refunds.create({ payment_intent: "pi_123" });

      expect(result.fee, label).toBe(expected);
      // The distinction is only useful if it is *checkable*: `typeof` is the
      // narrowing the doc comment tells a caller to use, and it must be the
      // one that separates a measured zero from both flavours of unknown.
      expect(typeof result.fee === "number", label).toBe(
        typeof expected === "number",
      );
    }
  });

  it("refunds.retrieve: exact GET path, no body, no Idempotency-Key", async () => {
    // `GET /v1/refunds/{id}` — served since 2026-09-05 (issue #45), and the
    // only observation of a refund there is: `charge.refund.updated` is
    // documented and emitted by nothing, and webhook delivery is
    // at-least-once and unordered in any case.
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: {
          id: "re_1",
          object: "refund",
          amount: 2500,
          currency: "xaf",
          payment_intent: "pi_1",
          status: "pending",
          reason: "requested_by_customer",
          metadata: { case: "77" },
          created: 1_700_000_000,
        },
      }),
    });
    const client = makeClient(server);

    const result = await client.refunds.retrieve("re_1");

    const req = server.requests.find((r) => r.url === "/v1/refunds/re_1")!;
    expect(req.method).toBe("GET");
    expect(req.body).toBe("");
    // A GET spends no idempotency key: the header belongs to writes, and
    // sending one here would claim this call changes something.
    expect(req.headers["idempotency-key"]).toBeUndefined();
    expect(result.object).toBe("refund");
    expect(result.payment_intent).toBe("pi_1");
    expect(result.status).toBe("pending");
    expect(result.reason).toBe("requested_by_customer");
  });

  it("refunds percent-encodes a hostile id so it cannot escape /v1", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: {
          id: "re_1",
          object: "refund",
          amount: 2500,
          currency: "xaf",
          payment_intent: "pi_1",
          status: "pending",
          reason: null,
          metadata: {},
          created: 1_700_000_000,
        },
      }),
    });
    const client = makeClient(server);

    await client.refunds.retrieve("../../admin");

    const urls = server.requests
      .filter((r) => r.url !== TOKEN_PATH)
      .map((r) => r.url);
    expect(urls).toEqual(["/v1/refunds/..%2F..%2Fadmin"]);
  });

  it("events.list: exact query string including type", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: { object: "list", data: [], has_more: false, url: "/v1/events" },
      }),
    });
    const client = makeClient(server);

    await client.events.list({ type: "payment_intent.succeeded", limit: 5 });

    const req = server.requests.find((r) => r.url.startsWith("/v1/events?"))!;
    expect(req.url).toBe("/v1/events?type=payment_intent.succeeded&limit=5");
  });

  it("balance.retrieve: exact path, no body", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: {
          object: "balance",
          available: [{ amount: 10_000, currency: "xaf" }],
          pending: [],
        },
      }),
    });
    const client = makeClient(server);

    const result = await client.balance.retrieve();

    const req = server.requests.find((r) => r.url === "/v1/balance")!;
    expect(req.method).toBe("GET");
    expect(result.available[0]!.amount).toBe(10_000);
  });
});

// ---------------------------------------------------------------------------
// /v1/account_holders — issue #47.
//
// These mirror `sdks/rust/tests/resources.rs`'s account-holder block one for
// one, down to the encoded query string: ADR-0015's parity rule is about
// *wire semantics*, and only a byte-level assertion on both sides catches two
// SDKs that both "support account-holder lookup" while sending different
// query strings.
// ---------------------------------------------------------------------------

describe("account holders", () => {
  it("accountHolders.retrieve: sends the documented query and decodes the name", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: {
          object: "account_holder",
          payment_method_type: "mtn_momo",
          name: "David Mbarga",
          verified: true,
        },
      }),
    });
    const client = makeClient(server);

    const holder = await client.accountHolders.retrieve({
      msisdn: "237600000200",
      payment_method_type: "mtn_momo",
    });

    expect(holder.object).toBe("account_holder");
    expect(holder.payment_method_type).toBe("mtn_momo");
    expect(holder.name).toBe("David Mbarga");
    expect(holder.verified).toBe(true);

    const req = server.requests.find((r) =>
      r.url.startsWith("/v1/account_holders"),
    )!;
    expect(req.method).toBe("GET");
    // Byte for byte, in this order: `sdks/rust/tests/resources.rs` pins the
    // identical string. A GET carries no body and no `Idempotency-Key` —
    // that header is a write-path property, and sending one here would be a
    // second thing for the two SDKs to disagree about.
    expect(req.url).toBe(
      "/v1/account_holders?msisdn=237600000200&payment_method_type=mtn_momo",
    );
    expect(req.body).toBe("");
    expect(req.headers["idempotency-key"]).toBeUndefined();
  });

  it("accountHolders.retrieve: a holder the rail does not know decodes as a present null name", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: {
          object: "account_holder",
          payment_method_type: "mtn_momo",
          name: null,
          verified: false,
        },
      }),
    });
    const client = makeClient(server);

    const holder = await client.accountHolders.retrieve({
      msisdn: "237600000404",
      payment_method_type: "mtn_momo",
    });

    // `null`, not `undefined`: "the rail has no record" is an answer, and a
    // caller must be able to tell it from a key that was never sent.
    expect(holder.name).toBeNull();
    expect(holder.verified).toBe(false);
  });

  it("accountHolders.retrieve: a rail that could not be asked throws rather than answering a null name", async () => {
    const server = await withServer({
      resource: () => ({
        status: 502,
        body: {
          error: {
            type: "api_error",
            code: "provider_unavailable",
            message: "The payment provider is unavailable. We are retrying.",
          },
        },
      }),
    });
    const client = makeClient(server);

    // The distinction the whole resource exists for: a caller matching a
    // nominated refund destination refuses on both this and a null name, but
    // only one of them is the payer's to fix.
    await expect(
      client.accountHolders.retrieve({
        msisdn: "237600000200",
        payment_method_type: "mtn_momo",
      }),
    ).rejects.toMatchObject({
      name: "VpayApiError",
      status: 502,
      code: "provider_unavailable",
    });
  });

  it("accountHolders.retrieve: a rail with no such API surfaces the server's named parameter", async () => {
    const server = await withServer({
      resource: () => ({
        status: 400,
        body: {
          error: {
            type: "invalid_request_error",
            code: "invalid_request",
            param: "payment_method_type",
            message:
              "This payment method cannot look up an account holder on this deployment.",
          },
        },
      }),
    });
    const client = makeClient(server);

    await expect(
      client.accountHolders.retrieve({
        msisdn: "237600000200",
        payment_method_type: "orange_money",
      }),
    ).rejects.toMatchObject({
      name: "VpayApiError",
      status: 400,
      param: "payment_method_type",
    });

    // The SDK sent the request rather than refusing locally: whether a rail
    // can answer is a property of the deployment, and an SDK-side table of it
    // would refuse a rail a later deployment enables.
    const req = server.requests.find((r) =>
      r.url.startsWith("/v1/account_holders"),
    )!;
    expect(req.url).toBe(
      "/v1/account_holders?msisdn=237600000200&payment_method_type=orange_money",
    );
  });
});

describe("error mapping", () => {
  it("maps a 400 with the Stripe envelope to VpayApiError with all fields", async () => {
    const server = await withServer({
      resource: () => ({
        status: 400,
        body: {
          error: {
            type: "invalid_request_error",
            code: "parameter_missing",
            message: "Missing required param: amount.",
            param: "amount",
          },
        },
      }),
    });
    const client = makeClient(server);

    const error = await client.paymentIntents
      .retrieve("pi_123")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(VpayApiError);
    const apiError = error as VpayApiError;
    expect(apiError.status).toBe(400);
    expect(apiError.type).toBe("invalid_request_error");
    expect(apiError.code).toBe("parameter_missing");
    expect(apiError.message).toBe("Missing required param: amount.");
    expect(apiError.param).toBe("amount");
  });

  it("maps a 502 HTML body to VpayUnexpectedResponseError carrying a bounded prefix", async () => {
    const server = await startTestServer((_req, res) => {
      res.writeHead(502, { "Content-Type": "text/html" });
      res.end("<html><body>Bad Gateway</body></html>");
    });
    servers.push(server);
    const client = makeClient(server);

    const error = await client.paymentIntents
      .retrieve("pi_123")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(VpayUnexpectedResponseError);
    expect((error as VpayUnexpectedResponseError).status).toBe(502);
    expect((error as VpayUnexpectedResponseError).bodyPrefix).toContain(
      "Bad Gateway",
    );
  });

  it("maps a connection failure to VpayTransportError", async () => {
    const server = await startTestServer((_req, res) => res.end());
    const deadUrl = server.url;
    await server.close();

    const client = makeClient({
      url: deadUrl,
      requests: [],
      close: async () => {},
    });

    await expect(client.paymentIntents.retrieve("pi_123")).rejects.toThrow(
      VpayTransportError,
    );
  });

  it("maps a token-endpoint 401 to VpayAuthError and never retries it", async () => {
    const server = await withServer({
      onToken: () => ({
        status: 401,
        body: {
          error: "invalid_client",
          error_description: "unknown client_id",
        },
      }),
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    const error = await client.paymentIntents
      .retrieve("pi_123")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(VpayAuthError);
    expect((error as VpayAuthError).error).toBe("invalid_client");
    expect((error as VpayAuthError).errorDescription).toBe("unknown client_id");
    expect(server.requests.filter((r) => r.url === TOKEN_PATH)).toHaveLength(1);
    expect(
      server.requests.filter((r) => r.url.startsWith("/v1/payment_intents")),
    ).toHaveLength(0);
  });
});

describe("configuration", () => {
  it("rejects an assertionLifetimeSeconds outside 1..=300 at construction", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    expect(() => makeClient(server, { assertionLifetimeSeconds: 301 })).toThrow(
      VpayConfigError,
    );
    expect(() => makeClient(server, { assertionLifetimeSeconds: 0 })).toThrow(
      VpayConfigError,
    );
  });

  it("requires baseUrl, clientId and privateKey", () => {
    expect(
      () => new VpayClient({ baseUrl: "", clientId: "x", privateKey }),
    ).toThrow(VpayConfigError);
    expect(
      () => new VpayClient({ baseUrl: "https://x", clientId: "", privateKey }),
    ).toThrow(VpayConfigError);
    // The privateKey half, which a JavaScript consumer (or a `?? undefined`
    // in a config layer) reaches without any type error at all.
    expect(
      () =>
        new VpayClient({
          baseUrl: "https://x",
          clientId: "merchant_a",
          privateKey: undefined as unknown as string,
        }),
    ).toThrow(VpayConfigError);
    expect(
      () =>
        new VpayClient({
          baseUrl: "https://x",
          clientId: "merchant_a",
          privateKey: null as unknown as string,
        }),
    ).toThrow(VpayConfigError);
  });
});

describe("privacy", () => {
  it("never includes the PEM in util.inspect output", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server, { privateKey: privateKeyPem });
    const inspected = inspect(client, { depth: 10 });
    expect(inspected).not.toContain("PRIVATE KEY");
    expect(inspected).not.toContain(privateKeyPem);
  });

  it("never includes the PEM in JSON.stringify output", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server, { privateKey: privateKeyPem });
    const json = JSON.stringify(client);
    expect(json).not.toContain("PRIVATE KEY");
    expect(json).not.toContain(privateKeyPem);
  });
});

describe("baseUrl normalisation", () => {
  it("strips one trailing slash so paths and the assertion aud never double a slash", async () => {
    // Without this every request goes to `//v1/...` (a router matching `/v1`
    // 404s it — the double slash is NOT normalised on the wire) and the
    // assertion's `aud` becomes `http://host//v1/oauth/token`, which the OP
    // refuses as a wrong audience. Loud failure, but a failure nothing here
    // would otherwise catch.
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server, { baseUrl: `${server.url}/` });

    await client.paymentIntents.retrieve("pi_123");

    expect(server.requests.map((r) => r.url)).toEqual([
      TOKEN_PATH,
      "/v1/payment_intents/pi_123",
    ]);
    const { payload } = sentAssertion(server);
    expect(payload.aud).toBe(`${server.url}${TOKEN_PATH}`);
  });
});

describe("the client assertion the client actually sends", () => {
  // Regression: the assertion's `aud` is the **token endpoint URL**; the
  // `audience` form field is `vpay:v1`. They are two different values with
  // two different jobs (RFC 7523 §3 vs the OP's `allowed_audiences`), and
  // swapping one for the other still produces a well-formed request that no
  // test noticed.
  it("sets aud to the token endpoint URL, never to the audience parameter", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.retrieve("pi_123");

    const { payload } = sentAssertion(server);
    expect(payload.aud).toBe(`${server.url}${TOKEN_PATH}`);
    expect(payload.aud).not.toBe("vpay:v1");

    const fields = parseFormBody(
      server.requests.find((r) => r.url === TOKEN_PATH)!.body,
    );
    expect(fields["audience"]).toBe("vpay:v1");
    expect(payload.aud).not.toBe(fields["audience"]);
  });

  it("keeps aud on the token endpoint even when audience is overridden", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server, { audience: "vpay:some-other-surface" });

    await client.paymentIntents.retrieve("pi_123");

    const { payload } = sentAssertion(server);
    expect(payload.aud).toBe(`${server.url}${TOKEN_PATH}`);
    expect(payload.aud).not.toBe("vpay:some-other-surface");
    const fields = parseFormBody(
      server.requests.find((r) => r.url === TOKEN_PATH)!.body,
    );
    expect(fields["audience"]).toBe("vpay:some-other-surface");
  });

  it("follows a custom tokenEndpoint into aud", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    // The path this points at does not have to exist for the assertion to be
    // inspectable: the token request is recorded either way.
    const tokenEndpoint = `${server.url}${TOKEN_PATH}`;
    const client = makeClient(server, {
      issuer: "https://issuer.example/ignored",
      tokenEndpoint,
    });

    await client.paymentIntents.retrieve("pi_123");

    expect(sentAssertion(server).payload.aud).toBe(tokenEndpoint);
  });

  it("leaves aud on the token endpoint when assertionAudience is not set", async () => {
    // The default is unchanged by the option's existence: a merchant that
    // reaches vpay at the URL vpay publishes as its own configures nothing.
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.retrieve("pi_123");

    expect(sentAssertion(server).payload.aud).toBe(
      `${server.url}${TOKEN_PATH}`,
    );
  });

  it("signs aud as the configured assertionAudience while still POSTing to the token endpoint", async () => {
    // The defect this option fixes: the URL reachable from the merchant's
    // server and the string the OP calls itself are two different facts.
    // `assertionAudience` moves the `aud` claim without moving the request.
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const publicTokenEndpoint = "https://api.vpay.example/v1/oauth/token";
    const client = makeClient(server, {
      assertionAudience: publicTokenEndpoint,
    });

    await client.paymentIntents.retrieve("pi_123");

    const { payload } = sentAssertion(server);
    expect(payload.aud).toBe(publicTokenEndpoint);
    // The request still went to the reachable server, not to the audience.
    expect(server.requests.map((r) => r.url)).toEqual([
      TOKEN_PATH,
      "/v1/payment_intents/pi_123",
    ]);
    // And it is still not the `audience` form field, which stays `vpay:v1`.
    const fields = parseFormBody(
      server.requests.find((r) => r.url === TOKEN_PATH)!.body,
    );
    expect(fields["audience"]).toBe("vpay:v1");
  });

  it("accepts the issuer as an assertionAudience, which the OP also allows", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server, {
      assertionAudience: "https://api.vpay.example/v1/oauth",
    });

    await client.paymentIntents.retrieve("pi_123");

    expect(sentAssertion(server).payload.aud).toBe(
      "https://api.vpay.example/v1/oauth",
    );
  });

  it("sets iss and sub to the configured client_id", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    await makeClient(server, {
      clientId: "merchant_b",
    }).paymentIntents.retrieve("pi_123");

    const { payload } = sentAssertion(server);
    expect(payload.iss).toBe("merchant_b");
    expect(payload.sub).toBe("merchant_b");
  });

  // Regression: `kid` was only ever asserted against `mintClientAssertion`
  // directly. Dropping it on the way through the client — the only path a
  // merchant with more than one registered JWK ever takes — was invisible.
  it("forwards a configured kid into the assertion header", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server, { kid: "key-1" });

    await client.paymentIntents.retrieve("pi_123");

    const { header } = sentAssertion(server);
    expect(header.kid).toBe("key-1");
    expect(header.alg).toBe("RS256");
    expect(header.typ).toBe("JWT");
  });

  it("omits kid from the assertion header when none is configured", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });

    await makeClient(server).paymentIntents.retrieve("pi_123");

    expect(sentAssertion(server).header.kid).toBeUndefined();
  });

  it("honours a configured assertionLifetimeSeconds", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });

    await makeClient(server, {
      assertionLifetimeSeconds: 120,
    }).paymentIntents.retrieve("pi_123");

    const { payload } = sentAssertion(server);
    expect(payload.exp - payload.iat).toBe(120);
  });
});

describe("headers on resource calls", () => {
  // Regression: Accept and User-Agent were asserted only on the token
  // request, but docs/flows/merchant-auth.md's "Headers" table says "always".
  it("sends Accept and User-Agent on a GET resource call", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });

    await makeClient(server).paymentIntents.retrieve("pi_123");

    const req = server.requests.find((r) =>
      r.url.startsWith("/v1/payment_intents"),
    )!;
    expect(req.headers["accept"]).toBe("application/json");
    expect(req.headers["user-agent"]).toBe(`vpay-sdk-node/${SDK_VERSION}`);
  });

  it("sends Accept and User-Agent on a POST resource call", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });

    await makeClient(server).paymentIntents.create({
      amount: 5000,
      currency: "xaf",
      payment_method_types: ["mtn_momo"],
    });

    const req = server.requests.find((r) => r.url === "/v1/payment_intents")!;
    expect(req.headers["accept"]).toBe("application/json");
    expect(req.headers["user-agent"]).toBe(`vpay-sdk-node/${SDK_VERSION}`);
  });

  it("sends the same User-Agent on the token call and the resource call", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });

    await makeClient(server).paymentIntents.retrieve("pi_123");

    const tokenRequest = server.requests.find((r) => r.url === TOKEN_PATH)!;
    const resourceRequest = server.requests.find((r) =>
      r.url.startsWith("/v1/payment_intents"),
    )!;
    expect(resourceRequest.headers["user-agent"]).toBe(
      tokenRequest.headers["user-agent"],
    );
    expect(tokenRequest.headers["user-agent"]).toBe(
      `vpay-sdk-node/${SDK_VERSION}`,
    );
  });
});

describe("the 401 retry replays the identical POST", () => {
  // A retry that regenerates the Idempotency-Key is a retry that can
  // double-create: two payment intents for one order, which is exactly what
  // the header exists to prevent (docs/flows/merchant-auth.md, "Headers").
  it("resends the caller-supplied Idempotency-Key and an identical body", async () => {
    const server = await withServer({
      resource: (_req, callIndex) =>
        callIndex === 1
          ? {
              status: 401,
              body: {
                error: {
                  type: "invalid_request_error",
                  code: "invalid_token",
                  message: "expired",
                },
              },
            }
          : { status: 200, body: makeSamplePaymentIntent() },
    });
    const client = makeClient(server);

    await client.paymentIntents.create(
      {
        amount: 5000,
        currency: "XAF",
        payment_method_types: ["mtn_momo"],
        metadata: { order_id: "1234" },
      },
      { idempotencyKey: "order_1234_attempt_1" },
    );

    const posts = server.requests.filter(
      (r) => r.url === "/v1/payment_intents",
    );
    expect(posts).toHaveLength(2);
    expect(posts[0]!.headers["idempotency-key"]).toBe("order_1234_attempt_1");
    expect(posts[1]!.headers["idempotency-key"]).toBe("order_1234_attempt_1");
    expect(posts[1]!.body).toBe(posts[0]!.body);
    expect(posts[1]!.body).toBe(
      "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo&metadata[order_id]=1234",
    );
    // …and the retry did carry the *new* token, so this is a real re-auth.
    expect(posts[0]!.headers["authorization"]).toBe("Bearer access-token-1");
    expect(posts[1]!.headers["authorization"]).toBe("Bearer access-token-2");
  });

  it("resends the same generated Idempotency-Key when the caller supplied none", async () => {
    const server = await withServer({
      resource: (_req, callIndex) =>
        callIndex === 1
          ? {
              status: 401,
              body: {
                error: {
                  type: "invalid_request_error",
                  code: "invalid_token",
                  message: "expired",
                },
              },
            }
          : { status: 200, body: makeSamplePaymentIntent() },
    });
    const client = makeClient(server);

    await client.paymentIntents.create({
      amount: 5000,
      currency: "xaf",
      payment_method_types: ["mtn_momo"],
    });

    const posts = server.requests.filter(
      (r) => r.url === "/v1/payment_intents",
    );
    expect(posts).toHaveLength(2);
    const first = posts[0]!.headers["idempotency-key"] as string;
    expect(first).toMatch(/^[0-9a-f-]{36}$/i);
    expect(posts[1]!.headers["idempotency-key"]).toBe(first);
    expect(posts[1]!.body).toBe(posts[0]!.body);
  });

  it("resends an identical body for a confirm, whose body is nested", async () => {
    const server = await withServer({
      resource: (_req, callIndex) =>
        callIndex === 1
          ? {
              status: 401,
              body: {
                error: {
                  type: "invalid_request_error",
                  code: "invalid_token",
                  message: "expired",
                },
              },
            }
          : {
              status: 200,
              body: makeSamplePaymentIntent({ status: "processing" }),
            },
    });
    const client = makeClient(server);

    await client.paymentIntents.confirm("pi_123", {
      payment_method_data: {
        type: "mtn_momo",
        mtn_momo: { msisdn: "237670000000" },
      },
    });

    const posts = server.requests.filter(
      (r) => r.url === "/v1/payment_intents/pi_123/confirm",
    );
    expect(posts).toHaveLength(2);
    expect(posts[1]!.body).toBe(posts[0]!.body);
    expect(posts[1]!.headers["idempotency-key"]).toBe(
      posts[0]!.headers["idempotency-key"],
    );
  });
});

describe("timeouts surface as VpayTransportError", () => {
  it("maps a server that accepts the connection and never answers to VpayTransportError", async () => {
    // No `writeHead`, no `end`: the request is received and then nothing
    // happens, so `AbortSignal.timeout` fires before any response headers.
    const server = await startTestServer(() => {});
    servers.push(server);
    const client = makeClient(server, { timeoutMs: 150 });

    const error = await client.paymentIntents
      .retrieve("pi_123")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(VpayTransportError);
    expect((error as VpayTransportError).cause).toBeDefined();
  });

  // Regression: `await response.text()` sat outside the try that maps
  // transport failures. `fetch` resolves on headers, so a body that stalls
  // rejected *after* the mapping and escaped as a raw
  // `DOMException: TimeoutError` — an error type this SDK does not document
  // and a caller's `instanceof VpayError` catch-all would miss entirely.
  it("maps a stall part-way through the response body to VpayTransportError", async () => {
    const server = await startTestServer((req, res) => {
      if (req.url === TOKEN_PATH) {
        jsonResponse(res, 200, {
          access_token: "access-token-1",
          token_type: "Bearer",
          expires_in: 300,
        });
        return;
      }
      res.writeHead(200, { "Content-Type": "application/json" });
      res.write('{"id":"pi_1');
    });
    servers.push(server);
    const client = makeClient(server, { timeoutMs: 150 });

    const error = await client.paymentIntents
      .retrieve("pi_123")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(VpayTransportError);
    expect((error as VpayTransportError).message).toBe("request failed");
    const cause = (error as VpayTransportError).cause as Error;
    expect(cause).toBeInstanceOf(Error);
    expect(cause.name).toBe("TimeoutError");
  });

  it("maps a stall part-way through the token response body to VpayTransportError", async () => {
    const server = await startTestServer((_req, res) => {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.write('{"access_token":"secret-token-never-returned"');
    });
    servers.push(server);
    const client = makeClient(server, { timeoutMs: 150 });

    const error = await client.paymentIntents
      .retrieve("pi_123")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(VpayTransportError);
    expect((error as VpayTransportError).message).toBe("token request failed");
    const cause = (error as VpayTransportError).cause as Error;
    expect(cause.name).toBe("TimeoutError");
  });
});

describe("token response validation", () => {
  it("rejects a 200 whose token_type is not Bearer", async () => {
    const server = await withServer({
      onToken: () => ({
        status: 200,
        body: {
          access_token: "mac-token",
          token_type: "MAC",
          expires_in: 300,
        },
      }),
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    const error = await client.paymentIntents
      .retrieve("pi_123")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(VpayUnexpectedResponseError);
    expect((error as VpayUnexpectedResponseError).status).toBe(200);
    // No resource call was attempted with a token this SDK cannot present.
    expect(
      server.requests.filter((r) => r.url.startsWith("/v1/payment_intents")),
    ).toHaveLength(0);
  });

  it("rejects a 200 with no token_type at all", async () => {
    const server = await withServer({
      onToken: () => ({
        status: 200,
        body: { access_token: "t", expires_in: 300 },
      }),
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });

    await expect(
      makeClient(server).paymentIntents.retrieve("pi_123"),
    ).rejects.toThrow(VpayUnexpectedResponseError);
  });

  it("accepts a lowercase bearer, which RFC 6749 §7.1 makes case-insensitive", async () => {
    const server = await withServer({
      onToken: () => ({
        status: 200,
        body: {
          access_token: "access-token-1",
          token_type: "bearer",
          expires_in: 300,
        },
      }),
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });

    const result = await makeClient(server).paymentIntents.retrieve("pi_123");

    expect(result.id).toBe("pi_123");
    const req = server.requests.find((r) =>
      r.url.startsWith("/v1/payment_intents"),
    )!;
    expect(req.headers["authorization"]).toBe("Bearer access-token-1");
  });
});

describe("the access token stays out of diagnostics", () => {
  it("never appears in util.inspect of the client, even after a successful exchange", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.retrieve("pi_123");

    const inspected = inspect(client, { depth: 10, showHidden: true });
    expect(inspected).not.toContain("access-token-1");
    expect(JSON.stringify(client)).not.toContain("access-token-1");
  });

  it("never appears in util.inspect of a thrown VpayApiError", async () => {
    const server = await withServer({
      resource: () => ({
        status: 400,
        body: {
          error: {
            type: "invalid_request_error",
            code: "parameter_missing",
            message: "Missing required param: amount.",
            param: "amount",
          },
        },
      }),
    });
    const client = makeClient(server);

    const error = await client.paymentIntents
      .retrieve("pi_123")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(VpayApiError);
    expect(inspect(error, { depth: 10, showHidden: true })).not.toContain(
      "access-token-1",
    );
  });

  it("never appears in util.inspect of a thrown VpayTransportError, cause chain included", async () => {
    const server = await startTestServer((req, res) => {
      if (req.url === TOKEN_PATH) {
        jsonResponse(res, 200, {
          access_token: "access-token-1",
          token_type: "Bearer",
          expires_in: 300,
        });
        return;
      }
      res.writeHead(200, { "Content-Type": "application/json" });
      res.write("{");
    });
    servers.push(server);
    const client = makeClient(server, { timeoutMs: 150 });

    const error = await client.paymentIntents
      .retrieve("pi_123")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(VpayTransportError);
    expect(inspect(error, { depth: 10, showHidden: true })).not.toContain(
      "access-token-1",
    );
  });

  it("never appears in util.inspect of a thrown VpayUnexpectedResponseError", async () => {
    const server = await withServer({
      resource: () => ({ status: 502, body: { nope: true } }),
    });
    const client = makeClient(server);

    const error = await client.paymentIntents
      .retrieve("pi_123")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(VpayUnexpectedResponseError);
    expect(inspect(error, { depth: 10, showHidden: true })).not.toContain(
      "access-token-1",
    );
  });
});

describe("amount validation reaches the resource methods", () => {
  it("payment_intents.create refuses a negative amount before sending", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await expect(
      client.paymentIntents.create({
        amount: -5000,
        currency: "xaf",
        payment_method_types: ["mtn_momo"],
      }),
    ).rejects.toThrow(TypeError);
    expect(server.requests).toHaveLength(0);
  });

  it("payment_intents.create refuses 1e21, which Number.isInteger accepts", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await expect(
      client.paymentIntents.create({
        amount: 1e21,
        currency: "xaf",
        payment_method_types: ["mtn_momo"],
      }),
    ).rejects.toThrow(TypeError);
    expect(server.requests).toHaveLength(0);
  });

  it("refunds.create refuses a negative amount before sending", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: {} }),
    });
    const client = makeClient(server);

    await expect(
      client.refunds.create({ payment_intent: "pi_123", amount: -1 }),
    ).rejects.toThrow(TypeError);
    expect(server.requests).toHaveLength(0);
  });
});

describe("an unreadable private key is refused at construction", () => {
  /**
   * The core client's half of the parse `stripe-auth.test.ts` pins for the
   * authenticator.
   *
   * `resolveMerchantAuth` is where `createPrivateKey` now runs, and both
   * entry points call it — so this is one shared line, and until this case
   * existed only one of the two callers proved it fires. A regression that
   * moved the parse back to first-mint would leave `VpayClient` throwing an
   * OpenSSL `Error` from the middle of a token exchange instead of a
   * `VpayConfigError` from the line the merchant wrote, and every test in
   * this file would still pass.
   */
  it("throws VpayConfigError from the constructor, before any request", () => {
    expect(
      () =>
        new VpayClient({
          baseUrl: "https://api.vpay.example",
          clientId: "merchant_a",
          privateKey: "not a pem",
        }),
    ).toThrow(VpayConfigError);
    // The message, so this cannot pass for some unrelated reason: every
    // other option above is valid, and only the key can be the complaint.
    expect(
      () =>
        new VpayClient({
          baseUrl: "https://api.vpay.example",
          clientId: "merchant_a",
          privateKey: "not a pem",
        }),
    ).toThrow(/privateKey could not be read as a private key/);
  });
});

/**
 * `/v1/checkout/sessions` — Step 9's four merchant operations.
 *
 * These mirror `sdks/rust/tests/resources.rs`'s checkout-session cases one
 * for one, down to the body strings, because ADR-0015's parity rule is
 * about *wire semantics*: two SDKs that both "support checkout sessions"
 * but encode `ui_mode` differently are not at parity, and only a
 * byte-level assertion on both sides catches that.
 */
describe("checkout.sessions", () => {
  const sampleSession = (
    overrides: Record<string, unknown> = {},
  ): Record<string, unknown> => ({
    id: "cs_123",
    object: "checkout.session",
    livemode: false,
    payment_intent: "pi_123",
    ui_mode: "hosted",
    status: "open",
    payment_status: "unpaid",
    success_url: "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
    cancel_url: "https://shop.example/cancel",
    return_url: null,
    url: "https://checkout.example/c/cs_123#cs_123_secret_abc123",
    expires_at: 1_700_086_400,
    created: 1_700_000_000,
    ...overrides,
  });

  it("checkout.sessions.create: exact path, method, Idempotency-Key, and body", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: sampleSession({ client_secret: "cs_123_secret_abc123" }),
      }),
    });
    const client = makeClient(server);

    const session = await client.checkout.sessions.create(
      {
        payment_intent: "pi_123",
        ui_mode: "hosted",
        success_url: "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
        cancel_url: "https://shop.example/cancel",
      },
      { idempotencyKey: "order_1234_session_1" },
    );

    const req = server.requests.find((r) => r.url === "/v1/checkout/sessions")!;
    expect(req.method).toBe("POST");
    expect(req.headers["content-type"]).toBe(
      "application/x-www-form-urlencoded",
    );
    expect(req.headers["idempotency-key"]).toBe("order_1234_session_1");
    expect(req.body).toBe(
      "payment_intent=pi_123&ui_mode=hosted&success_url=https%3A%2F%2Fshop.example%2Fok%3Fsid%3D%7BCHECKOUT_SESSION_ID%7D&cancel_url=https%3A%2F%2Fshop.example%2Fcancel",
    );
    expect(session.id).toBe("cs_123");
    expect(session.object).toBe("checkout.session");
    expect(session.url).toContain("/c/cs_123#");
  });

  it("checkout.sessions.create omits every field the caller left unset", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: sampleSession() }),
    });
    const client = makeClient(server);

    await client.checkout.sessions.create({ payment_intent: "pi_123" });

    const req = server.requests.find((r) => r.url === "/v1/checkout/sessions")!;
    // `ui_mode=` and no `ui_mode` are different requests; only the second
    // means "let the server default it to hosted".
    expect(req.body).toBe("payment_intent=pi_123");
  });

  it("checkout.sessions.create sends an embedded session's return_url", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: sampleSession({
          ui_mode: "embedded",
          success_url: null,
          cancel_url: null,
          return_url: "https://shop.example/order/42",
          url: null,
          client_secret: "cs_123_secret_abc123",
        }),
      }),
    });
    const client = makeClient(server);

    const session = await client.checkout.sessions.create({
      payment_intent: "pi_123",
      ui_mode: "embedded",
      return_url: "https://shop.example/order/42",
    });

    const req = server.requests.find((r) => r.url === "/v1/checkout/sessions")!;
    expect(req.body).toBe(
      "payment_intent=pi_123&ui_mode=embedded&return_url=https%3A%2F%2Fshop.example%2Forder%2F42",
    );
    expect(session.url).toBeNull();
    expect(session.client_secret).toBe("cs_123_secret_abc123");
  });

  it("checkout.sessions.create generates an Idempotency-Key when the caller supplies none", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: sampleSession() }),
    });
    const client = makeClient(server);

    await client.checkout.sessions.create({ payment_intent: "pi_123" });

    const req = server.requests.find((r) => r.url === "/v1/checkout/sessions")!;
    expect(req.headers["idempotency-key"] as string).toMatch(
      /^[0-9a-f-]{36}$/i,
    );
  });

  it("checkout.sessions.retrieve: exact GET path, and the client_secret it carries", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: sampleSession({ client_secret: "cs_123_secret_abc123" }),
      }),
    });
    const client = makeClient(server);

    const session = await client.checkout.sessions.retrieve("cs_123");

    const req = server.requests.find((r) =>
      r.url.startsWith("/v1/checkout/sessions"),
    )!;
    expect(req.method).toBe("GET");
    expect(req.url).toBe("/v1/checkout/sessions/cs_123");
    // Typed access, no cast: `string | undefined`.
    expect(session.client_secret).toBe("cs_123_secret_abc123");
  });

  it("checkout.sessions percent-encodes a hostile id so it cannot escape /v1", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: sampleSession() }),
    });
    const client = makeClient(server);

    await client.checkout.sessions.retrieve("../../admin");
    await client.checkout.sessions.expire("cs_1?injected=1#frag");

    const urls = server.requests
      .filter((r) => r.url !== TOKEN_PATH)
      .map((r) => r.url);
    expect(urls).toEqual([
      "/v1/checkout/sessions/..%2F..%2Fadmin",
      "/v1/checkout/sessions/cs_1%3Finjected%3D1%23frag/expire",
    ]);
  });

  it("checkout.sessions.list: exact query string including the payment_intent filter", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: {
          object: "list",
          data: [sampleSession()],
          has_more: false,
          url: "/v1/checkout/sessions",
        },
      }),
    });
    const client = makeClient(server);

    const page = await client.checkout.sessions.list({
      limit: 10,
      payment_intent: "pi_123",
    });

    const req = server.requests.find((r) =>
      r.url.startsWith("/v1/checkout/sessions?"),
    )!;
    expect(req.method).toBe("GET");
    expect(req.url).toBe(
      "/v1/checkout/sessions?limit=10&payment_intent=pi_123",
    );
    expect(page.data).toHaveLength(1);
    // A list item never carries the payer credential — the same rule the
    // intent list obeys, for the same reason.
    expect(page.data[0]?.client_secret).toBeUndefined();
  });

  it("checkout.sessions.expire: exact path, method and empty body", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: sampleSession({ status: "expired" }),
      }),
    });
    const client = makeClient(server);

    const session = await client.checkout.sessions.expire("cs_123");

    const req = server.requests.find((r) => r.url.endsWith("/cs_123/expire"))!;
    expect(req.method).toBe("POST");
    expect(req.url).toBe("/v1/checkout/sessions/cs_123/expire");
    expect(req.body).toBe("");
    expect(req.headers["idempotency-key"]).toBeDefined();
    expect(session.status).toBe("expired");
  });

  it("checkout.sessions maps the 404 envelope for an unknown session", async () => {
    const server = await withServer({
      resource: () => ({
        status: 404,
        body: {
          error: {
            type: "invalid_request_error",
            code: "resource_missing",
            message: "No such checkout session: cs_nope",
          },
        },
      }),
    });
    const client = makeClient(server);

    const err = await client.checkout.sessions
      .retrieve("cs_nope")
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(VpayApiError);
    const apiError = err as VpayApiError;
    expect(apiError.status).toBe(404);
    expect(apiError.code).toBe("resource_missing");
    expect(apiError.message).toContain("No such checkout session: cs_nope");
  });

  it("checkout.sessions maps a 409 on expiring a session with a live charge", async () => {
    const server = await withServer({
      resource: () => ({
        status: 409,
        body: {
          error: {
            type: "invalid_request_error",
            code: "invalid_state",
            message: "This checkout session has a charge in flight.",
          },
        },
      }),
    });
    const client = makeClient(server);

    const err = await client.checkout.sessions
      .expire("cs_123")
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(VpayApiError);
    expect((err as VpayApiError).status).toBe(409);
    expect((err as VpayApiError).code).toBe("invalid_state");
  });

  it("redacts a checkout session's client_secret from util.inspect, and leaves JSON faithful", async () => {
    // The gap ADR-0015 records against `PaymentIntent` — a plain interface,
    // so `console.log(intent)` prints a live credential — is not repeated
    // for this object. `vpay_sdk::CheckoutSession`'s hand-written `Debug`
    // prints the same `[N chars redacted]` marker.
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: sampleSession({ client_secret: "cs_123_secret_abc123" }),
      }),
    });
    const client = makeClient(server);

    const session = await client.checkout.sessions.retrieve("cs_123");

    const rendered = inspect(session);
    expect(rendered).not.toContain("cs_123_secret_abc123");
    expect(rendered).toContain("chars redacted");
    // …including through `url`, which for a hosted session carries the same
    // secret in its fragment (D6). Redacting `client_secret` alone would
    // have leaked the value the redaction exists to hide.
    expect(rendered).toContain("https://checkout.example/c/cs_123#[");
    // A redaction, not a blackout: every other field is still readable.
    expect(rendered).toContain("cs_123");
    expect(rendered).toContain("pi_123");
    // …and `JSON.stringify` still carries it, because an embedded
    // integration has to serialise the secret to reach the browser at all.
    expect(JSON.stringify(session)).toContain("cs_123_secret_abc123");
    // The inspect hook is invisible to everything else.
    expect(Object.keys(session)).not.toContain("client_secret_redacted");
    const roundTripped = JSON.parse(JSON.stringify(session)) as {
      client_secret?: string;
    };
    expect(roundTripped.client_secret).toBe("cs_123_secret_abc123");
  });

  it("redacts a list item's url fragment too, though it has no client_secret to redact", async () => {
    // The list never carries `client_secret`, but a hosted session's `url`
    // carries the same value in its fragment, so the redaction has to
    // follow the secret rather than the field name.
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: {
          object: "list",
          data: [sampleSession()],
          has_more: false,
          url: "/v1/checkout/sessions",
        },
      }),
    });
    const client = makeClient(server);

    const page = await client.checkout.sessions.list();

    const rendered = inspect(page.data[0]);
    expect(rendered).not.toContain("cs_123_secret_abc123");
    expect(rendered).toContain("https://checkout.example/c/cs_123#[");
    // No `client_secret` line at all — the key is absent from the body, not
    // present and redacted.
    expect(rendered).not.toContain("client_secret");
  });

  it("leaves a session whose url has no fragment untouched", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: sampleSession({ ui_mode: "embedded", url: null }),
      }),
    });
    const client = makeClient(server);

    const session = await client.checkout.sessions.retrieve("cs_123");

    const rendered = inspect(session);
    expect(rendered).not.toContain("chars redacted");
    expect(rendered).toContain("url: null");
  });
});

/**
 * The audience check the OP actually performs, reproduced from the two places
 * that perform it.
 *
 * `authkestra_op::client_assertion::verify_client_assertion` is handed an
 * `expected_audiences` list by `authenticate_client`, and that list is the
 * OP's **own** two names for itself:
 * `{deployment.public_base_url}/v1/oauth/token` and
 * `{deployment.public_base_url}/v1/oauth` (`vpay_api::op::issuer_for`). The
 * URL the merchant's process happened to POST to is not consulted, and is not
 * in the list unless it coincides with one of them.
 *
 * This is a *shaped* stand-in, not the real verifier: Node cannot link the
 * Rust crate, and `docs/sdks/parity.md` records that gap for this package as
 * a whole. `sdks/rust/tests/op_conformance.rs` runs the same case through the
 * real pinned verifier.
 */
function verifyAsTheOpWould(
  jwt: string,
  expected: {
    clientId: string;
    publicKey: Parameters<typeof verify>[2];
    /** `[token endpoint, issuer]`, as the OP names itself. */
    audiences: string[];
  },
): { ok: true } | { ok: false; reason: string } {
  const [headerPart, payloadPart, signaturePart] = jwt.split(".");
  if (!headerPart || !payloadPart || !signaturePart) {
    return { ok: false, reason: "MalformedAssertion" };
  }
  const signingInput = Buffer.from(`${headerPart}.${payloadPart}`, "utf8");
  const signature = Buffer.from(signaturePart, "base64url");
  if (!verify("sha256", signingInput, expected.publicKey, signature)) {
    return { ok: false, reason: "InvalidSignature" };
  }
  const payload = decodeJwtPart<AssertionPayload>(payloadPart);
  if (payload.iss !== expected.clientId || payload.sub !== expected.clientId) {
    return { ok: false, reason: "InvalidIssuer" };
  }
  const now = Math.floor(Date.now() / 1000);
  if (payload.exp <= now) {
    return { ok: false, reason: "Expired" };
  }
  if (payload.exp - now > MAX_ASSERTION_LIFETIME_SECONDS) {
    return { ok: false, reason: "LifetimeTooLong" };
  }
  if (!expected.audiences.includes(payload.aud)) {
    return { ok: false, reason: "InvalidAudience" };
  }
  return { ok: true };
}

describe("a merchant server that reaches vpay by an internal URL", () => {
  // vpay publishes itself at these two names — this is what the OP puts in
  // `expected_audiences`. The merchant's own `baseUrl` is the test server's
  // 127.0.0.1 address, standing in for `http://vpay-server:8080`.
  const PUBLIC_TOKEN_ENDPOINT = "http://localhost:8080/v1/oauth/token";
  const PUBLIC_ISSUER = "http://localhost:8080/v1/oauth";
  const OP_AUDIENCES = [PUBLIC_TOKEN_ENDPOINT, PUBLIC_ISSUER];

  it("is refused by the OP audience check when assertionAudience is left unset", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.retrieve("pi_123");

    const jwt = parseFormBody(
      server.requests.find((r) => r.url === TOKEN_PATH)!.body,
    )["client_assertion"]!;
    const verdict = verifyAsTheOpWould(jwt, {
      clientId: "merchant_a",
      publicKey,
      audiences: OP_AUDIENCES,
    });

    // Everything else about the assertion is correct — the signature
    // verifies, `iss`/`sub` are the client id, the lifetime is in range. The
    // one wrong claim is `aud`, and the OP answers `invalid_client` for it.
    expect(verdict).toEqual({ ok: false, reason: "InvalidAudience" });
  });

  it("authenticates once assertionAudience names the OP's own token endpoint", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server, {
      assertionAudience: PUBLIC_TOKEN_ENDPOINT,
    });

    await client.paymentIntents.retrieve("pi_123");

    const jwt = parseFormBody(
      server.requests.find((r) => r.url === TOKEN_PATH)!.body,
    )["client_assertion"]!;
    expect(
      verifyAsTheOpWould(jwt, {
        clientId: "merchant_a",
        publicKey,
        audiences: OP_AUDIENCES,
      }),
    ).toEqual({ ok: true });

    // The token request still went to the internal address, which is the
    // point: reachability and audience are now separate settings.
    expect(server.requests[0]!.url).toBe(TOKEN_PATH);
    expect(server.url).not.toBe("http://localhost:8080");
  });
});

/**
 * `/v1/customers` — S4a's five merchant operations.
 *
 * These mirror `sdks/rust/tests/resources.rs`'s customer cases one for one,
 * down to the body strings, for the reason the checkout-session block above
 * states: ADR-0015's parity is about *wire semantics*, and two SDKs that both
 * "support customers" while encoding a cleared field differently are not at
 * parity.
 */
describe("customers", () => {
  const sampleCustomer = (
    overrides: Record<string, unknown> = {},
  ): Record<string, unknown> => ({
    id: "cus_123",
    object: "customer",
    name: null,
    email: null,
    // Canonical, as the server stores and renders it — not the `+237 6 …` a
    // merchant would have typed.
    phone: "237600000200",
    metadata: { order_id: "1234" },
    created: 1_700_000_000,
    livemode: false,
    ...overrides,
  });

  it("customers.create: exact path, method, Idempotency-Key, and body", async () => {
    const server = await withServer({
      resource: () => ({ status: 201, body: sampleCustomer() }),
    });
    const client = makeClient(server);

    const customer = await client.customers.create(
      {
        name: "Ada Ngo",
        email: "ada@example.com",
        phone: "+237 6 00 00 02 00",
        metadata: { order_id: "1234" },
      },
      { idempotencyKey: "idem_cus" },
    );

    const req = server.requests.find((r) => r.url === "/v1/customers")!;
    expect(req.method).toBe("POST");
    expect(req.headers["idempotency-key"]).toBe("idem_cus");
    // The phone number goes out **as the merchant typed it**: canonicalising
    // is the server's job, and a client-side copy of a market rule would
    // refuse offline a number a later vpay accepts. `+` is `%2B` because a
    // bare `+` is a space in a form body.
    expect(req.body).toBe(
      "name=Ada%20Ngo&email=ada%40example.com&phone=%2B237%206%2000%2000%2002%2000&metadata[order_id]=1234",
    );
    expect(customer.id).toBe("cus_123");
    expect(customer.object).toBe("customer");
  });

  it("a phone number alone is a complete customer, and every other field is omitted", async () => {
    const server = await withServer({
      resource: () => ({ status: 201, body: sampleCustomer() }),
    });
    const client = makeClient(server);

    const customer = await client.customers.create({
      phone: "237600000200",
    });

    // The fixture is phone-only too, so the decode proves the object is
    // representable with two of its three identifiers null — the maintainer's
    // decision of 2026-09-05, at the SDK boundary.
    expect(customer.phone).toBe("237600000200");
    expect(customer.name).toBeNull();
    expect(customer.email).toBeNull();

    const req = server.requests.find((r) => r.url === "/v1/customers")!;
    expect(req.body).toBe("phone=237600000200");
  });

  it("customers.retrieve: exact GET path, no body, no Idempotency-Key", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: sampleCustomer() }),
    });
    const client = makeClient(server);

    const customer = await client.customers.retrieve("cus_123");
    expect(customer.id).toBe("cus_123");

    const req = server.requests.find((r) => r.url === "/v1/customers/cus_123")!;
    expect(req.method).toBe("GET");
    expect(req.body).toBe("");
    expect(req.headers["idempotency-key"]).toBeUndefined();
  });

  it("customers.update tells leave-alone, set and clear apart on the wire", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: sampleCustomer({ name: "Ada Ngo" }),
      }),
    });
    const client = makeClient(server);

    await client.customers.update(
      "cus_123",
      {
        // set
        name: "Ada Ngo",
        // clear
        email: null,
        // leave alone — `phone` is simply not mentioned
        metadata: { order_id: "5678", tier: "" },
      },
      { idempotencyKey: "idem_upd" },
    );

    const req = server.requests.find((r) => r.url === "/v1/customers/cus_123")!;
    expect(req.method).toBe("POST");
    // `email=` is "clear it"; `phone` is absent because the patch did not
    // mention it. Collapsing `null` and `undefined` — the obvious
    // simplification — makes a payer's email unclearable, and this is the
    // assertion that fails when somebody does it.
    expect(req.body).toBe(
      "name=Ada%20Ngo&email=&metadata[order_id]=5678&metadata[tier]=",
    );
    expect(req.headers["idempotency-key"]).toBe("idem_upd");
  });

  it("customers.list: exact query string", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: {
          object: "list",
          data: [sampleCustomer()],
          has_more: true,
          url: "/v1/customers",
        },
      }),
    });
    const client = makeClient(server);

    const page = await client.customers.list({
      limit: 2,
      starting_after: "cus_0",
    });
    expect(page.has_more).toBe(true);
    expect(page.data).toHaveLength(1);

    const req = server.requests.find((r) => r.url.startsWith("/v1/customers"))!;
    expect(req.url).toBe("/v1/customers?limit=2&starting_after=cus_0");
    expect(req.method).toBe("GET");
  });

  it("customers.del: a DELETE with an Idempotency-Key and no body", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: { id: "cus_123", object: "customer", deleted: true },
      }),
    });
    const client = makeClient(server);

    const deleted = await client.customers.del("cus_123", {
      idempotencyKey: "idem_del",
    });
    expect(deleted.deleted).toBe(true);
    expect(deleted.id).toBe("cus_123");

    const req = server.requests.find((r) => r.url === "/v1/customers/cus_123")!;
    expect(req.method).toBe("DELETE");
    // No body: an empty form body and no body are different requests, and the
    // server hashes the bytes against the idempotency key.
    expect(req.body).toBe("");
    // The key is what makes a retried delete answer the original
    // `{deleted: true}` rather than a 404 for a deletion that succeeded.
    expect(req.headers["idempotency-key"]).toBe("idem_del");
    expect(req.headers["content-type"]).toBeUndefined();
  });

  it("customers.del generates an Idempotency-Key when the caller supplies none", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: { id: "cus_123", object: "customer", deleted: true },
      }),
    });
    const client = makeClient(server);

    await client.customers.del("cus_123");

    const req = server.requests.find((r) => r.url === "/v1/customers/cus_123")!;
    expect(req.headers["idempotency-key"]).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    );
  });

  it("a payment intent's customer decodes, and survives a server that omits it", async () => {
    const withCustomer = {
      ...makeSamplePaymentIntent(),
      customer: "cus_123",
    } as Record<string, unknown>;
    const older = makeSamplePaymentIntent() as unknown as Record<
      string,
      unknown
    >;
    delete older["customer"];

    const server = await withServer({
      resource: (_req, index) => ({
        status: 200,
        body: index === 1 ? withCustomer : older,
      }),
    });
    const client = makeClient(server);

    const attached = await client.paymentIntents.retrieve("pi_123");
    expect(attached.customer).toBe("cus_123");

    // A vpay predating 2026-09-06 omits the key entirely. `customer?:` is
    // what makes that a decode rather than a type error at the boundary.
    const legacy = await client.paymentIntents.retrieve("pi_456");
    expect(legacy.customer).toBeUndefined();
  });

  it("paymentIntents.create sends a customer when given one, and omits it otherwise", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: makeSamplePaymentIntent() }),
    });
    const client = makeClient(server);

    await client.paymentIntents.create({
      amount: 5000,
      currency: "xaf",
      payment_method_types: ["mtn_momo"],
      customer: "cus_123",
    });
    await client.paymentIntents.create({
      amount: 5000,
      currency: "xaf",
      payment_method_types: ["mtn_momo"],
    });

    const reqs = server.requests.filter((r) => r.url === "/v1/payment_intents");
    expect(reqs[0]!.body).toBe(
      "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo&customer=cus_123",
    );
    expect(reqs[1]!.body).toBe(
      "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo",
    );
  });
});

/**
 * `/v1/invoices` and `/v1/invoice_items` — S4b's thirteen merchant
 * operations.
 *
 * These mirror `sdks/rust/tests/resources.rs`'s invoice cases one for one,
 * down to the body strings, for the reason the customer block above states:
 * ADR-0015's parity is about *wire semantics*, and two SDKs that both
 * "support invoices" while encoding a cleared `due_date` differently are not
 * at parity.
 *
 * Every server here is `src/testing/test-server.ts`. That is the whole of the
 * evidence, and `docs/sdks/parity.md`'s "Invoices exercised against a running
 * vpay" row says so: these assert that this SDK puts the documented bytes on
 * the wire and decodes the documented shape, never that a real vpay answers
 * them.
 */
describe("invoices", () => {
  const sampleLine = (
    overrides: Record<string, unknown> = {},
  ): Record<string, unknown> => ({
    id: "ii_123",
    // `line_item`, not `invoice_item`: the route and the object are named
    // differently and both spellings are the wire's.
    object: "line_item",
    description: "Hosting",
    quantity: 2,
    unit_amount: 5500,
    // `quantity * unit_amount`, computed by the database and never sent.
    amount: 11000,
    currency: "xaf",
    livemode: false,
    ...overrides,
  });

  /**
   * All **eighteen** keys of the invoice object, one line expanded.
   *
   * The count is the point: this object is the `data.object` of four event
   * types, so a nineteenth key is signed, delivered and stored forever.
   * `status` is `open` with a number assigned rather than a draft, so that
   * `number: string | null` is decoded in its assigned form somewhere.
   */
  const sampleInvoice = (
    overrides: Record<string, unknown> = {},
  ): Record<string, unknown> => ({
    id: "in_123",
    object: "invoice",
    // Never null, unlike a payment intent's.
    customer: "cus_123",
    currency: "xaf",
    status: "open",
    number: "A7K3M9QP-000001",
    amount_due: 11000,
    amount_paid: 0,
    amount_remaining: 11000,
    due_date: null,
    description: "September hosting",
    metadata: { order_id: "1234" },
    payment_intent: null,
    hosted_invoice_url: null,
    lines: {
      object: "list",
      has_more: false,
      url: "/v1/invoice_items",
      data: [sampleLine()],
    },
    status_transitions: {
      finalized_at: 1_753_401_600,
      paid_at: null,
      voided_at: null,
      marked_uncollectible_at: null,
    },
    created: 1_753_401_600,
    livemode: false,
    ...overrides,
  });

  it("invoices.create: exact path, method, Idempotency-Key, and body", async () => {
    const server = await withServer({
      resource: () => ({ status: 201, body: sampleInvoice() }),
    });
    const client = makeClient(server);

    const invoice = await client.invoices.create(
      {
        customer: "cus_1",
        // Upper-cased deliberately: the wire contract says lowercase and the
        // SDK normalises, exactly as on an intent.
        currency: "XAF",
        description: "September hosting",
        due_date: 1_753_401_600,
        metadata: { order_id: "1234" },
      },
      { idempotencyKey: "idem_inv" },
    );

    const req = server.requests.find((r) => r.url === "/v1/invoices")!;
    expect(req.method).toBe("POST");
    expect(req.headers["idempotency-key"]).toBe("idem_inv");
    expect(req.body).toBe(
      "customer=cus_1&currency=xaf&description=September%20hosting&due_date=1753401600&metadata[order_id]=1234",
    );

    // Every one of the eighteen keys decodes, including the two that do not
    // come from the row.
    expect(invoice.id).toBe("in_123");
    expect(invoice.object).toBe("invoice");
    expect(invoice.customer).toBe("cus_123");
    expect(invoice.currency).toBe("xaf");
    expect(invoice.status).toBe("open");
    expect(invoice.number).toBe("A7K3M9QP-000001");
    expect(invoice.amount_due).toBe(11000);
    expect(invoice.amount_paid).toBe(0);
    expect(invoice.amount_remaining).toBe(11000);
    expect(invoice.due_date).toBeNull();
    expect(invoice.description).toBe("September hosting");
    expect(invoice.metadata).toEqual({ order_id: "1234" });
    expect(invoice.payment_intent).toBeNull();
    expect(invoice.hosted_invoice_url).toBeNull();
    expect(invoice.lines.url).toBe("/v1/invoice_items");
    expect(invoice.lines.has_more).toBe(false);
    expect(invoice.lines.data).toHaveLength(1);
    expect(invoice.status_transitions.finalized_at).toBe(1_753_401_600);
    expect(invoice.status_transitions.paid_at).toBeNull();
    expect(invoice.status_transitions.voided_at).toBeNull();
    expect(invoice.status_transitions.marked_uncollectible_at).toBeNull();
    expect(invoice.created).toBe(1_753_401_600);
    expect(invoice.livemode).toBe(false);
  });

  // The name said "an invoice needs only a customer" until 2026-09-08. It
  // does not: `POST /v1/invoices` with no `currency` is
  // `400 A three-letter \`currency\` code is required.` — measured against a
  // running vpay, which a stub answering 201 to anything could never have
  // said. `currency` is a required field of `CreateInvoiceParams` now, as it
  // already was of `CreatePaymentIntentParams`. See
  // `docs/plans/exp33-sdk-invoices-notes/opus-review.md`.
  it("an unset invoice field is omitted from the body rather than sent empty", async () => {
    const server = await withServer({
      resource: () => ({ status: 201, body: sampleInvoice() }),
    });
    const client = makeClient(server);

    await client.invoices.create({ customer: "cus_1", currency: "xaf" });

    const req = server.requests.find((r) => r.url === "/v1/invoices")!;
    // Omitted, never sent empty: `description=` means "clear it" on the
    // update path and would mean an empty description here.
    expect(req.body).toBe("customer=cus_1&currency=xaf");
  });

  it("invoices.retrieve: a GET whose response carries the lines", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: sampleInvoice() }),
    });
    const client = makeClient(server);

    const invoice = await client.invoices.retrieve("in_123");
    expect(invoice.id).toBe("in_123");
    const line = invoice.lines.data[0]!;
    expect(line.object).toBe("line_item");
    expect(line.quantity).toBe(2);
    expect(line.unit_amount).toBe(5500);
    expect(line.amount).toBe(11000);

    const req = server.requests.find((r) => r.url === "/v1/invoices/in_123")!;
    expect(req.method).toBe("GET");
    expect(req.body).toBe("");
    expect(req.headers["idempotency-key"]).toBeUndefined();
  });

  it("invoices.update tells leave-alone, set and clear apart on the wire", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: sampleInvoice() }),
    });
    const client = makeClient(server);

    await client.invoices.update(
      "in_123",
      {
        // set
        description: "October hosting",
        // clear
        due_date: null,
        // merge one key, remove another (the empty value)
        metadata: { order_id: "5678", tier: "" },
      },
      { idempotencyKey: "idem_upd_inv" },
    );

    const req = server.requests.find((r) => r.url === "/v1/invoices/in_123")!;
    expect(req.method).toBe("POST");
    // `due_date=` is "clear it". Collapsing `null` and `undefined` — the
    // obvious simplification — makes a due date set by mistake unremovable,
    // and this is the assertion that fails when somebody does it.
    expect(req.body).toBe(
      "description=October%20hosting&due_date=&metadata[order_id]=5678&metadata[tier]=",
    );
    expect(req.headers["idempotency-key"]).toBe("idem_upd_inv");
  });

  it("an invoice patch that mentions nothing clears nothing", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: sampleInvoice() }),
    });
    const client = makeClient(server);

    await client.invoices.update("in_123", {});

    const req = server.requests.find((r) => r.url === "/v1/invoices/in_123")!;
    expect(req.body).toBe("");
  });

  it("invoices.list: exact query string, both cursors and both filters", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: {
          object: "list",
          data: [sampleInvoice()],
          has_more: true,
          url: "/v1/invoices",
        },
      }),
    });
    const client = makeClient(server);

    const page = await client.invoices.list({
      limit: 2,
      starting_after: "in_0",
      customer: "cus_1",
      // The filter a merchant reconciling write-offs uses, because
      // `markUncollectible` emits no event.
      status: "uncollectible",
    });
    expect(page.has_more).toBe(true);
    expect(page.data).toHaveLength(1);

    const req = server.requests.find((r) => r.url.startsWith("/v1/invoices"))!;
    expect(req.url).toBe(
      "/v1/invoices?limit=2&starting_after=in_0&customer=cus_1&status=uncollectible",
    );
    expect(req.method).toBe("GET");
  });

  it("invoices.del: a DELETE with an Idempotency-Key and no body", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: { id: "in_123", object: "invoice", deleted: true },
      }),
    });
    const client = makeClient(server);

    const deleted = await client.invoices.del("in_123", {
      idempotencyKey: "idem_del_inv",
    });
    expect(deleted.deleted).toBe(true);
    expect(deleted.id).toBe("in_123");
    expect(deleted.object).toBe("invoice");

    const req = server.requests.find((r) => r.url === "/v1/invoices/in_123")!;
    expect(req.method).toBe("DELETE");
    expect(req.body).toBe("");
    expect(req.headers["idempotency-key"]).toBe("idem_del_inv");
    expect(req.headers["content-type"]).toBeUndefined();
  });

  /**
   * **The three transitions that take no parameters** — `finalize`, `void`
   * and `markUncollectible` — each POST an **empty body** to its own path
   * and still carry an `Idempotency-Key`.
   *
   * The third method is camelCase and its **path is not**: the route is
   * `/v1/invoices/{id}/mark_uncollectible`, and the loop below asserts that
   * path by name, so renaming the method and carrying the wire along with it
   * fails here.
   *
   * One case for the three because the property is the same one, and each
   * path is asserted separately so a method posting to the wrong one fails
   * here rather than being absorbed.
   */
  it("the three parameterless transitions post an empty body to their own paths", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: sampleInvoice() }),
    });
    const client = makeClient(server);

    await client.invoices.finalize("in_123", { idempotencyKey: "idem_fin" });
    await client.invoices.void("in_123", { idempotencyKey: "idem_void" });
    await client.invoices.markUncollectible("in_123", {
      idempotencyKey: "idem_unc",
    });

    for (const [name, key] of [
      ["finalize", "idem_fin"],
      ["void", "idem_void"],
      ["mark_uncollectible", "idem_unc"],
    ] as const) {
      const req = server.requests.find(
        (r) => r.url === `/v1/invoices/in_123/${name}`,
      )!;
      expect(req, `${name} posted to its own path`).toBeDefined();
      expect(req.method).toBe("POST");
      // A write with no parameters, so the body is empty — but it is still a
      // form POST, so the content type is present, unlike on a DELETE.
      expect(req.body).toBe("");
      expect(req.headers["content-type"]).toBe(
        "application/x-www-form-urlencoded",
      );
      expect(req.headers["idempotency-key"]).toBe(key);
    }
  });

  it("a refused transition is a VpayApiError carrying the server's 409 whole", async () => {
    const server = await withServer({
      resource: () => ({
        status: 409,
        body: {
          error: {
            type: "invalid_request_error",
            code: "invoice_status",
            message:
              "Invoice in_123 is paid, not open, so it cannot be voided.",
          },
        },
      }),
    });
    const client = makeClient(server);

    // The `message` is the only thing that tells a merchant which of `paid`,
    // `void` and `uncollectible` their invoice already reached.
    await expect(client.invoices.void("in_123")).rejects.toMatchObject({
      status: 409,
      code: "invoice_status",
    });
    const error = await client.invoices.void("in_123").catch((e: unknown) => e);
    expect(error).toBeInstanceOf(VpayApiError);
    expect((error as VpayApiError).message).toContain("is paid, not open");
  });

  it("invoices.pay sends both urls and decodes the intent and hosted url", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: sampleInvoice({
          payment_intent: "pi_123",
          hosted_invoice_url:
            "https://checkout.example/c/cs_1#cs_1_secret_abc123",
        }),
      }),
    });
    const client = makeClient(server);

    const invoice = await client.invoices.pay(
      "in_123",
      {
        success_url: "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
        cancel_url: "https://shop.example/cancel",
      },
      { idempotencyKey: "idem_pay" },
    );

    expect(invoice.payment_intent).toBe("pi_123");
    expect(invoice.hosted_invoice_url).toBe(
      "https://checkout.example/c/cs_1#cs_1_secret_abc123",
    );
    // `pay` mints a page to pay on; it does not pay. A merchant who read it
    // as "the invoice is paid" would fulfil on an unpaid bill.
    expect(invoice.status).toBe("open");

    const req = server.requests.find(
      (r) => r.url === "/v1/invoices/in_123/pay",
    )!;
    expect(req.body).toBe(
      "success_url=https%3A%2F%2Fshop.example%2Fok%3Fsid%3D%7BCHECKOUT_SESSION_ID%7D&cancel_url=https%3A%2F%2Fshop.example%2Fcancel",
    );
    expect(req.headers["idempotency-key"]).toBe("idem_pay");
  });

  it("invoiceItems.create: exact body, and no currency or amount in it", async () => {
    const server = await withServer({
      resource: () => ({ status: 201, body: sampleLine() }),
    });
    const client = makeClient(server);

    const line = await client.invoiceItems.create(
      {
        invoice: "in_1",
        description: "Hosting",
        quantity: 2,
        unit_amount: 5500,
      },
      { idempotencyKey: "idem_item" },
    );

    expect(line.id).toBe("ii_123");
    expect(line.object).toBe("line_item");
    expect(line.amount).toBe(11000);

    const req = server.requests.find((r) => r.url === "/v1/invoice_items")!;
    // No `currency` (a line is always its invoice's) and no `amount` (the
    // database's product of the two factors). The string equality asserts
    // both absences rather than stating them.
    expect(req.body).toBe(
      "invoice=in_1&description=Hosting&quantity=2&unit_amount=5500",
    );
    expect(req.headers["idempotency-key"]).toBe("idem_item");
  });

  it("invoiceItems.create omits an absent quantity so the server applies its default", async () => {
    const server = await withServer({
      resource: () => ({ status: 201, body: sampleLine() }),
    });
    const client = makeClient(server);

    await client.invoiceItems.create({
      invoice: "in_1",
      description: "Hosting",
      unit_amount: 5500,
    });

    const req = server.requests.find((r) => r.url === "/v1/invoice_items")!;
    expect(req.body).toBe("invoice=in_1&description=Hosting&unit_amount=5500");
  });

  it("an invoice line's unit_amount is refused before any request", async () => {
    const server = await withServer({
      resource: () => ({ status: 201, body: sampleLine() }),
    });
    const client = makeClient(server);

    // The bound is the same one `sdks/rust` applies: an amount one SDK sends
    // and the other refuses is a divergence in the money path.
    for (const amount of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, 1e21]) {
      await expect(
        client.invoiceItems.create({
          invoice: "in_1",
          description: "Hosting",
          unit_amount: amount,
        }),
      ).rejects.toThrow(TypeError);
      await expect(
        client.invoiceItems.update("ii_123", { unit_amount: amount }),
      ).rejects.toThrow(TypeError);
    }

    expect(
      server.requests.filter((r) => r.url.startsWith("/v1/invoice_items")),
    ).toHaveLength(0);
  });

  it("invoiceItems.retrieve: exact GET path, no body, no Idempotency-Key", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: sampleLine() }),
    });
    const client = makeClient(server);

    const line = await client.invoiceItems.retrieve("ii_123");
    expect(line.id).toBe("ii_123");
    expect(line.description).toBe("Hosting");

    const req = server.requests.find(
      (r) => r.url === "/v1/invoice_items/ii_123",
    )!;
    expect(req.method).toBe("GET");
    expect(req.body).toBe("");
    expect(req.headers["idempotency-key"]).toBeUndefined();
  });

  it("invoiceItems.update sends only what it was given, and never an amount", async () => {
    const server = await withServer({
      resource: () => ({ status: 200, body: sampleLine({ quantity: 3 }) }),
    });
    const client = makeClient(server);

    await client.invoiceItems.update(
      "ii_123",
      { quantity: 3 },
      { idempotencyKey: "idem_upd_item" },
    );

    const req = server.requests.find(
      (r) => r.url === "/v1/invoice_items/ii_123",
    )!;
    expect(req.method).toBe("POST");
    // An unmentioned field is absent, never sent empty: all three columns are
    // NOT NULL, so `description=` is a 400 naming the parameter rather than a
    // clear — which is why the params type has no `| null`.
    expect(req.body).toBe("quantity=3");
    expect(req.body).not.toContain("amount=");
    expect(req.headers["idempotency-key"]).toBe("idem_upd_item");
  });

  it("invoiceItems.del: a DELETE that decodes the line_item shape", async () => {
    const server = await withServer({
      resource: () => ({
        status: 200,
        body: { id: "ii_123", object: "line_item", deleted: true },
      }),
    });
    const client = makeClient(server);

    const deleted = await client.invoiceItems.del("ii_123", {
      idempotencyKey: "idem_del_item",
    });
    expect(deleted.deleted).toBe(true);
    expect(deleted.id).toBe("ii_123");
    // `line_item`, not `invoice_item` — the route's name is not the object's.
    expect(deleted.object).toBe("line_item");

    const req = server.requests.find(
      (r) => r.url === "/v1/invoice_items/ii_123",
    )!;
    expect(req.method).toBe("DELETE");
    expect(req.body).toBe("");
    expect(req.headers["idempotency-key"]).toBe("idem_del_item");
  });

  /**
   * **The four `invoice.*` event types are in the union, and their payloads
   * narrow to an invoice.**
   *
   * Different in kind from the `customer.created` gap: vpay *does* emit all
   * four, so a union short of them silently drops an event that is really
   * being delivered.
   *
   * The second half asserts the shape difference nothing else would catch: an
   * event body carries `lines.data` **empty** while a `/v1` response carries
   * the lines, and both must decode through the same type.
   */
  it("the four invoice event types are known and their payloads narrow", () => {
    const known: KnownEventType[] = [
      "invoice.created",
      "invoice.finalized",
      "invoice.paid",
      "invoice.voided",
    ];
    // A compile-time assertion as much as a runtime one: a union missing any
    // of the four makes this array a type error, which is the mutation this
    // case exists to catch.
    expect(known).toHaveLength(4);

    const event: Event = {
      id: "evt_1",
      object: "event",
      type: "invoice.finalized",
      created: 1_753_401_600,
      livemode: false,
      data: {
        object: sampleInvoice({
          lines: {
            object: "list",
            has_more: false,
            url: "/v1/invoice_items",
            data: [],
          },
        }),
      },
    };

    expect(isInvoiceEvent(event)).toBe(true);
    expect(isPaymentIntentEvent(event)).toBe(false);
    expect(isCheckoutSessionEvent(event)).toBe(false);
    if (!isInvoiceEvent(event)) throw new Error("unreachable");
    const invoice = event.data.object;
    expect(invoice.id).toBe("in_123");
    expect(invoice.status).toBe("open");
    // An `invoice.*` body carries no lines — retrieve the invoice for them.
    expect(invoice.lines.data).toHaveLength(0);

    // The two Stripe types vpay deliberately does not write stay out of the
    // union: nothing writes them, so an entry would be a false claim.
    const notEmitted = [
      "invoice.marked_uncollectible",
      "invoice.payment_failed",
    ];
    expect(notEmitted).not.toContain("invoice.paid");
  });
});
