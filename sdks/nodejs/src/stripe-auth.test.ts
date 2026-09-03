/**
 * Tests for `@vpay/sdk/stripe`.
 *
 * Everything that touches HTTP runs against a real `node:http` server started
 * by the test, and the last case drives the **real `stripe` package** end to
 * end so the assertions are about the bytes the official SDK actually puts on
 * the wire, not about a harness that agrees with this file by construction.
 *
 * There is exactly one injected `fetch` in this file, in
 * `treats an omitted port as the scheme's default on both sides`, and its own
 * comment says why: a loopback server is always on an ephemeral port, so
 * "the scheme's default port" is not observable against one.
 */
import { verify } from "node:crypto";
import type { ServerResponse } from "node:http";
import { inspect } from "node:util";
import Stripe from "stripe";
import { afterEach, describe, expect, it, vi } from "vitest";
import { VpayAuthError, VpayConfigError } from "./errors.js";
import {
  createStripeAuthenticator,
  type StripeAuthenticatorRequest,
} from "./stripe-auth.js";
import { generateTestRsaKeyPair } from "./testing/keys.js";
import { startTestServer, type TestServer } from "./testing/test-server.js";

const { privateKey, publicKey, privateKeyPem } = generateTestRsaKeyPair();
const TOKEN_PATH = "/v1/oauth/token";

/** stripe-node's `StripeRequest`, as it hands it to an authenticator. */
type StripeRequestShape = {
  host: string;
  port: string;
  path: string;
  method: string;
  headers: Record<string, string | number | string[]>;
  body: string;
  protocol: string;
};

/**
 * A request addressed at the server the authenticator under test is
 * configured for.
 *
 * The address is taken from `server.url` rather than written as a literal
 * because the authenticator is **bound to its `baseUrl`'s origin** and
 * refuses anything else (`assertRequestIsBound`). A hard-coded
 * `api.vpay.example` here would make every test in this file a host-mismatch
 * test, which is the opposite of what most of them are about — and the tests
 * that *are* about the binding override these fields explicitly, so the
 * refusal is still exercised on purpose rather than by accident.
 */
function makeStripeRequest(
  server: TestServer,
  overrides: Partial<StripeRequestShape> = {},
): StripeRequestShape {
  const url = new URL(server.url);
  return {
    host: url.hostname,
    port: url.port,
    path: "/v1/payment_intents",
    method: "POST",
    headers: {
      "Content-Type": "application/x-www-form-urlencoded",
      "Content-Length": 42,
      "Idempotency-Key": "stripe-node-retry-abc",
    },
    body: "amount=5000&currency=xaf",
    protocol: url.protocol.slice(0, -1),
    ...overrides,
  };
}

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

function decodeJwtPart<T>(part: string): T {
  return JSON.parse(Buffer.from(part, "base64url").toString("utf8")) as T;
}

function jsonResponse(
  res: ServerResponse,
  status: number,
  body: unknown,
): void {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

interface TokenServerConfig {
  tokenExpiresIn?: number;
  /** Overrides the default 200 for a given 1-indexed token call; `undefined` falls through. */
  onToken?: (
    callIndex: number,
  ) => { status: number; body: unknown } | undefined;
  /** Handles every non-token request. Absent means "nothing else should be called". */
  resource?: (
    req: {
      method: string;
      url: string;
      body: string;
      headers: Record<string, string | string[] | undefined>;
    },
    callIndex: number,
  ) => { status: number; body: unknown };
}

/** A server that serves the real `client_credentials` + `private_key_jwt` token endpoint. */
async function startTokenServer(
  config: TokenServerConfig,
): Promise<TestServer> {
  let tokenCount = 0;
  let resourceCount = 0;
  return startTestServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    if (req.method === "POST" && url.pathname === TOKEN_PATH) {
      tokenCount += 1;
      const override = config.onToken?.(tokenCount);
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
    if (!config.resource) {
      jsonResponse(res, 500, { unexpected: req.url });
      return;
    }
    const { status, body } = config.resource(req, resourceCount);
    jsonResponse(res, status, body);
  });
}

let servers: TestServer[] = [];
async function withServer(config: TokenServerConfig): Promise<TestServer> {
  const server = await startTokenServer(config);
  servers.push(server);
  return server;
}

