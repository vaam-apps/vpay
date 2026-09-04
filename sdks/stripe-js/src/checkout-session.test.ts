/**
 * `retrieveCheckoutSession` against the same real `node:http` stub of
 * `/v1/browser` the rest of this suite drives (`src/testing/browser-stub.ts`
 * — an SDK's own unit-test server, not a test double reachable from a
 * shipping process in ADR-0006's sense).
 *
 * The session is a **second** payer credential (Step 9's D1): `cs_…_secret_…`
 * authorises reading one checkout session and nothing else, and in
 * particular is not the intent's secret that authorises `confirm`. These
 * tests pin the three things that keeps true — the URL the secret is sent
 * in, the object that comes back, and the byte-identical 404 that every
 * credential failure renders.
 *
 * The object that comes back carries **both** credentials, by the
 * integrator's ruling of 2026-09-04: on the browser routes
 * `payment_intent` is the expanded intent, `client_secret` and all, because
 * vpay's checkout page confirms and polls it through the existing browser
 * routes. So `retrieveCheckoutSession` hands its caller a live confirm
 * credential as well as a session-read one, and the tests below say so
 * out loud rather than leaving it to be discovered.
 */
import { inspect } from "node:util";
import { afterEach, describe, expect, it } from "vitest";
import { loadStripe } from "./index.js";
import {
  json,
  sampleCheckoutSession,
  samplePaymentIntent,
  sessionNotFoundEnvelope,
  startBrowserStub,
  type BrowserStub,
  type StubHandler,
} from "./testing/browser-stub.js";
import type { Stripe } from "./types.js";

const PK = "pk_test_abcdefghijklmnop";
const SUFFIX = "a".repeat(32);
const SESSION_SECRET = `cs_123_secret_${SUFFIX}`;
/** The *intent's* secret, which the expanded `payment_intent` carries. */
const INTENT_SECRET = `pi_123_secret_${SUFFIX}`;
const SESSION_PATH = "/v1/browser/checkout/sessions/cs_123";

const stubs: BrowserStub[] = [];

afterEach(async () => {
  await Promise.all(stubs.splice(0).map((stub) => stub.close()));
});

async function withStub(
  handler: StubHandler,
): Promise<{ stub: BrowserStub; stripe: Stripe }> {
  const stub = await startBrowserStub(handler);
  stubs.push(stub);
  const stripe = await loadStripe(PK, { baseUrl: stub.url });
  return { stub, stripe };
}

function answering(status: number, body: unknown): StubHandler {
  return (_req, res) => json(res, status, body);
}

