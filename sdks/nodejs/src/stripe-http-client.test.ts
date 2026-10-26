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
import {
  VpayConfigError,
  VpayUnsignedResponseError,
  VpayUnverifiedResponseError,
} from "./errors.js";
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
  it("rejects with a StripeConnectionError that carries VpayUnsignedResponseError when vpay refuses the key", async () => {
    // **Measured against `stripe@22.6.1`**, because the authenticator this
    // replaces failed the other way: its rejection was thrown inside a
    // detached promise chain that never called stripe-node's callback, so the
    // caller's `await` never settled. An `httpClient` failure is reported
    // through the normal path.
    const server = await startSealedServer(
      (_req, res) => jsonResponse(res, 200, SAMPLE_INTENT),
      { registerMerchant: false },
    );
    servers.push(server);

    const error = await stripeFor(server)
      .paymentIntents.retrieve("pi_1")
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(Stripe.errors.StripeConnectionError);
    const raw = error as { detail?: unknown; raw?: { exception?: unknown } };
    const cause = raw.detail ?? raw.raw?.exception;
    expect(cause).toBeInstanceOf(VpayUnsignedResponseError);
    expect(server.opened).toHaveLength(0);
  });

  it("rejects with VpayUnverifiedResponseError underneath when the answer is sealed under another key", async () => {
    const server = await withServer(() => ({
      status: 200,
      body: SAMPLE_INTENT,
    }));
    const other = newTestIdentity();

    const error = await stripeFor(server, {
      serverKeys: { current: other.serverPublic },
    })
      .paymentIntents.retrieve("pi_1")
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(Stripe.errors.StripeConnectionError);
    const raw = error as { detail?: unknown; raw?: { exception?: unknown } };
    expect(raw.detail ?? raw.raw?.exception).toBeInstanceOf(
      VpayUnverifiedResponseError,
    );
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
    await expect(
      client.makeRequest(
        url.hostname,
        url.port,
        "/oauth/token",
        "GET",
        {},
        "",
        "http",
        1000,
      ),
    ).rejects.toThrow(/\/v1/u);
    await expect(
      client.makeRequest(
        url.hostname,
        url.port,
        "/v1/payment_intents/pi_1",
        "PUT",
        {},
        "",
        "http",
        1000,
      ),
    ).rejects.toThrow(/no PUT operation/u);
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
  it("is VpayUnsignedResponseError for a proxy's HTML page", async () => {
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

    await expect(
      client.makeRequest(
        url.hostname,
        url.port,
        "/v1/payment_intents/pi_1",
        "GET",
        {},
        "",
        "http",
        2000,
      ),
    ).rejects.toThrow(VpayUnsignedResponseError);
  });
});