function tokenRequests(server: TestServer) {
  return server.requests.filter((r) => r.url === TOKEN_PATH);
}

afterEach(async () => {
  await Promise.all(servers.map((s) => s.close()));
  servers = [];
  vi.useRealTimers();
});

describe("createStripeAuthenticator — the handshake", () => {
  it("sets Authorization from a bearer minted by a real token exchange", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
      kid: "acme-cameroon-2026-08",
    });

    const request = makeStripeRequest(server);
    await authenticator(request);

    expect(request.headers["Authorization"]).toBe("Bearer access-token-1");

    // The exchange that produced it was the real one, not a shortcut.
    const [tokenRequest] = tokenRequests(server);
    expect(tokenRequest).toBeDefined();
    expect(tokenRequest!.method).toBe("POST");
    expect(tokenRequest!.headers["content-type"]).toBe(
      "application/x-www-form-urlencoded",
    );
    const fields = parseFormBody(tokenRequest!.body);
    expect(fields["grant_type"]).toBe("client_credentials");
    expect(fields["client_id"]).toBe("acme-cameroon");
    expect(fields["client_assertion_type"]).toBe(
      "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
    );
    expect(fields["audience"]).toBe("vpay:v1");
    expect(fields).not.toHaveProperty("client_secret");

    const [headerPart, payloadPart, signaturePart] =
      fields["client_assertion"]!.split(".");
    expect(signaturePart).toBeTruthy();
    const header = decodeJwtPart<{ alg: string; typ: string; kid?: string }>(
      headerPart!,
    );
    expect(header).toEqual({
      alg: "RS256",
      typ: "JWT",
      kid: "acme-cameroon-2026-08",
    });
    const payload = decodeJwtPart<Record<string, unknown>>(payloadPart!);
    expect(payload["iss"]).toBe("acme-cameroon");
    expect(payload["sub"]).toBe("acme-cameroon");
    expect(payload["aud"]).toBe(`${server.url}${TOKEN_PATH}`);
  });

  it("accepts a PEM string as well as a KeyObject", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey: privateKeyPem,
    });
    const request = makeStripeRequest(server);
    await authenticator(request);
    expect(request.headers["Authorization"]).toBe("Bearer access-token-1");
  });

  it("rejects a misconfiguration at construction, before any request", () => {
    expect(() =>
      createStripeAuthenticator({
        baseUrl: "",
        clientId: "acme-cameroon",
        privateKey,
      }),
    ).toThrow(VpayConfigError);
    expect(() =>
      createStripeAuthenticator({
        baseUrl: "https://api.vpay.example",
        clientId: "acme-cameroon",
        privateKey,
        assertionLifetimeSeconds: 301,
      }),
    ).toThrow(VpayConfigError);
  });

  /**
   * A `baseUrl` that is a *string* but not a **URL** is refused at
   * construction, by name.
   *
   * The case above uses `""`, which never reaches the URL parse: it fails
   * one line earlier on `baseUrl is required`. So nothing pinned the parse
   * itself, and the two values below are the ones a merchant actually
   * writes — a bare host copied from a dashboard, and a protocol-relative
   * URL copied from a browser address bar. Both are accepted by every check
   * before `parseBoundOrigin` and would otherwise become a host binding
   * derived from a `baseUrl` nobody validated.
   *
   * The message is asserted, not just the class: `baseUrl is required` and
   * `baseUrl is not an absolute URL` are different mistakes with different
   * fixes, and a merchant reading the throw needs to be told which one they
   * made.
   */
  it("refuses a baseUrl that is not an absolute URL, naming it", () => {
    for (const baseUrl of ["api.vpay.example", "//api.vpay.example"]) {
      const construct = (): unknown =>
        createStripeAuthenticator({
          baseUrl,
          clientId: "acme-cameroon",
          privateKey,
        });
      expect(construct, `${baseUrl} must be refused`).toThrow(VpayConfigError);
      expect(construct, `${baseUrl} must say why`).toThrow(
        `baseUrl is not an absolute URL: ${baseUrl}`,
      );
    }
  });

  /**
   * A key `createPrivateKey` cannot read is a startup failure, not a
   * first-request one.
   *
   * The distinction is the whole point: through stripe-node a failure at
   * first request does not reach the caller at all (see the last test in this
   * file — it arrives as a process-level `unhandledRejection` and the awaited
   * promise never settles), so "the merchant's key file was empty" would be
   * an outage with no diagnosable error. Parsed at construction it is a
   * `VpayConfigError` thrown by the line the merchant wrote.
   *
   * The empty string is not a contrived case: it is what
   * `readFileSync(path, 'utf8')` returns for a key file a deployment
   * mounted but never populated.
   */
  it("refuses an unreadable private key at construction, not at first request", async () => {
    const server = await withServer({});
    for (const badKey of [
      "",
      "not a pem",
      "-----BEGIN PRIVATE KEY-----\nnope\n-----END PRIVATE KEY-----\n",
    ]) {
      expect(
        () =>
          createStripeAuthenticator({
            baseUrl: server.url,
            clientId: "acme-cameroon",
            privateKey: badKey,
          }),
        `${JSON.stringify(badKey)} must be refused at construction`,
      ).toThrow(VpayConfigError);
    }
    // …and refusing it cost no token exchange, because there was no request.
    expect(tokenRequests(server)).toHaveLength(0);
  });

  /**
   * The parse happens **once**, and every later assertion is signed with the
   * cached `KeyObject` rather than by re-reading the PEM.
   *
   * Asserted through behaviour rather than by counting `createPrivateKey`
   * calls: two mints from the same authenticator produce two assertions that
   * both verify against the keypair's public half. A cache that handed back
   * the wrong key, or a re-parse that produced a different one, fails here.
   */
  it("signs every assertion with the same key it parsed at construction", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey: privateKeyPem,
    });

    await authenticator(makeStripeRequest(server));
    authenticator.invalidate();
    await authenticator(makeStripeRequest(server));

    const assertions = tokenRequests(server).map(
      (r) => parseFormBody(r.body)["client_assertion"]!,
    );
    expect(assertions).toHaveLength(2);
    for (const assertion of assertions) {
      const [headerPart, payloadPart, signaturePart] = assertion.split(".");
      expect(
        verify(
          "sha256",
          Buffer.from(`${headerPart}.${payloadPart}`, "utf8"),
          publicKey,
          Buffer.from(signaturePart!, "base64url"),
        ),
      ).toBe(true);
    }
  });
});

