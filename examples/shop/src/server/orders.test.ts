/**
 * The tRPC procedures, against a real `VpayClient` pointed at a real local
 * HTTP server.
 *
 * What is asserted is what went on the wire: the amount the shop computed
 * (never the one the browser sent), the `Idempotency-Key` derived from the
 * order id, the exact session parameters including the literal
 * `{CHECKOUT_SESSION_ID}` placeholder, and the two ids the shop stored.
 */
import { afterEach, describe, expect, it } from "vitest";
import { VpayClient } from "@vaam-apps/vpay-sdk";
import { MemoryShopStore } from "../testing/memory-store";
import { testPrivateKeyPem } from "../testing/keys";
import {
  reply,
  startVpayTestServer,
  type RouteHandler,
  type VpayTestServer,
} from "../testing/vpay-test-server";
import {
  cancelOrder,
  cancelUrl,
  embeddedClientSecret,
  idempotencyKeys,
  placeOrder,
  priceCart,
  retryOrder,
  returnUrl,
  type OrderDeps,
} from "./orders";
import type { Product } from "./store/types";

const SHOP_URL = "http://shop.test";

const CATALOGUE: Product[] = [
  {
    id: "mbanga-coffee-1kg",
    name: "Mbanga highland coffee, 1 kg",
    description: "…",
    priceMinor: 7500,
    currency: "xaf",
  },
  {
    id: "njangi-tote",
    name: "Njangi tote bag",
    description: "…",
    priceMinor: 12000,
    currency: "xaf",
  },
  {
    id: "eur-oddity",
    name: "Priced in another currency",
    description: "…",
    priceMinor: 5000,
    currency: "eur",
  },
];

const servers: VpayTestServer[] = [];

afterEach(async () => {
  await Promise.all(servers.splice(0).map((server) => server.close()));
});

function intentResponse(id: string, amount: number): RouteHandler {
  return reply(201, {
    id,
    object: "payment_intent",
    amount,
    currency: "xaf",
    status: "requires_payment_method",
    payment_method_types: ["mtn_momo", "orange_money"],
    next_action: null,
    last_payment_error: null,
    metadata: {},
    description: null,
    created: 1_756_913_600,
    livemode: false,
    client_secret: `${id}_secret_notlogged`,
  });
}

function sessionResponse(overrides: Record<string, unknown>): RouteHandler {
  return reply(201, {
    id: "cs_test_1",
    object: "checkout.session",
    livemode: false,
    payment_intent: "pi_test_1",
    ui_mode: "hosted",
    status: "open",
    payment_status: "unpaid",
    success_url: null,
    cancel_url: null,
    return_url: null,
    url: "http://checkout.test/c/cs_test_1#cs_test_1_secret_abc",
    expires_at: 1_757_000_000,
    created: 1_756_913_600,
    client_secret: "cs_test_1_secret_abc",
    ...overrides,
  });
}

async function deps(
  routes: Record<string, RouteHandler>,
  products: Product[] = CATALOGUE,
): Promise<{
  deps: OrderDeps;
  server: VpayTestServer;
  store: MemoryShopStore;
}> {
  const server = await startVpayTestServer({ routes });
  servers.push(server);
  const store = new MemoryShopStore(products);
  return {
    server,
    store,
    deps: {
      store,
      vpay: new VpayClient({
        baseUrl: server.url,
        clientId: "shop-merchant",
        privateKey: testPrivateKeyPem(),
      }),
      shopPublicUrl: SHOP_URL,
      rails: { kind: "all", rails: ["mtn_momo", "orange_money"] },
    },
  };
}

describe("idempotencyKeys", () => {
  it("derives all three keys from the order id and nothing else", () => {
    expect(idempotencyKeys("ord_1")).toEqual({
      paymentIntent: "shop-order-ord_1-intent",
      hostedSession: "shop-order-ord_1-session-hosted",
      embeddedSession: "shop-order-ord_1-session-embedded",
    });
    // Called twice with the same id it must be the same three strings —
    // that is the whole reason the key is derived rather than random.
    expect(idempotencyKeys("ord_1")).toEqual(idempotencyKeys("ord_1"));
    expect(idempotencyKeys("ord_2").paymentIntent).not.toEqual(
      idempotencyKeys("ord_1").paymentIntent,
    );
  });
});

