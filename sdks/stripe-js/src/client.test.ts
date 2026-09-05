/**
 * `@vpay/stripe-js` against a real `node:http` stub of vpay's `/v1/browser`
 * surface.
 *
 * The stub is `src/testing/browser-stub.ts`. ADR-0006 and AGENTS.md's rule 1
 * forbid a test double reachable from `vpay-server` or `vpay-worker-bin`; an
 * SDK's own unit-test server is neither — it is excluded from `dist` by
 * `tsconfig.build.json` and imported only from `*.test.ts`.
 *
 * A real socket rather than a patched `fetch`, because the contract under
 * test is bytes: the query string that carries `key` and `client_secret`,
 * the bracket-nested `payment_method_data`, and the headers that are
 * *absent* (an `Idempotency-Key` here would turn every confirm into a CORS
 * preflight).
 */
import { inspect } from "node:util";
import { afterEach, describe, expect, it } from "vitest";
import { loadStripe } from "./index.js";
import {
  errorEnvelope,
  json,
  notFoundEnvelope,
  samplePaymentIntent,
  startBrowserStub,
  type BrowserStub,
  type StubHandler,
} from "./testing/browser-stub.js";
import type { PaymentIntentResult, Stripe } from "./types.js";

const PK = "pk_test_abcdefghijklmnop";
const SUFFIX = "a".repeat(32);
const SECRET = `pi_123_secret_${SUFFIX}`;
const INTENT_PATH = "/v1/browser/payment_intents/pi_123";

const stubs: BrowserStub[] = [];

afterEach(async () => {
  await Promise.all(stubs.splice(0).map((stub) => stub.close()));
});

/** Starts a stub and a `Stripe` bound to it. */
async function withStub(
  handler: StubHandler,
): Promise<{ stub: BrowserStub; stripe: Stripe }> {
  const stub = await startBrowserStub(handler);
  stubs.push(stub);
  const stripe = await loadStripe(PK, { baseUrl: stub.url });
  return { stub, stripe };
}

/** A stub that answers every request with one status and body. */
function answering(status: number, body: unknown): { handler: StubHandler } {
  return { handler: (_req, res) => json(res, status, body) };
}

describe("retrievePaymentIntent", () => {
  it("GETs the browser route with key and client_secret in the query string", async () => {
    const intent = samplePaymentIntent({ status: "processing" });
    const { stub, stripe } = await withStub(answering(200, intent).handler);

    const result = await stripe.retrievePaymentIntent(SECRET);

    expect(result.error).toBeUndefined();
    expect(result.paymentIntent).toEqual(intent);
    expect(stub.requests).toHaveLength(1);
    const request = stub.requests[0]!;
    expect(request.method).toBe("GET");
    expect(request.url).toBe(
      `${INTENT_PATH}?key=${PK}&client_secret=pi_123_secret_${SUFFIX}`,
    );
    expect(request.body).toBe("");
  });

  it("renders all thirteen keys of PaymentIntentWithSecret", async () => {
    const { stripe } = await withStub(
      answering(200, samplePaymentIntent()).handler,
    );
    const result = await stripe.retrievePaymentIntent(SECRET);
    expect(Object.keys(result.paymentIntent ?? {}).sort()).toEqual(
      [
        "amount",
        "client_secret",
        "created",
        "currency",
        "description",
        "id",
        "last_payment_error",
        "livemode",
        "metadata",
        "next_action",
        "object",
        "payment_method_types",
        "status",
      ].sort(),
    );
  });

  it("sends no Idempotency-Key and no Authorization — a preflight is the cost of either", async () => {
    const { stub, stripe } = await withStub(
      answering(200, samplePaymentIntent()).handler,
    );
    await stripe.retrievePaymentIntent(SECRET);
    const headers = stub.requests[0]!.headers;
    expect(headers["idempotency-key"]).toBeUndefined();
    expect(headers["authorization"]).toBeUndefined();
  });
});