/**
 * The token is bound to the host it was minted for.
 *
 * stripe-node hands the authenticator **every** outbound request, and
 * `host`/`port`/`protocol` are configured on the `Stripe` instance — not
 * here. Omit them and stripe-node addresses `api.stripe.com:443`, so an
 * authenticator that wrote `Authorization` unconditionally would send a live
 * vpay bearer token to Stripe. These cases are what makes that unreachable.
 */
describe("createStripeAuthenticator — host binding", () => {
  it("refuses to sign a request addressed to Stripe, and mints nothing", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
    });

    const request = makeStripeRequest(server, {
      host: "api.stripe.com",
      port: "443",
      protocol: "https",
    });

    await expect(authenticator(request)).rejects.toThrow(VpayConfigError);
    await expect(authenticator(request)).rejects.toThrow(/api\.stripe\.com/);
    // Both origins are named, so the merchant can see which line of config is
    // wrong rather than only that something is.
    // `toThrow(string)` is a substring match, so no regex escaping is needed.
    await expect(authenticator(request)).rejects.toThrow(
      new URL(server.url).hostname,
    );

    // The decisive half: no `Authorization`, and **no token exchange**. A
    // check that ran after the mint would leave a freshly minted bearer in
    // the process for a request that was refused, and would spend a `jti`
    // per refused call.
    expect(request.headers).not.toHaveProperty("Authorization");
    expect(tokenRequests(server)).toHaveLength(0);
  });

  it("refuses the right host on the wrong port", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
    });

    const request = makeStripeRequest(server, { port: "9999" });
    await expect(authenticator(request)).rejects.toThrow(VpayConfigError);
    expect(request.headers).not.toHaveProperty("Authorization");
    expect(tokenRequests(server)).toHaveLength(0);
  });

  it("refuses the right host and port on the wrong protocol", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
    });

    const request = makeStripeRequest(server, { protocol: "https" });
    await expect(authenticator(request)).rejects.toThrow(VpayConfigError);
    expect(tokenRequests(server)).toHaveLength(0);
  });

  /**
   * A `baseUrl` on the scheme's default port and a request that omits the
   * port are the *same* origin, and must not be read as a mismatch —
   * `https://api.vpay.example` is `:443` whether or not anyone writes it.
   *
   * This is the one case in this file that cannot use a real server: a
   * loopback test server gets an ephemeral port, never 443, so there is no
   * way to observe "the default port" against one. The `fetch` here stands in
   * for the token endpoint only; what is being asserted is which requests the
   * authenticator agrees to sign, not how it talks to the OP (every other
   * test in this file covers that against real sockets).
   */
  it("treats an omitted port as the scheme's default on both sides", async () => {
    let tokenCalls = 0;
    const fetchImpl: typeof fetch = async () => {
      tokenCalls += 1;
      return new Response(
        JSON.stringify({
          access_token: "access-token-1",
          token_type: "Bearer",
          expires_in: 300,
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    };

    const authenticator = createStripeAuthenticator({
      baseUrl: "https://api.vpay.example",
      clientId: "acme-cameroon",
      privateKey,
      fetch: fetchImpl,
    });

    // stripe-node's own shape when only `host` is configured: no `port`.
    const request: StripeRequestShape = {
      host: "api.vpay.example",
      port: "",
      path: "/v1/payment_intents",
      method: "POST",
      headers: {},
      body: "",
      protocol: "https",
    };
    await authenticator(request);
    expect(request.headers["Authorization"]).toBe("Bearer access-token-1");
    expect(tokenCalls).toBe(1);
  });

  /**
   * The README's startup probe — `await authenticator({ headers: {} })` at
   * boot, so a wrong key fails deployment rather than the first payment.
   *
   * It has no host to check, and must not be turned into a refusal by the
   * check above. Pinned at runtime here and at the type level in the
   * compatibility block below (finding M4: the documented call has to both
   * compile and work).
   */
  it("allows the documented startup probe, which has no host at all", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
    });

    const probe: StripeAuthenticatorRequest = { headers: {} };
    await authenticator(probe);

    expect(probe.headers["Authorization"]).toBe("Bearer access-token-1");
    expect(tokenRequests(server)).toHaveLength(1);
  });
});