describe("retrieveCheckoutSession", () => {
  it("GETs the browser checkout-session route with key and client_secret in the query string", async () => {
    const session = sampleCheckoutSession();
    const { stub, stripe } = await withStub(answering(200, session));

    const result = await stripe.retrieveCheckoutSession(SESSION_SECRET);

    expect(result.error).toBeUndefined();
    expect(result.checkoutSession).toEqual(session);
    expect(stub.requests).toHaveLength(1);
    const request = stub.requests[0]!;
    expect(request.method).toBe("GET");
    expect(request.url).toBe(
      `${SESSION_PATH}?key=${PK}&client_secret=cs_123_secret_${SUFFIX}`,
    );
    expect(request.body).toBe("");
    // The same rule as the intent routes: a browser POST carrying an
    // `Idempotency-Key` or an `Authorization` would be CORS-preflighted,
    // and this GET carries neither.
    expect(request.headers["idempotency-key"]).toBeUndefined();
    expect(request.headers["authorization"]).toBeUndefined();
  });

  it("renders all fourteen keys of the checkout session", async () => {
    const { stripe } = await withStub(answering(200, sampleCheckoutSession()));

    const result = await stripe.retrieveCheckoutSession(SESSION_SECRET);

    expect(Object.keys(result.checkoutSession ?? {}).sort()).toEqual([
      "cancel_url",
      "client_secret",
      "created",
      "expires_at",
      "id",
      "livemode",
      "object",
      "payment_intent",
      "payment_status",
      "return_url",
      "status",
      "success_url",
      "ui_mode",
      "url",
    ]);
  });

  it("expands payment_intent into the whole intent, with the intent's own client_secret typed", async () => {
    // The integrator's ruling of 2026-09-04 on the one place the plan's
    // wire contract was readable two ways. On `/v1` the field is the `pi_…`
    // id — `@vpay/sdk` and `vpay_sdk` both keep it a string — and on the
    // browser routes it is the whole intent. A test that only checked
    // `typeof payment_intent === "object"` would pass for a half-rendered
    // one, so this asserts the intent's thirteen keys.
    const session = sampleCheckoutSession();
    const { stripe } = await withStub(answering(200, session));

    const result = await stripe.retrieveCheckoutSession(SESSION_SECRET);

    const intent = result.checkoutSession?.payment_intent;
    expect(Object.keys(intent ?? {}).sort()).toEqual([
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
    ]);
    // Typed access, no cast and no narrowing: `payment_intent` is a
    // `PaymentIntent`, not `string | PaymentIntent`, and its
    // `client_secret` is `string`, not `string | undefined`.
    expect(intent?.id).toBe("pi_123");
    expect(intent?.object).toBe("payment_intent");
    expect(intent?.status).toBe("requires_payment_method");
    expect(intent?.client_secret).toBe(INTENT_SECRET);
    // …and the session's own secret is a different value, for a different
    // authorisation. Reading the session never became confirming it.
    expect(result.checkoutSession?.client_secret).toBe(SESSION_SECRET);
    expect(result.checkoutSession?.client_secret).not.toBe(
      intent?.client_secret,
    );
  });

  it("keeps the expanded intent's secret out of the client's diagnostics, exactly as it keeps the session's", async () => {
    // `@vpay/stripe-js` does not wrap wire objects, so there is no
    // `CheckoutSession` inspect hook to redact through — the merchant SDKs
    // have that because they hand a long-lived object to a server-side
    // logger. What this package guarantees instead, for both credentials
    // equally, is that the `Stripe` object retains neither and that no
    // error it builds quotes either. The expanded intent doubles the number
    // of secrets flowing through `retrieveCheckoutSession`, so it doubles
    // what this has to hold for.
    const { stripe } = await withStub(answering(200, sampleCheckoutSession()));
    await stripe.retrieveCheckoutSession(SESSION_SECRET);

    for (const rendered of [inspect(stripe), JSON.stringify(stripe)]) {
      expect(rendered).not.toContain(SUFFIX);
      expect(rendered).not.toContain("_secret_");
    }

    // The same for the error path: a 502 that is not the envelope.
    const { stripe: failing } = await withStub((_req, res) => {
      res.writeHead(502, { "Content-Type": "text/html" });
      res.end("<html/>");
    });
    const failed = await failing.retrieveCheckoutSession(SESSION_SECRET);
    expect(JSON.stringify(failed.error)).not.toContain(SUFFIX);
    expect(JSON.stringify(failed.error)).not.toContain(INTENT_SECRET);
  });

  it("reads a hosted session's url and both forwarding URLs", async () => {
    const hosted = sampleCheckoutSession({
      ui_mode: "hosted",
      success_url: "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
      cancel_url: "https://shop.example/cancel",
      return_url: null,
      url: `https://checkout.example/c/cs_123#${SESSION_SECRET}`,
    });
    const { stripe } = await withStub(answering(200, hosted));

    const result = await stripe.retrieveCheckoutSession(SESSION_SECRET);

    expect(result.checkoutSession?.ui_mode).toBe("hosted");
    // D5: the placeholder is carried verbatim, never substituted here — the
    // substitution is vpay's, at the moment it forwards the payer.
    expect(result.checkoutSession?.success_url).toContain(
      "{CHECKOUT_SESSION_ID}",
    );
    expect(result.checkoutSession?.return_url).toBeNull();
  });

  it("maps the uniform 404 every checkout-session credential failure renders", async () => {
    const { stripe } = await withStub(
      answering(404, sessionNotFoundEnvelope("cs_123")),
    );

    const result = await stripe.retrieveCheckoutSession(SESSION_SECRET);

    expect(result.checkoutSession).toBeUndefined();
    expect(result.error).toEqual({
      type: "invalid_request_error",
      code: "resource_missing",
      message: "No such checkout session: cs_123",
    });
  });

  it("refuses a payment-intent secret where a checkout-session secret belongs, without sending anything", async () => {
    // The two credentials are the same shape and mean different things:
    // `pi_…_secret_…` authorises `confirm`, `cs_…_secret_…` authorises
    // reading a session. Sending the first to the session route would put a
    // live confirm credential in a URL the session route logs.
    const { stub, stripe } = await withStub(
      answering(200, sampleCheckoutSession()),
    );

    const result = await stripe.retrieveCheckoutSession(
      `pi_123_secret_${SUFFIX}`,
    );

    expect(stub.requests).toHaveLength(0);
    expect(result.error?.type).toBe("invalid_request_error");
    expect(result.error?.code).toBe("invalid_request");
    expect(result.error?.param).toBe("clientSecret");
    // The refused value is never quoted back — it is a live credential.
    expect(result.error?.message).not.toContain(SUFFIX);
  });

  it("reports a 200 that is not a checkout session as unexpected_response", async () => {
    // A payment intent is a 200 with a perfectly good `object` — just not
    // this one. Accepting it would hand the caller a `CheckoutSession`
    // typed object with none of its fields.
    const { stripe } = await withStub(answering(200, samplePaymentIntent()));

    const result = await stripe.retrieveCheckoutSession(SESSION_SECRET);

    expect(result.checkoutSession).toBeUndefined();
    expect(result.error?.code).toBe("unexpected_response");
  });

  it("reports a refused connection on the session route as api_connection_error and never rejects", async () => {
    const stub = await startBrowserStub(() => undefined);
    const url = stub.url;
    await stub.close();
    const stripe = await loadStripe(PK, { baseUrl: url });

    const result = await stripe.retrieveCheckoutSession(SESSION_SECRET);

    expect(result.error?.type).toBe("api_connection_error");
    expect(result.error?.code).toBeUndefined();
    expect(result.error?.message).not.toContain(SUFFIX);
  });

  it("keeps the session client secret out of a non-envelope failure it reports", async () => {
    const { stripe } = await withStub((_req, res) => {
      res.writeHead(502, { "Content-Type": "text/html" });
      res.end("<html/>");
    });

    const result = await stripe.retrieveCheckoutSession(SESSION_SECRET);

    expect(result.error?.code).toBe("unexpected_response");
    expect(JSON.stringify(result.error)).not.toContain(SUFFIX);
  });
});