describe("confirmPayment", () => {
  it("POSTs the form-encoded body the design specifies, byte for byte", async () => {
    const { stub, stripe } = await withStub(
      answering(200, samplePaymentIntent({ status: "processing" })).handler,
    );

    const result = await stripe.confirmPayment({
      clientSecret: SECRET,
      confirmParams: {
        payment_method_data: {
          type: "mtn_momo",
          mtn_momo: { msisdn: "237690000000" },
        },
        return_url: "https://shop.example/thanks",
      },
    });

    expect(result.error).toBeUndefined();
    const request = stub.requests[0]!;
    expect(request.method).toBe("POST");
    expect(request.url).toBe(`${INTENT_PATH}/confirm`);
    expect(request.headers["content-type"]).toBe(
      "application/x-www-form-urlencoded",
    );
    expect(request.headers["idempotency-key"]).toBeUndefined();
    expect(request.body).toBe(
      `key=${PK}&client_secret=pi_123_secret_${SUFFIX}&` +
        "payment_method_data[type]=mtn_momo&" +
        "payment_method_data[mtn_momo][msisdn]=237690000000&" +
        "return_url=https%3A%2F%2Fshop.example%2Fthanks",
    );
  });

  it("omits return_url and payment_method_data when the caller sent neither", async () => {
    const { stub, stripe } = await withStub(
      answering(200, samplePaymentIntent()).handler,
    );
    await stripe.confirmPayment({ clientSecret: SECRET });
    expect(stub.requests[0]!.body).toBe(
      `key=${PK}&client_secret=pi_123_secret_${SUFFIX}`,
    );
  });

  it("answers invalid_request_error rather than sending an unencodable payment_method_data", async () => {
    const { stub, stripe } = await withStub(
      answering(200, samplePaymentIntent()).handler,
    );

    const result = await stripe.confirmPayment({
      clientSecret: SECRET,
      confirmParams: {
        payment_method_data: { type: "mtn_momo", when: new Date(0) },
      },
    });

    expect(result.paymentIntent).toBeUndefined();
    expect(result.error?.type).toBe("invalid_request_error");
    expect(result.error?.param).toBe("payment_method_data");
    expect(stub.requests).toHaveLength(0);
  });
});

describe("confirmMobileMoneyPayment", () => {
  it("writes the rail code as both payment_method_data[type] and the nested key", async () => {
    const { stub, stripe } = await withStub(
      answering(200, samplePaymentIntent({ status: "processing" })).handler,
    );

    const result = await stripe.confirmMobileMoneyPayment(SECRET, {
      type: "mtn_momo",
      msisdn: "237690000000",
    });

    expect(result.paymentIntent?.status).toBe("processing");
    expect(stub.requests[0]!.body).toBe(
      `key=${PK}&client_secret=pi_123_secret_${SUFFIX}&` +
        "payment_method_data[type]=mtn_momo&" +
        "payment_method_data[mtn_momo][msisdn]=237690000000",
    );
  });
});

describe("handleNextAction", () => {
  it("retrieves, and resolves unchanged when there is nothing to act on", async () => {
    const intent = samplePaymentIntent({ status: "processing" });
    const { stub, stripe } = await withStub(answering(200, intent).handler);

    const result = await stripe.handleNextAction({ clientSecret: SECRET });

    expect(result.paymentIntent).toEqual(intent);
    expect(stub.requests[0]!.method).toBe("GET");
  });
});