describe("the return and cancel URLs", () => {
  it("carries the placeholder as a literal, unescaped", () => {
    expect(returnUrl(SHOP_URL, "ord_1")).toBe(
      "http://shop.test/orders/ord_1/return?session_id={CHECKOUT_SESSION_ID}",
    );
    // %7BCHECKOUT_SESSION_ID%7D would not be substituted by vpay (D5).
    expect(returnUrl(SHOP_URL, "ord_1")).not.toContain("%7B");
    expect(cancelUrl(SHOP_URL, "ord_1")).toBe(
      "http://shop.test/orders/ord_1/cancelled",
    );
  });
});

describe("priceCart", () => {
  it("totals from the catalogue, in integer minor units", async () => {
    const store = new MemoryShopStore(CATALOGUE);
    const priced = await priceCart(store, [
      { productId: "mbanga-coffee-1kg", quantity: 2 },
      { productId: "njangi-tote", quantity: 1 },
    ]);
    expect(priced.totalMinor).toBe(7500 * 2 + 12000);
    expect(priced.currency).toBe("xaf");
    expect(priced.items).toEqual([
      {
        productId: "mbanga-coffee-1kg",
        name: "Mbanga highland coffee, 1 kg",
        quantity: 2,
        unitMinor: 7500,
      },
      {
        productId: "njangi-tote",
        name: "Njangi tote bag",
        quantity: 1,
        unitMinor: 12000,
      },
    ]);
  });

  it("merges duplicate lines rather than double-counting them", async () => {
    const store = new MemoryShopStore(CATALOGUE);
    const priced = await priceCart(store, [
      { productId: "njangi-tote", quantity: 1 },
      { productId: "njangi-tote", quantity: 2 },
    ]);
    expect(priced.items).toHaveLength(1);
    expect(priced.items[0]?.quantity).toBe(3);
    expect(priced.totalMinor).toBe(36000);
  });

  it("refuses a product that is not in the catalogue", async () => {
    const store = new MemoryShopStore(CATALOGUE);
    await expect(
      priceCart(store, [{ productId: "free-stuff", quantity: 1 }]),
    ).rejects.toThrow(/no such product: free-stuff/);
  });

  it("refuses a cart that mixes currencies rather than summing it", async () => {
    const store = new MemoryShopStore(CATALOGUE);
    await expect(
      priceCart(store, [
        { productId: "njangi-tote", quantity: 1 },
        { productId: "eur-oddity", quantity: 1 },
      ]),
    ).rejects.toThrow(/may not mix currencies/);
  });

  it("refuses an empty cart and a non-positive quantity", async () => {
    const store = new MemoryShopStore(CATALOGUE);
    await expect(priceCart(store, [])).rejects.toThrow(/cart is empty/);
    await expect(
      priceCart(store, [{ productId: "njangi-tote", quantity: 0 }]),
    ).rejects.toThrow(/positive integer/);
  });
});

