/**
 * Tests for `@vaam-apps/vpay-sdk/stripe`.
 *
 * Everything that touches HTTP runs against a real `node:http` server that
 * opens the sealed request under the merchant's registered key and seals its
 * answer (`src/testing/test-server.ts`), and most cases drive the **real
 * `stripe` package** so the assertions are about the bytes the official SDK
 * actually hands the client, not about a harness that agrees with this file by
 * construction.
 */
import { generateKeyPairSync } from "node:crypto";
import { inspect } from "node:util";
import Stripe from "stripe";
import { afterEach, describe, expect, it } from "vitest";
import { VpayConfigError } from "./errors.js";
import { createStripeHttpClient } from "./stripe-http-client.js";
import { newTestIdentity } from "./testing/responder.js";
import {
  startSealedServer,
  startTestServer,
  type RecordedRequest,
  type SealedResponder,
  type SealedTestServer,
  type TestServer,
} from "./testing/test-server.js";

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

function jsonResponse(
  res: SealedResponder,
  status: number,
  body: unknown,
): void {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

const SAMPLE_INTENT = {
  id: "pi_e2e_1",
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
};

let servers: (SealedTestServer | TestServer)[] = [];
afterEach(async () => {
  await Promise.all(servers.map((s) => s.close()));
  servers = [];
});

async function withServer(
  resource: (req: RecordedRequest) => { status: number; body: unknown },
): Promise<SealedTestServer> {
  const server = await startSealedServer((req, res) => {
    const { status, body } = resource(req);
    jsonResponse(res, status, body);
  });
  servers.push(server);
  return server;
}

function stripeFor(
  server: { url: string; identity: SealedTestServer["identity"] },
  overrides: Partial<Parameters<typeof createStripeHttpClient>[0]> = {},
  stripeConfig: { maxNetworkRetries?: number } = {},
): Stripe {
  const url = new URL(server.url);
  return new Stripe("unused-placeholder", {
    httpClient: createStripeHttpClient({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey: server.identity.merchantSeed,
      serverKeys: { current: server.identity.serverPublic },
      ...overrides,
    }),
    host: url.hostname,
    port: url.port,
    protocol: "http",
    maxNetworkRetries: 0,
    telemetry: false,
    ...stripeConfig,
  });
}

describe("createStripeHttpClient — a real stripe-node client", () => {
  it("is assignable to Stripe.StripeConfig['httpClient']", () => {
    // The type-level half of the contract: `stripe-http-client.ts` writes the
    // client's shape out structurally rather than importing it (so the module
    // builds with `stripe` absent), and this assignment is what stops that
    // hand-written shape drifting from the real `HttpClientInterface`. It is
    // checked by `pnpm --filter @vaam-apps/vpay-sdk typecheck`; the runtime
    // assertion only makes the value exist.
    const identity = newTestIdentity();
    const client = createStripeHttpClient({
      baseUrl: "https://api.vpay.example",
      clientId: "acme-cameroon",
      privateKey: identity.merchantSeed,
      serverKeys: { current: identity.serverPublic },
    });
    const asConfig: Stripe.StripeConfig = {
      httpClient: client,
      host: "api.vpay.example",
      port: "443",
      protocol: "https",
      maxNetworkRetries: 2,
    };
    expect(asConfig.httpClient).toBe(client);
    expect(client.getClientName()).toBe("vpay-sealed");
  });

  it("creates a payment intent: sealed form body, bound Idempotency-Key, no Authorization", async () => {
    const server = await withServer((req) =>
      req.method === "POST" && req.url === "/v1/payment_intents"
        ? { status: 200, body: SAMPLE_INTENT }
        : {
            status: 404,
            body: {
              error: {
                type: "invalid_request_error",
                code: "resource_missing",
                message: req.url,
              },
            },
          },
    );

    const intent = await stripeFor(server).paymentIntents.create({
      amount: 5000,
      currency: "xaf",
      // `mtn_momo` is a vpay rail code, not one of Stripe's; stripe-node types
      // `payment_method_types` as `Array<string>`, so no cast is needed.
      payment_method_types: ["mtn_momo"],
    });

    expect(intent.id).toBe("pi_e2e_1");

    const call = server.opened[0]!;
    expect(call.headers["content-type"]).toBe(
      "application/x-www-form-urlencoded",
    );
    expect(parseFormBody(call.body)).toEqual({
      amount: "5000",
      currency: "xaf",
      "payment_method_types[0]": "mtn_momo",
    });
    // The idempotency key stripe-node puts on every v1 POST is what makes
    // vpay's mandatory `Idempotency-Key` free for Stripe SDK users — and it is
    // the one header the signature binds.
    expect(call.headers["idempotency-key"]).toMatch(
      /^stripe-node-retry-[0-9a-f-]{36}$/u,
    );

    // The placeholder API key, `Stripe-Version` and the rest never reach the
    // wire: the request is the sealed message and a handful of fixed headers.
    const wire = server.wire[0]!;
    expect(wire.headers["authorization"]).toBeUndefined();
    expect(wire.headers["stripe-version"]).toBeUndefined();
    expect(wire.headers["x-stripe-client-user-agent"]).toBeUndefined();
    expect(wire.headers["content-type"]).toMatch(/^application\/cose/u);
  });

  it("lists with a query string, which the signature binds in canonical form", async () => {
    const server = await withServer((req) => ({
      status: 200,
      body: { object: "list", data: [], has_more: false, url: req.url },
    }));

    await stripeFor(server).paymentIntents.list({
      limit: 3,
      starting_after: "pi_9",
    });

    const call = server.opened[0]!;
    expect(call.method).toBe("GET");
    expect(call.url).toBe("/v1/payment_intents?limit=3&starting_after=pi_9");
    expect(call.headers["idempotency-key"]).toBeUndefined();
  });

  it("retrieves by id", async () => {
    const server = await withServer(() => ({
      status: 200,
      body: SAMPLE_INTENT,
    }));

    const intent = await stripeFor(server).paymentIntents.retrieve("pi_e2e_1");

    expect(intent.id).toBe("pi_e2e_1");
    expect(server.opened[0]!.url).toBe("/v1/payment_intents/pi_e2e_1");
  });

  it("gives a DELETE an Idempotency-Key, which stripe-node does not set but vpay requires", async () => {
    const server = await withServer(() => ({
      status: 200,
      body: { id: "cus_1", object: "customer", deleted: true },
    }));

    await stripeFor(server).customers.del("cus_1");

    const call = server.opened[0]!;
    expect(call.method).toBe("DELETE");
    expect(call.url).toBe("/v1/customers/cus_1");
    expect(call.headers["idempotency-key"]).toMatch(/^[0-9a-f-]{36}$/u);
    expect(call.body).toBe("");
  });

  it("maps a Stripe-shaped error envelope onto stripe-node's own error classes", async () => {
    const server = await withServer(() => ({
      status: 404,
      body: {
        error: {
          type: "invalid_request_error",
          code: "resource_missing",
          message: "No such payment_intent: 'pi_nope'",
          param: "id",
        },
      },
    }));

    const error = await stripeFor(server)
      .paymentIntents.retrieve("pi_nope")
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(Stripe.errors.StripeInvalidRequestError);
    expect((error as Stripe.errors.StripeError).statusCode).toBe(404);
    expect((error as Stripe.errors.StripeError).code).toBe("resource_missing");
  });
});

describe("createStripeHttpClient — refusals reach the caller", () => {
  // **Measured against `stripe@22.6.1`, with its default of two network
  // retries (`maxNetworkRetries: 2`) switched ON**, because that is what a
  // merchant who did nothing gets, and because rejecting `makeRequest` would
  // have sent a refused call out three times (stripe-node retries every
  // rejection, and its public API cannot exempt one). A refusal is therefore a
  // resolved, non-retryable Stripe-shaped response, and the server is asked once.
  it("surfaces a refused key once, as StripeAuthenticationError, even with stripe-node's default retries on", async () => {
    const server = await startSealedServer(
      (_req, res) => jsonResponse(res, 200, SAMPLE_INTENT),
      { registerMerchant: false },
    );
    servers.push(server);

    const error = await stripeFor(server, {}, { maxNetworkRetries: 2 })
      .paymentIntents.retrieve("pi_1")
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(Stripe.errors.StripeAuthenticationError);
    expect((error as Stripe.errors.StripeError).code).toBe("unsigned_response");
    expect((error as Error).message).toContain("without a signature");
    expect(server.opened).toHaveLength(0);
    expect(server.wire).toHaveLength(1);
  });

  it("surfaces an answer sealed under another key once, as StripeAPIError, with retries on", async () => {
    const server = await withServer(() => ({
      status: 200,
      body: SAMPLE_INTENT,
    }));
    const other = newTestIdentity();

    const error = await stripeFor(
      server,
      { serverKeys: { current: other.serverPublic } },
      { maxNetworkRetries: 2 },
    )
      .paymentIntents.retrieve("pi_1")
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(Stripe.errors.StripeAPIError);
    expect((error as Stripe.errors.StripeError).code).toBe(
      "unverified_response",
    );
    expect(server.wire).toHaveLength(1);
  });

  // The client-side counterpart of the refusals above. A method the contract
  // has no route for (a GET on a POST-only path) cannot be sealed. Rejecting
  // would have stripe-node retry it as a connection error, three times over;
  // it is a resolved, non-retryable invalid-request error, and nothing, signed
  // or not, goes to the server.
  it("answers an operation the contract does not have once, as StripeInvalidRequestError, and sends nothing", async () => {
    const server = await withServer(() => ({
      status: 200,
      body: SAMPLE_INTENT,
    }));

    const error: unknown = await stripeFor(server, {}, { maxNetworkRetries: 2 })
      .rawRequest("GET", "/v1/payment_intents/pi_1/confirm")
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(Stripe.errors.StripeInvalidRequestError);
    const stripeError = error as Stripe.errors.StripeInvalidRequestError;
    expect(stripeError.statusCode).toBe(404);
    expect(stripeError.code).toBe("unknown_operation");
    expect(stripeError.message).toContain("GET");
    expect(server.wire).toHaveLength(0);
  });

  // A sealed refusal that did not come from vpay's own error renderer: the body
  // bound is answered by a layer in front of it, sealed in cratestack's
  // `{ code, message, details }`. stripe-node reads an error only from an
  // `error` key, so this used to RESOLVE, handing the merchant that object as
  // though it were their PaymentIntent.
  it("answers a sealed 413 that is not in the Stripe envelope as an error, once, and never as a result", async () => {
    const server = await withServer(() => ({
      status: 413,
      body: {
        code: "INTERNAL_ERROR",
        message: "internal error",
        details: null,
      },
    }));

    const error = await stripeFor(server, {}, { maxNetworkRetries: 2 })
      .paymentIntents.create({ amount: 5000, currency: "xaf" })
      .catch((e: unknown) => e);

    // stripe-node picks the class from the status alone, and has none for 413.
    expect(error).toBeInstanceOf(Stripe.errors.StripeAPIError);
    const stripeError = error as Stripe.errors.StripeAPIError;
    expect(stripeError.statusCode).toBe(413);
    expect(stripeError.code).toBe("request_too_large");
    expect(stripeError.message).toContain("over the limit");
    expect(stripeError.headers?.["stripe-should-retry"]).toBe("false");
    expect(server.wire).toHaveLength(1);
  });

  it("answers any other sealed error outside the Stripe envelope as an error naming its status", async () => {
    const server = await withServer(() => ({
      status: 500,
      body: {
        code: "INTERNAL_ERROR",
        message: "internal error",
        details: null,
      },
    }));

    const error = await stripeFor(server, {}, { maxNetworkRetries: 0 })
      .paymentIntents.retrieve("pi_1")
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(Stripe.errors.StripeAPIError);
    const stripeError = error as Stripe.errors.StripeAPIError;
    expect(stripeError.statusCode).toBe(500);
    expect(stripeError.code).toBe("unexpected_error_response");
    expect(stripeError.message).toContain("500 (INTERNAL_ERROR)");
  });

  it("still retries a genuine transport failure, which is what maxNetworkRetries is for", async () => {
    const server = await withServer(() => ({
      status: 200,
      body: SAMPLE_INTENT,
    }));
    await server.close();
    servers = servers.filter((candidate) => candidate !== server);

    const error = await stripeFor(
      { url: server.url, identity: server.identity },
      {},
      { maxNetworkRetries: 1 },
    )
      .paymentIntents.retrieve("pi_1")
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(Stripe.errors.StripeConnectionError);
  });

  it("re-seals a retried write with the same Idempotency-Key and a new message", async () => {
    let calls = 0;
    const server = await startSealedServer((_req, res) => {
      calls += 1;
      jsonResponse(res, 200, SAMPLE_INTENT);
    });
    servers.push(server);
    const client = createStripeHttpClient({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey: server.identity.merchantSeed,
      serverKeys: { current: server.identity.serverPublic },
    });
    const url = new URL(server.url);
    const attempt = () =>
      client.makeRequest(
        url.hostname,
        url.port,
        "/v1/payment_intents",
        "POST",
        { "Idempotency-Key": "stripe-node-retry-1" },
        "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo",
        "http",
        5000,
      );

    await attempt();
    await attempt();

    // Two distinct messages (a replayed one would have been refused), one key.
    expect(calls).toBe(2);
    expect(server.opened.map((r) => r.headers["idempotency-key"])).toEqual([
      "stripe-node-retry-1",
      "stripe-node-retry-1",
    ]);
  });
});

describe("createStripeHttpClient — host binding", () => {
  async function bound() {
    const server = await withServer(() => ({
      status: 200,
      body: SAMPLE_INTENT,
    }));
    const client = createStripeHttpClient({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey: server.identity.merchantSeed,
      serverKeys: { current: server.identity.serverPublic },
    });
    return { server, client, url: new URL(server.url) };
  }

  it("refuses to seal a request addressed to Stripe, and signs nothing", async () => {
    const { server, client } = await bound();

    const error = await client
      .makeRequest(
        "api.stripe.com",
        "443",
        "/v1/payment_intents",
        "GET",
        {},
        "",
        "https",
        1000,
      )
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(VpayConfigError);
    expect((error as Error).message).toContain("api.stripe.com");
    expect((error as Error).message).toContain("127.0.0.1");
    expect(server.wire).toHaveLength(0);
  });

  it("refuses the right host on the wrong port", async () => {
    const { client, url } = await bound();
    await expect(
      client.makeRequest(
        url.hostname,
        "1",
        "/v1/payment_intents/pi_1",
        "GET",
        {},
        "",
        "http",
        1000,
      ),
    ).rejects.toThrow(VpayConfigError);
  });

  it("refuses the right host and port on the wrong protocol", async () => {
    const { client, url } = await bound();
    await expect(
      client.makeRequest(
        url.hostname,
        url.port,
        "/v1/payment_intents/pi_1",
        "GET",
        {},
        "",
        "https",
        1000,
      ),
    ).rejects.toThrow(VpayConfigError);
  });

  it("compares hosts without regard to case, a trailing dot or IPv6 brackets", async () => {
    const { client, url } = await bound();
    await expect(
      client.makeRequest(
        url.hostname.toUpperCase(),
        url.port,
        "/v1/payment_intents/pi_1",
        "GET",
        {},
        "",
        "HTTP:",
        2000,
      ),
    ).resolves.toBeDefined();
  });

  it("refuses a path outside /v1, and a verb /v1 has no operation for", async () => {
    const { client, url } = await bound();
    const outside = await client.makeRequest(
      url.hostname,
      url.port,
      "/oauth/token",
      "GET",
      {},
      "",
      "http",
      1000,
    );
    expect(outside.getStatusCode()).toBe(404);
    expect(outside.getHeaders()["stripe-should-retry"]).toBe("false");
    expect(await outside.toJSON()).toMatchObject({
      error: {
        type: "invalid_request_error",
        code: "unknown_operation",
        message: expect.stringMatching(/\/v1/u) as unknown,
      },
    });

    const put = await client.makeRequest(
      url.hostname,
      url.port,
      "/v1/payment_intents/pi_1",
      "PUT",
      {},
      "",
      "http",
      1000,
    );
    expect(put.getStatusCode()).toBe(404);
    expect(await put.toJSON()).toMatchObject({
      error: {
        code: "unknown_operation",
        message: expect.stringMatching(/no PUT operation/u) as unknown,
      },
    });
  });
});

describe("createStripeHttpClient — construction", () => {
  const identity = newTestIdentity();
  const base = {
    baseUrl: "https://api.vpay.example",
    clientId: "acme-cameroon",
    privateKey: identity.merchantSeed,
    serverKeys: { current: identity.serverPublic },
  };

  it("rejects a misconfiguration before any request", () => {
    expect(() => createStripeHttpClient({ ...base, baseUrl: "" })).toThrow(
      VpayConfigError,
    );
    expect(() => createStripeHttpClient({ ...base, clientId: "" })).toThrow(
      VpayConfigError,
    );
    expect(() =>
      createStripeHttpClient({
        ...base,
        serverKeys: undefined as unknown as { current: string },
      }),
    ).toThrow(VpayConfigError);
  });

  it("refuses a baseUrl that is not an absolute URL, naming it", () => {
    expect(() =>
      createStripeHttpClient({ ...base, baseUrl: "api.vpay.example" }),
    ).toThrow(/baseUrl is not an absolute URL: api\.vpay\.example/u);
  });

  it("refuses an RSA key", () => {
    const rsa = generateKeyPairSync("rsa", { modulusLength: 2048 }).privateKey;
    expect(() => createStripeHttpClient({ ...base, privateKey: rsa })).toThrow(
      /Ed25519/u,
    );
  });

  it("never reveals the key through inspect, JSON, String or keys", () => {
    const client = createStripeHttpClient(base);
    for (const text of [
      inspect(client, { depth: 10, showHidden: true }),
      JSON.stringify(client),
      Object.prototype.toString.call(client),
      Object.keys(client).join(","),
    ]) {
      expect(text).not.toContain(identity.merchantSeed);
    }
  });
});

describe("createStripeHttpClient — plain-server cases", () => {
  it("presents a proxy's HTML 502 as a retryable unsigned_response, not a rejection", async () => {
    const server = await startTestServer((_req, res) => {
      res.writeHead(502, { "Content-Type": "text/html" });
      res.end("<html>Bad Gateway</html>");
    });
    servers.push(server);
    const identity = newTestIdentity();
    const url = new URL(server.url);
    const client = createStripeHttpClient({
      baseUrl: server.url,
      clientId: "m",
      privateKey: identity.merchantSeed,
      serverKeys: { current: identity.serverPublic },
    });

    const response = await client.makeRequest(
      url.hostname,
      url.port,
      "/v1/payment_intents/pi_1",
      "GET",
      {},
      "",
      "http",
      2000,
    );
    expect(response.getStatusCode()).toBe(502);
    // A gateway's 5xx is the one refusal left retryable: no `stripe-should-retry`.
    expect(response.getHeaders()["stripe-should-retry"]).toBeUndefined();
    const body = (await response.toJSON()) as {
      error: { code: string; type: string };
    };
    expect(body.error).toMatchObject({
      code: "unsigned_response",
      type: "api_error",
    });
  });
});