describe("error mapping", () => {
  it("maps the uniform 404 every browser credential failure renders", async () => {
    // `ApiError::NotFound { resource: "payment intent", id }` →
    // `Category::NotFound` → 404 / invalid_request_error / resource_missing,
    // with no `param`. An unknown publishable key, a wrong `client_secret`
    // and another merchant's key are byte-identical to this, by design.
    const { stripe } = await withStub(
      answering(404, notFoundEnvelope("pi_123")).handler,
    );

    const result = await stripe.retrievePaymentIntent(SECRET);

    expect(result.paymentIntent).toBeUndefined();
    expect(result.error).toEqual({
      type: "invalid_request_error",
      code: "resource_missing",
      message: "No such payment intent: pi_123",
    });
    expect("param" in (result.error as object)).toBe(false);
  });

  it.each([
    [
      "a rejected parameter",
      400,
      errorEnvelope(
        "invalid_request_error",
        "invalid_request",
        "amount must be a positive integer",
        "amount",
      ),
    ],
    [
      "a second confirm of the same intent",
      409,
      errorEnvelope(
        "invalid_request_error",
        "invalid_state",
        "This payment intent has already been confirmed. Create a new payment intent to retry.",
      ),
    ],
    [
      "a rail refusal",
      402,
      errorEnvelope(
        "invalid_request_error",
        "charge_declined",
        "The payer's mobile money account has insufficient funds.",
      ),
    ],
    [
      "an unreachable rail",
      503,
      errorEnvelope(
        "api_error",
        "provider_unavailable",
        "The payment provider is unavailable. Retry after a short delay.",
      ),
    ],
    [
      "an unrecognised route on the browser nest",
      404,
      errorEnvelope(
        "invalid_request_error",
        "unknown_route",
        "Unrecognized request URL.",
      ),
    ],
  ])("passes %s through 1:1", async (_name, status, envelope) => {
    const { stripe } = await withStub(answering(status, envelope).handler);
    const result = await stripe.retrievePaymentIntent(SECRET);
    expect(result.paymentIntent).toBeUndefined();
    expect(result.error).toEqual(envelope.error);
  });

  it("reports a non-envelope failure body as unexpected_response, without quoting it", async () => {
    const { stripe } = await withStub((_req, res) => {
      res.writeHead(502, { "Content-Type": "text/html" });
      res.end("<html><body>Bad gateway</body></html>");
    });

    const result = await stripe.retrievePaymentIntent(SECRET);

    expect(result.error).toEqual({
      type: "api_error",
      code: "unexpected_response",
      message: "The vpay API returned an unexpected response (HTTP 502).",
    });
  });

  it("reports a 200 that is not a payment intent as unexpected_response", async () => {
    const { stripe } = await withStub(
      answering(200, { object: "list", data: [] }).handler,
    );
    const result = await stripe.retrievePaymentIntent(SECRET);
    expect(result.error?.code).toBe("unexpected_response");
  });

  it("reports a refused connection as api_connection_error and never rejects", async () => {
    const dead = await startBrowserStub((_req, res) => res.end());
    const baseUrl = dead.url;
    await dead.close();
    const stripe = await loadStripe(PK, { baseUrl });

    const result = await stripe.retrievePaymentIntent(SECRET);

    expect(result.paymentIntent).toBeUndefined();
    expect(result.error).toEqual({
      type: "api_connection_error",
      message: "Could not reach the vpay API.",
    });
    // No `code`, and nothing from the thrown `TypeError` — whose `cause` in
    // some engines carries the request URL, i.e. the client secret.
    expect(result.error?.code).toBeUndefined();
  });
});

describe("client secret parsing", () => {
  it.each([
    ["an empty string", ""],
    ["no separator", "pi_123"],
    ["the wrong object prefix", `re_123_secret_${SUFFIX}`],
    ["no id before the separator", `_secret_${SUFFIX}`],
    ["a bare prefix as the id", `pi__secret_${SUFFIX}`],
    ["no suffix after the separator", "pi_123_secret_"],
    ["a non-string", 7 as unknown as string],
  ])("refuses %s without sending a request", async (_name, secret) => {
    const { stub, stripe } = await withStub(
      answering(200, samplePaymentIntent()).handler,
    );

    const result = await stripe.retrievePaymentIntent(secret);

    expect(result.paymentIntent).toBeUndefined();
    expect(result.error?.type).toBe("invalid_request_error");
    expect(result.error?.param).toBe("clientSecret");
    expect(stub.requests).toHaveLength(0);
  });

  it("takes the id from the first separator, so a suffix containing one still resolves", async () => {
    const { stub, stripe } = await withStub(
      answering(200, samplePaymentIntent()).handler,
    );
    await stripe.retrievePaymentIntent(`pi_123_secret_x_secret_y`);
    expect(stub.requests[0]!.url.startsWith(`${INTENT_PATH}?`)).toBe(true);
  });

  it("refuses on confirm too, before any charge could be attempted", async () => {
    const { stub, stripe } = await withStub(
      answering(200, samplePaymentIntent()).handler,
    );
    const result = await stripe.confirmPayment({ clientSecret: "nope" });
    expect(result.error?.param).toBe("clientSecret");
    expect(stub.requests).toHaveLength(0);
  });
});