describe("placeOrder, hosted", () => {
  it("sends the amount it computed, keys both calls off the order id, and stores both vpay ids", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 27000),
      "POST /v1/checkout/sessions": sessionResponse({}),
    });

    const result = await placeOrder(context.deps, {
      email: "buyer@example.test",
      lines: [
        { productId: "mbanga-coffee-1kg", quantity: 2 },
        { productId: "njangi-tote", quantity: 1 },
      ],
      mode: "hosted",
    });

    expect(result.orderId).toBe("ord_1");
    expect(result.url).toBe(
      "http://checkout.test/c/cs_test_1#cs_test_1_secret_abc",
    );

    const [intentRequest] = context.server.requestsTo(
      "POST",
      "/v1/payment_intents",
    );
    expect(intentRequest).toBeDefined();
    expect(intentRequest?.headers["idempotency-key"]).toBe(
      "shop-order-ord_1-intent",
    );
    expect(intentRequest?.form.get("amount")).toBe("27000");
    expect(intentRequest?.form.get("currency")).toBe("xaf");
    expect(intentRequest?.form.get("payment_method_types[0]")).toBe("mtn_momo");
    expect(intentRequest?.form.get("payment_method_types[1]")).toBe(
      "orange_money",
    );
    expect(intentRequest?.form.get("metadata[shop_order_id]")).toBe("ord_1");

    const [sessionRequest] = context.server.requestsTo(
      "POST",
      "/v1/checkout/sessions",
    );
    expect(sessionRequest).toBeDefined();
    expect(sessionRequest?.headers["idempotency-key"]).toBe(
      "shop-order-ord_1-session-hosted",
    );
    expect(Object.fromEntries(sessionRequest?.form ?? [])).toEqual({
      payment_intent: "pi_test_1",
      ui_mode: "hosted",
      success_url:
        "http://shop.test/orders/ord_1/return?session_id={CHECKOUT_SESSION_ID}",
      cancel_url: "http://shop.test/orders/ord_1/cancelled",
    });

    const stored = await context.store.getOrder("ord_1");
    expect(stored?.paymentIntentId).toBe("pi_test_1");
    expect(stored?.checkoutSessionId).toBe("cs_test_1");
    expect(stored?.totalMinor).toBe(27000);
    expect(stored?.status).toBe("unpaid");
  });

  it("never lets the browser name a price: only ids and quantities are sent", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({}),
    });
    await placeOrder(context.deps, {
      email: "buyer@example.test",
      // A caller that tried to smuggle a price in has nowhere to put it —
      // `CartLine` has two members — and the amount below is the catalogue's.
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "hosted",
    });
    const [intentRequest] = context.server.requestsTo(
      "POST",
      "/v1/payment_intents",
    );
    expect(intentRequest?.form.get("amount")).toBe("12000");
  });

  it("persists the intent id before the session is created", async () => {
    let intentIdAtSessionTime: string | null | undefined;
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": (_request, response) => {
        // Read the store at the moment vpay is asked for a session: the
        // order must already name the intent (AGENTS.md's crash-safety rule).
        void context.store.getOrder("ord_1").then((order) => {
          intentIdAtSessionTime = order?.paymentIntentId;
          sessionResponse({})(_request, response);
        });
      },
    });
    await placeOrder(context.deps, {
      email: "buyer@example.test",
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "hosted",
    });
    expect(intentIdAtSessionTime).toBe("pi_test_1");
  });

  it("refuses a hosted session that came back without a url instead of guessing one", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({ url: null }),
    });
    await expect(
      placeOrder(context.deps, {
        email: "buyer@example.test",
        lines: [{ productId: "njangi-tote", quantity: 1 }],
        mode: "hosted",
      }),
    ).rejects.toThrow(/no url/);
  });
});

describe("placeOrder, embedded", () => {
  it("creates the intent and no session, so the intent has no open session yet", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
    });
    const result = await placeOrder(context.deps, {
      email: "buyer@example.test",
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "embedded",
    });
    expect(result.url).toBeNull();
    expect(
      context.server.requestsTo("POST", "/v1/checkout/sessions"),
    ).toHaveLength(0);
    const stored = await context.store.getOrder(result.orderId);
    expect(stored?.paymentIntentId).toBe("pi_test_1");
    expect(stored?.checkoutSessionId).toBeNull();
  });
});