/**
 * The same four checks `client.test.ts` makes of `VpayClient`, made of the
 * authenticator.
 *
 * Different mechanism, same requirement. `VpayClient` keeps its secrets in
 * private class fields, which `util.inspect` and `JSON.stringify` skip by
 * construction; the authenticator is a *closure*, whose captured variables
 * are not reachable from the function object at all. Neither fact is
 * self-evidently true of a future edit — a `Object.assign(fn, { options })`
 * for debugging would break both — so both are pinned.
 */
describe("createStripeAuthenticator — privacy", () => {
  it("never reveals the PEM or the token through inspect, JSON, String or keys", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey: privateKeyPem,
    });

    // After a real exchange, so the token exists to be leaked.
    const request = makeStripeRequest(server);
    await authenticator(request);
    expect(request.headers["Authorization"]).toBe("Bearer access-token-1");

    const renderings = [
      inspect(authenticator, { depth: 10, showHidden: true }),
      JSON.stringify(authenticator) ?? "",
      String(authenticator),
      Object.keys(authenticator).join(","),
    ];
    for (const rendering of renderings) {
      expect(rendering).not.toContain("PRIVATE KEY");
      expect(rendering).not.toContain(privateKeyPem);
      expect(rendering).not.toContain("access-token-1");
    }
  });
});