describe("loadStripe", () => {
  it("rejects a blank publishable key — an integration mistake, not a payer failure", async () => {
    await expect(
      loadStripe("  ", { baseUrl: "https://api.example" }),
    ).rejects.toThrow(TypeError);
  });

  it("rejects a blank base URL", async () => {
    await expect(loadStripe(PK, { baseUrl: "" })).rejects.toThrow(TypeError);
  });

  it("strips trailing slashes so the path cannot become //v1/browser", async () => {
    const stub = await startBrowserStub((_req, res) =>
      json(res, 200, samplePaymentIntent()),
    );
    stubs.push(stub);
    const stripe = await loadStripe(PK, { baseUrl: `${stub.url}//` });
    await stripe.retrievePaymentIntent(SECRET);
    expect(stub.requests[0]!.url.startsWith(INTENT_PATH)).toBe(true);
  });

  it("strips a long run of trailing slashes in linear time, not just one", async () => {
    // `stripTrailingSlashes` is a loop, not a regex, specifically so a caller
    // (or an attacker controlling `options.baseUrl`) cannot cost more than
    // O(n) with a pathological run of slashes. 50k is enough that a
    // polynomial-backtracking implementation would time the suite out; a
    // linear one finishes instantly.
    const manySlashes = "/".repeat(50_000);
    const stub = await startBrowserStub((_req, res) =>
      json(res, 200, samplePaymentIntent()),
    );
    stubs.push(stub);
    const stripe = await loadStripe(PK, {
      baseUrl: `${stub.url}${manySlashes}`,
    });
    await stripe.retrievePaymentIntent(SECRET);
    expect(stub.requests[0]!.url.startsWith(INTENT_PATH)).toBe(true);
  });

  it("uses the injected fetch rather than the global one", async () => {
    const calls: string[] = [];
    const stub = await startBrowserStub((_req, res) =>
      json(res, 200, samplePaymentIntent()),
    );
    stubs.push(stub);
    const stripe = await loadStripe(PK, {
      baseUrl: stub.url,
      fetch: (input, init) => {
        // A `Request` stringifies to "[object Request]".
        calls.push(
          typeof input === "string"
            ? input
            : input instanceof URL
              ? input.href
              : input.url,
        );
        return fetch(input, init);
      },
    });
    await stripe.retrievePaymentIntent(SECRET);
    expect(calls).toHaveLength(1);
  });
});

describe("fetch init", () => {
  it("sets credentials: 'omit' and mode: 'cors' on a GET", async () => {
    const inits: RequestInit[] = [];
    const stub = await startBrowserStub((_req, res) =>
      json(res, 200, samplePaymentIntent()),
    );
    stubs.push(stub);
    const stripe = await loadStripe(PK, {
      baseUrl: stub.url,
      fetch: (input, init) => {
        if (init !== undefined) {
          inits.push(init);
        }
        return fetch(input, init);
      },
    });

    await stripe.retrievePaymentIntent(SECRET);

    expect(inits).toHaveLength(1);
    expect(inits[0]!.credentials).toBe("omit");
    expect(inits[0]!.mode).toBe("cors");
  });

  it("sets credentials: 'omit' and mode: 'cors' on a POST", async () => {
    const inits: RequestInit[] = [];
    const stub = await startBrowserStub((_req, res) =>
      json(res, 200, samplePaymentIntent({ status: "processing" })),
    );
    stubs.push(stub);
    const stripe = await loadStripe(PK, {
      baseUrl: stub.url,
      fetch: (input, init) => {
        if (init !== undefined) {
          inits.push(init);
        }
        return fetch(input, init);
      },
    });

    await stripe.confirmPayment({ clientSecret: SECRET });

    expect(inits).toHaveLength(1);
    expect(inits[0]!.credentials).toBe("omit");
    expect(inits[0]!.mode).toBe("cors");
  });
});