describe("embeddedClientSecret", () => {
  it("creates an embedded session with the shop's return URL and answers with its secret", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({
        ui_mode: "embedded",
        url: null,
        return_url:
          "http://shop.test/orders/ord_1/return?session_id={CHECKOUT_SESSION_ID}",
      }),
    });
    const { orderId } = await placeOrder(context.deps, {
      email: "buyer@example.test",
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "embedded",
    });

    const secret = await embeddedClientSecret(context.deps, orderId);
    expect(secret).toEqual({
      clientSecret: "cs_test_1_secret_abc",
      sessionId: "cs_test_1",
    });

    const [sessionRequest] = context.server.requestsTo(
      "POST",
      "/v1/checkout/sessions",
    );
    expect(sessionRequest?.headers["idempotency-key"]).toBe(
      "shop-order-ord_1-session-embedded",
    );
    expect(Object.fromEntries(sessionRequest?.form ?? [])).toEqual({
      payment_intent: "pi_test_1",
      ui_mode: "embedded",
      return_url:
        "http://shop.test/orders/ord_1/return?session_id={CHECKOUT_SESSION_ID}",
    });
    expect((await context.store.getOrder(orderId))?.checkoutSessionId).toBe(
      "cs_test_1",
    );
  });

  it("retrieves the session it already made rather than creating a second one", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({
        ui_mode: "embedded",
        url: null,
      }),
      "GET /v1/checkout/sessions/cs_test_1": sessionResponse({
        ui_mode: "embedded",
        url: null,
      }),
    });
    const { orderId } = await placeOrder(context.deps, {
      email: "buyer@example.test",
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "embedded",
    });

    await embeddedClientSecret(context.deps, orderId);
    const again = await embeddedClientSecret(context.deps, orderId);

    expect(again.sessionId).toBe("cs_test_1");
    // One create, one retrieve — never two creates. D1 allows one open
    // session per PaymentIntent.
    expect(
      context.server.requestsTo("POST", "/v1/checkout/sessions"),
    ).toHaveLength(1);
    expect(
      context.server.requestsTo("GET", "/v1/checkout/sessions/cs_test_1"),
    ).toHaveLength(1);
  });

  it("refuses to hand back the secret of a session that is no longer open", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({
        ui_mode: "embedded",
        url: null,
      }),
      "GET /v1/checkout/sessions/cs_test_1": sessionResponse({
        ui_mode: "embedded",
        url: null,
        status: "expired",
      }),
    });
    const { orderId } = await placeOrder(context.deps, {
      email: "buyer@example.test",
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "embedded",
    });
    await embeddedClientSecret(context.deps, orderId);
    await expect(embeddedClientSecret(context.deps, orderId)).rejects.toThrow(
      /is expired, not open/,
    );
  });

  it("refuses to mint an embedded session for an order that already has a hosted one", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({}),
      "GET /v1/checkout/sessions/cs_test_1": sessionResponse({}),
    });
    const { orderId } = await placeOrder(context.deps, {
      email: "buyer@example.test",
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "hosted",
    });
    await expect(embeddedClientSecret(context.deps, orderId)).rejects.toThrow(
      /already has a hosted checkout session/,
    );
  });

  it("refuses an order that is already settled", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
    });
    const { orderId } = await placeOrder(context.deps, {
      email: "buyer@example.test",
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "embedded",
    });
    await context.store.applyWebhookEvent({
      eventId: "evt_1",
      type: "payment_intent.succeeded",
      paymentIntentId: "pi_test_1",
      nextStatus: "paid",
      failureCode: null,
      failureMessage: null,
    });
    await expect(embeddedClientSecret(context.deps, orderId)).rejects.toThrow(
      /is paid; there is nothing to pay/,
    );
  });
});

describe("the three integration surfaces", () => {
  it("mints one hosted session for `popup`, with the hosted key, and answers its url", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({}),
    });
    const result = await placeOrder(context.deps, {
      email: null,
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "popup",
    });
    expect(result.url).toBe(
      "http://checkout.test/c/cs_test_1#cs_test_1_secret_abc",
    );
    const session = context.server.requests.find(
      (request) => request.url === "/v1/checkout/sessions",
    );
    // The SAME key `hosted` would have sent, because it is the same request:
    // a payer who falls back from a blocked popup to a redirect must get the
    // session they already had, not a second one.
    expect(session?.headers["idempotency-key"]).toBe(
      idempotencyKeys(result.orderId).hostedSession,
    );
    expect(session?.body).toContain("ui_mode=hosted");
    expect(session?.body).toContain("success_url=");
  });

  it("mints no session at all for `embedded`", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
    });
    const result = await placeOrder(context.deps, {
      email: null,
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "embedded",
    });
    expect(result.url).toBeNull();
    expect(
      context.server.requests.some((r) => r.url === "/v1/checkout/sessions"),
    ).toBe(false);
  });
});

describe("the e-mail is optional", () => {
  it("places an order with no e-mail at all and stores null", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({}),
    });
    const { orderId } = await placeOrder(context.deps, {
      email: null,
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "hosted",
    });
    expect((await context.store.getOrder(orderId))?.email).toBeNull();
  });

  it("never puts the buyer's e-mail on the wire to vpay", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({}),
    });
    await placeOrder(context.deps, {
      email: "buyer@example.test",
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "hosted",
    });
    for (const request of context.server.requests) {
      expect(request.body).not.toContain("buyer%40example.test");
      expect(request.body).not.toContain("buyer@example.test");
    }
  });
});

describe("which rails an order is offered", () => {
  it("offers only the rails configured for the order's currency", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 5000),
      "POST /v1/checkout/sessions": sessionResponse({}),
    });
    context.deps.rails = {
      kind: "by_currency",
      byCurrency: { xaf: ["orange_money"], eur: ["mtn_momo"] },
    };
    const result = await placeOrder(context.deps, {
      email: null,
      lines: [{ productId: "eur-oddity", quantity: 1 }],
      mode: "hosted",
    });
    expect(result.paymentMethodTypes).toEqual(["mtn_momo"]);
    const intent = context.server.requests.find(
      (request) => request.url === "/v1/payment_intents",
    );
    expect(intent?.body).toContain("payment_method_types[0]=mtn_momo");
    expect(intent?.body).not.toContain("orange_money");
  });

  it("refuses the order, and writes no row, when no rail settles its currency", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 5000),
    });
    context.deps.rails = {
      kind: "by_currency",
      byCurrency: { xaf: ["orange_money"] },
    };
    await expect(
      placeOrder(context.deps, {
        email: null,
        lines: [{ productId: "eur-oddity", quantity: 1 }],
        mode: "hosted",
      }),
    ).rejects.toThrow(/offers no payment rail for EUR/);
    // The refusal has to come before the order row, or the shop is left
    // holding an order nothing can ever pay.
    expect(context.store.orders.size).toBe(0);
    expect(context.server.requests).toEqual([]);
  });
});