describe("createStripeAuthenticator — token lifecycle", () => {
  it("performs exactly one token fetch for N concurrent calls", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
    });

    const requests = Array.from({ length: 8 }, () => makeStripeRequest(server));
    await Promise.all(requests.map((r) => authenticator(r)));

    expect(tokenRequests(server)).toHaveLength(1);
    for (const request of requests) {
      expect(request.headers["Authorization"]).toBe("Bearer access-token-1");
    }
  });

  it("reuses the cached token across sequential calls", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
    });

    const first = makeStripeRequest(server);
    const second = makeStripeRequest(server);
    await authenticator(first);
    await authenticator(second);

    expect(tokenRequests(server)).toHaveLength(1);
    expect(second.headers["Authorization"]).toBe("Bearer access-token-1");
  });

  it("re-mints once the token passes expires_in minus the safety margin", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    // expires_in 100 -> margin = min(30, 50) = 30 -> cached for 70s.
    const server = await withServer({ tokenExpiresIn: 100 });
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
    });

    const first = makeStripeRequest(server);
    await authenticator(first);
    expect(first.headers["Authorization"]).toBe("Bearer access-token-1");

    // Still inside the cache window: no second exchange.
    vi.advanceTimersByTime(69_000);
    const stillCached = makeStripeRequest(server);
    await authenticator(stillCached);
    expect(tokenRequests(server)).toHaveLength(1);
    expect(stillCached.headers["Authorization"]).toBe("Bearer access-token-1");

    // Past it: a fresh assertion, a fresh token, on the same authenticator.
    vi.advanceTimersByTime(2_000);
    const refreshed = makeStripeRequest(server);
    await authenticator(refreshed);
    expect(tokenRequests(server)).toHaveLength(2);
    expect(refreshed.headers["Authorization"]).toBe("Bearer access-token-2");
  });

  it("invalidate() forces the next call to re-mint", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
    });

    const first = makeStripeRequest(server);
    await authenticator(first);
    expect(tokenRequests(server)).toHaveLength(1);

    authenticator.invalidate();

    const second = makeStripeRequest(server);
    await authenticator(second);
    expect(tokenRequests(server)).toHaveLength(2);
    expect(second.headers["Authorization"]).toBe("Bearer access-token-2");

    // Two distinct assertions, each with its own jti — a replayed one is
    // refused by the OP, so a re-mint that reused the assertion would be
    // worse than no re-mint at all.
    const [a, b] = tokenRequests(server);
    const jti = (body: string) =>
      decodeJwtPart<{ jti: string }>(
        parseFormBody(body)["client_assertion"]!.split(".")[1]!,
      ).jti;
    expect(jti(a!.body)).not.toBe(jti(b!.body));
  });

  it("propagates a rejected token exchange with the OP's own reason", async () => {
    const server = await withServer({
      onToken: () => ({
        status: 401,
        body: {
          error: "invalid_client",
          error_description: "assertion signature did not verify",
        },
      }),
    });
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
    });

    const request = makeStripeRequest(server);
    await expect(authenticator(request)).rejects.toThrow(VpayAuthError);
    await expect(authenticator(request)).rejects.toThrow(
      /invalid_client: assertion signature did not verify/,
    );
    expect(request.headers).not.toHaveProperty("Authorization");
  });
});

describe("createStripeAuthenticator — blast radius", () => {
  it("writes headers.Authorization and touches nothing else on the request", async () => {
    const server = await withServer({});
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
    });

    const request = makeStripeRequest(server);
    const before = structuredClone(request);

    await authenticator(request);

    // Every field but `headers` is byte-for-byte what stripe-node handed us.
    // This is not cosmetic: `Content-Length` was computed from `body` before
    // the authenticator ran, so a body rewrite here truncates the request.
    const { headers: _newHeaders, ...restAfter } = request;
    const { headers: beforeHeaders, ...restBefore } = before;
    expect(restAfter).toEqual(restBefore);

    // …and within `headers`, exactly one key was added and none changed.
    expect(Object.keys(request.headers).sort()).toEqual(
      [...Object.keys(beforeHeaders), "Authorization"].sort(),
    );
    const { Authorization: _auth, ...otherHeaders } = request.headers;
    expect(otherHeaders).toEqual(beforeHeaders);
    expect(request.headers["Authorization"]).toBe("Bearer access-token-1");
  });
});