describe("privacy", () => {
  it("never reveals a client secret through inspect or JSON of the client", async () => {
    const { stripe } = await withStub(
      answering(200, samplePaymentIntent()).handler,
    );
    await stripe.retrievePaymentIntent(SECRET);

    // The object holds a publishable key (non-secret by construction, D1)
    // and a base URL, and nothing else — no last-request URL, no cached
    // secret. These two assertions are what keeps that true.
    for (const rendered of [inspect(stripe), JSON.stringify(stripe)]) {
      expect(rendered).not.toContain(SUFFIX);
      expect(rendered).not.toContain("_secret_");
    }
    expect(JSON.parse(JSON.stringify(stripe))).toEqual({
      object: "vpay_stripe",
      publishableKey: PK,
      baseUrl: expect.stringContaining("http://127.0.0.1:") as unknown,
    });
  });

  it("keeps the client secret out of every error it builds", async () => {
    const cases: Array<() => Promise<PaymentIntentResult>> = [];
    const { stripe } = await withStub((_req, res) => {
      res.writeHead(502, { "Content-Type": "text/html" });
      res.end("<html/>");
    });
    cases.push(() => stripe.retrievePaymentIntent(SECRET));
    cases.push(() => stripe.confirmPayment({ clientSecret: SECRET }));
    cases.push(() => stripe.retrievePaymentIntent(`pi_1_secret_${SUFFIX}x`));

    const dead = await startBrowserStub((_req, res) => res.end());
    const deadUrl = dead.url;
    await dead.close();
    const offline = await loadStripe(PK, { baseUrl: deadUrl });
    cases.push(() => offline.retrievePaymentIntent(SECRET));

    for (const run of cases) {
      const result = await run();
      expect(result.error?.message ?? "").not.toContain(SUFFIX);
      expect(JSON.stringify(result)).not.toContain(SUFFIX);
    }
  });
});

describe("no logging", () => {
  it("has no console call anywhere in the shipping source", async () => {
    // A `console.log(url)` left in during debugging would print a
    // `client_secret` into the payer's devtools, and from there into any
    // error-reporting SDK the merchant's page has installed. Cheaper to
    // forbid outright than to review for.
    const { readdir, readFile } = await import("node:fs/promises");
    const { fileURLToPath } = await import("node:url");
    const dir = fileURLToPath(new URL(".", import.meta.url));
    const shipping = (await readdir(dir)).filter(
      (name) => name.endsWith(".ts") && !name.endsWith(".test.ts"),
    );
    expect(shipping.sort()).toEqual([
      "client.ts",
      "embedded.ts",
      "errors.ts",
      "form.ts",
      "index.ts",
      "types.ts",
    ]);
    for (const name of shipping) {
      const source = await readFile(`${dir}${name}`, "utf8");
      // Comments stripped first: this file's own prose talks about the
      // console, and a match there would be a false positive that the next
      // person "fixes" by weakening the pattern.
      const code = source
        .replace(/\/\*[\s\S]*?\*\//gu, "")
        .replace(/(^|[^:])\/\/.*$/gmu, "$1");
      expect(code, `${name} must not log`).not.toMatch(/console\s*\./u);
    }
  });

  it("posts no message into the frame, so there is no target origin to get wrong", async () => {
    // D8 forbids `postMessage(…, '*')` between vpay's embedded page and its
    // framer. The strongest form of that guarantee is that this side posts
    // nothing at all, and the cheapest way to keep it is to read the
    // shipping source — the same technique the test above uses to forbid a
    // `console` call. It lives here rather than in `embedded.test.ts`
    // because that file runs under jsdom, where `import.meta.url` is not a
    // file URL.
    const { readFile } = await import("node:fs/promises");
    const { fileURLToPath } = await import("node:url");
    const dir = fileURLToPath(new URL(".", import.meta.url));
    const source = await readFile(`${dir}embedded.ts`, "utf8");
    const code = source
      .replace(/\/\*[\s\S]*?\*\//gu, "")
      .replace(/(^|[^:])\/\/.*$/gmu, "$1");

    expect(code).not.toMatch(/postMessage/u);
  });
});