describe("retrying a failed order", () => {
  it("places a NEW order with the same lines, and its own idempotency keys", async () => {
    // A fresh intent id per create, because the store enforces the `@unique`
    // on `payment_intent_id` exactly as Postgres does — a retry that reused
    // one would be the second charge on one intent that vpay forbids.
    let minted = 0;
    const context = await deps({
      "POST /v1/payment_intents": (request, response) => {
        minted += 1;
        intentResponse(`pi_test_${minted}`, 12000)(request, response);
      },
      "POST /v1/checkout/sessions": sessionResponse({}),
    });
    const first = await placeOrder(context.deps, {
      email: "buyer@example.test",
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "hosted",
    });
    const second = await retryOrder(context.deps, first.orderId, "hosted");
    expect(second.orderId).not.toBe(first.orderId);

    const original = await context.store.getOrder(first.orderId);
    const retry = await context.store.getOrder(second.orderId);
    expect(retry?.items).toEqual(original?.items);
    expect(retry?.email).toBe("buyer@example.test");
    // The failed order is left exactly as it was: `failed` is terminal here.
    expect(original?.status).toBe("unpaid");

    const intents = context.server.requests.filter(
      (request) => request.url === "/v1/payment_intents",
    );
    expect(intents).toHaveLength(2);
    expect(intents[0]?.headers["idempotency-key"]).not.toBe(
      intents[1]?.headers["idempotency-key"],
    );
  });

  it("refuses to retry an order that is already paid", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({}),
    });
    const { orderId } = await placeOrder(context.deps, {
      email: null,
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "hosted",
    });
    await context.store.applyWebhookEvent({
      eventId: "evt_1",
      type: "payment_intent.succeeded",
      paymentIntentId: "pi_test_1",
      nextStatus: "paid",
      failureCode: null,
      failureMessage: null,
    });
    await expect(retryOrder(context.deps, orderId, "hosted")).rejects.toThrow(
      /is paid; there is nothing to retry/,
    );
  });
});

describe("cancelling an order", () => {
  it("cancels the intent at vpay and writes nothing itself", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({}),
      "POST /v1/payment_intents/pi_test_1/cancel": reply(200, {
        id: "pi_test_1",
        object: "payment_intent",
        amount: 12000,
        currency: "xaf",
        status: "canceled",
        payment_method_types: ["orange_money"],
        next_action: null,
        last_payment_error: null,
        metadata: {},
        description: null,
        created: 1_756_913_600,
        livemode: false,
      }),
    });
    const { orderId } = await placeOrder(context.deps, {
      email: null,
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "hosted",
    });
    const result = await cancelOrder(context.deps, orderId);
    expect(result.paymentIntentId).toBe("pi_test_1");
    expect(
      context.server.requests.some(
        (request) => request.url === "/v1/payment_intents/pi_test_1/cancel",
      ),
    ).toBe(true);
    // The status still comes from the webhook, and from nowhere else.
    expect((await context.store.getOrder(orderId))?.status).toBe("unpaid");
  });

  it("refuses to cancel an order that has already settled", async () => {
    const context = await deps({
      "POST /v1/payment_intents": intentResponse("pi_test_1", 12000),
      "POST /v1/checkout/sessions": sessionResponse({}),
    });
    const { orderId } = await placeOrder(context.deps, {
      email: null,
      lines: [{ productId: "njangi-tote", quantity: 1 }],
      mode: "hosted",
    });
    await context.store.applyWebhookEvent({
      eventId: "evt_1",
      type: "payment_intent.succeeded",
      paymentIntentId: "pi_test_1",
      nextStatus: "paid",
      failureCode: null,
      failureMessage: null,
    });
    await expect(cancelOrder(context.deps, orderId)).rejects.toThrow(
      /is paid; there is nothing to cancel/,
    );
  });
});