describe("createStripeAuthenticator — stripe-node compatibility", () => {
  /**
   * The type-level half of the contract. `stripe-auth.ts` writes
   * `StripeRequest` out structurally rather than importing it (so the module
   * builds with `stripe` absent); these assignments are what stops that
   * hand-written shape from silently drifting from the real one. They are
   * checked by `pnpm --filter @vpay/sdk typecheck`, not by vitest — which
   * strips types without checking them — so the `it` block below exists only
   * so the values are constructed at runtime too.
   */
  const typedAuthenticator = createStripeAuthenticator({
    baseUrl: "https://api.vpay.example",
    clientId: "acme-cameroon",
    privateKey,
  });

  const asAuthenticator: Stripe.StripeConfig["authenticator"] =
    typedAuthenticator;

  const asConfig: Stripe.StripeConfig = {
    authenticator: typedAuthenticator,
    host: "api.vpay.example",
    port: "443",
    protocol: "https",
    maxNetworkRetries: 2,
  };

  /**
   * The other half of the parameter type, and the one a narrower signature
   * would break: the README documents `await authenticator({ headers: {} })`
   * as a startup probe, so that call has to *compile*. Written as a value
   * rather than a call so the assignability is checked by `typecheck` without
   * a token exchange running at module load.
   *
   * Both directions matter and are checked together: this file would still
   * compile if the parameter were widened to `unknown`, but `asAuthenticator`
   * above would not — `Stripe.StripeConfig["authenticator"]` pins the floor
   * while this pins the ceiling.
   */
  const startupProbe: Parameters<typeof typedAuthenticator>[0] = {
    headers: {},
  };
  const probeReturnsAPromise: (request: {
    headers: Record<string, string | number | string[]>;
  }) => Promise<void> = typedAuthenticator;

  it("is assignable to Stripe.StripeConfig['authenticator']", () => {
    expect(typeof asAuthenticator).toBe("function");
    expect(asConfig.authenticator).toBe(typedAuthenticator);
    expect(typeof typedAuthenticator.invalidate).toBe("function");
    // The probe shape the README documents is constructible, and the
    // authenticator is a function that accepts it.
    expect(startupProbe.headers).toEqual({});
    expect(probeReturnsAPromise).toBe(typedAuthenticator);
  });

  it("authenticates a real `stripe` client end to end", async () => {
    const server = await withServer({
      resource: (req) => {
        if (req.method !== "POST" || req.url !== "/v1/payment_intents") {
          return {
            status: 404,
            body: {
              error: {
                type: "invalid_request_error",
                code: "resource_missing",
                message: req.url,
              },
            },
          };
        }
        return {
          status: 200,
          body: {
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
          },
        };
      },
    });

    const port = new URL(server.url).port;
    const authenticator = createStripeAuthenticator({
      baseUrl: server.url,
      clientId: "acme-cameroon",
      privateKey,
    });

    const stripe = new Stripe("", {
      authenticator,
      host: "127.0.0.1",
      port,
      protocol: "http",
      maxNetworkRetries: 0,
      telemetry: false,
    });

    const intent = await stripe.paymentIntents.create({
      amount: 5000,
      currency: "xaf",
      // `mtn_momo` is a vpay rail code, not one of Stripe's — the cast is the
      // documented divergence, not a shortcut around a type error.
      payment_method_types: ["mtn_momo"],
    } as unknown as Stripe.PaymentIntentCreateParams);

    expect(intent.id).toBe("pi_e2e_1");

    const call = server.requests.find((r) => r.url === "/v1/payment_intents");
    expect(call).toBeDefined();

    // The whole point: the bearer the authenticator minted reached the wire
    // through stripe-node's own request path.
    expect(call!.headers["authorization"]).toBe("Bearer access-token-1");
    expect(tokenRequests(server)).toHaveLength(1);

    // Form-encoded body, indexed array encoding, and the auto-generated
    // idempotency key stripe-node puts on every v1 POST — which is what makes
    // vpay's mandatory `Idempotency-Key` free for Stripe SDK users.
    expect(call!.headers["content-type"]).toBe(
      "application/x-www-form-urlencoded",
    );
    expect(parseFormBody(call!.body)).toEqual({
      amount: "5000",
      currency: "xaf",
      "payment_method_types[0]": "mtn_momo",
    });
    expect(call!.headers["idempotency-key"]).toMatch(
      /^stripe-node-retry-[0-9a-f-]{36}$/,
    );
  });

  /**
   * **Measured, not inferred.** The design for this step read
   * `RequestSender.ts` and concluded a rejecting authenticator surfaces as
   * `StripeError: Unable to authenticate the request`. It is thrown — but
   * inside a detached `.catch` that never calls stripe-node's `callback`
   * (`esm/RequestSender.js`, the `authenticator(request).then(...).catch(...)`
   * chain). So in `stripe@22.6.1` the error reaches the process as an
   * **unhandled rejection** and the promise the merchant awaited never
   * settles at all — not even after `timeout`, because no HTTP request was
   * ever started for a timeout to fire against.
   *
   * This test pins that behaviour so the README's warning about it cannot
   * quietly become false: if a future `stripe` release routes the failure to
   * the caller, this test fails and the docs get corrected.
   */
  it("leaves the caller hanging on a failed handshake — stripe-node 22.6.1 never rejects it", async () => {
    const server = await withServer({
      onToken: () => ({
        status: 401,
        body: { error: "invalid_client", error_description: "unknown client" },
      }),
    });

    const stripe = new Stripe("", {
      authenticator: createStripeAuthenticator({
        baseUrl: server.url,
        clientId: "acme-cameroon",
        privateKey,
      }),
      host: "127.0.0.1",
      port: new URL(server.url).port,
      protocol: "http",
      maxNetworkRetries: 0,
      telemetry: false,
    });

    // Take over `unhandledRejection` for the duration: the whole point of the
    // test is that stripe-node emits one, and vitest's own listener would
    // otherwise fail the run over an error this test is deliberately causing.
    const vitestListeners = process.listeners("unhandledRejection");
    for (const listener of vitestListeners) {
      process.off("unhandledRejection", listener);
    }
    //
    // Only *this* test's rejection is counted. A bare `unhandled.push(reason)`
    // counts whatever else the process happens to reject while the listener
    // is installed — a late timer from an earlier test, a socket teardown —
    // and `toHaveLength(1)` then fails, or passes for the wrong reason. The
    // filter is the failure this test causes and nothing else: stripe-node's
    // own wrapper message, carrying at `raw.exception` the `VpayAuthError`
    // this test's 401 produced.
    const isThisTestsRejection = (reason: unknown): boolean => {
      if (typeof reason !== "object" || reason === null) return false;
      const error = reason as {
        message?: unknown;
        raw?: { exception?: unknown };
      };
      return (
        error.message === "Unable to authenticate the request" &&
        error.raw?.exception instanceof VpayAuthError
      );
    };
    const unhandled: unknown[] = [];
    const strangers: unknown[] = [];
    const capture = (reason: unknown): void => {
      (isThisTestsRejection(reason) ? unhandled : strangers).push(reason);
    };
    process.on("unhandledRejection", capture);

    let settled: "resolved" | "rejected" | "pending" = "pending";
    try {
      const call = stripe.paymentIntents.retrieve("pi_1");
      call.then(
        () => {
          settled = "resolved";
        },
        () => {
          settled = "rejected";
        },
      );

      // Give the handshake, the rejection, and Node's unhandled-rejection
      // pass every chance to complete.
      for (let i = 0; i < 50 && unhandled.length === 0; i += 1) {
        await new Promise((resolve) => setTimeout(resolve, 20));
      }
      await new Promise((resolve) => setTimeout(resolve, 50));
    } finally {
      process.off("unhandledRejection", capture);
      for (const listener of vitestListeners) {
        process.on("unhandledRejection", listener);
      }
    }

    // The failure exists, and it carries the vpay reason a merchant needs to
    // tell "my key is wrong" from "vpay is down"…
    //
    // `strangers` is carried into the message rather than asserted on: an
    // unrelated late rejection is not this test's business, but it is exactly
    // the thing someone debugging a failure here would want to see.
    expect(
      unhandled,
      `unrelated rejections seen: ${inspect(strangers)}`,
    ).toHaveLength(1);

    const emitted = unhandled[0] as {
      message: string;
      raw?: { exception?: unknown };
    };
    expect(emitted.message).toBe("Unable to authenticate the request");
    expect(emitted.raw?.exception).toBeInstanceOf(VpayAuthError);
    expect((emitted.raw?.exception as VpayAuthError).error).toBe(
      "invalid_client",
    );

    // …but it never reaches the caller's `await`.
    expect(settled).toBe("pending");

    // And no resource request was attempted, so nothing was half-sent.
    expect(server.requests.filter((r) => r.url !== TOKEN_PATH)).toHaveLength(0);
  });
});
