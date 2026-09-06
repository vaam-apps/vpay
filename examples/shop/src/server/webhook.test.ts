/**
 * The webhook endpoint's four rules, plus the settling map.
 *
 * The signature is built by `src/testing/webhook-signature.ts`, an
 * independent implementation of the header grammar in docs/flows/webhooks.md
 * — not by the SDK's own verifier, which is the thing under test here.
 */
import { beforeEach, describe, expect, it } from "vitest";
import { MemoryShopStore } from "../testing/memory-store";
import { signWebhook } from "../testing/webhook-signature";
import type { Product } from "./store/types";
import { SETTLING_EVENTS, handleWebhook } from "./webhook";

const SECRET = "whsec_shop_demo_secret";
const NOW = 1_756_913_600;
const INTENT = "pi_test_1";

const CATALOGUE: Product[] = [
  {
    id: "njangi-tote",
    name: "Njangi tote bag",
    description: "…",
    priceMinor: 12000,
    currency: "xaf",
  },
];

let store: MemoryShopStore;
let orderId: string;

beforeEach(async () => {
  store = new MemoryShopStore(CATALOGUE);
  const order = await store.createOrder({
    email: "buyer@example.test",
    currency: "xaf",
    totalMinor: 12000,
    items: [
      {
        productId: "njangi-tote",
        name: "Njangi tote bag",
        quantity: 1,
        unitMinor: 12000,
      },
    ],
  });
  orderId = order.id;
  await store.setPaymentIntentId(orderId, INTENT);
});

function eventBody(
  id: string,
  type: string,
  intentId: string = INTENT,
  object: Record<string, unknown> = {},
): string {
  return JSON.stringify({
    id,
    object: "event",
    type,
    created: NOW,
    livemode: false,
    data: { object: { id: intentId, object: "payment_intent", ...object } },
  });
}

/**
 * A `payment_intent.payment_failed` body carrying the field that says which
 * of the eleven outcomes happened.
 *
 * There is no `failed` status on a PaymentIntent: a rail failure returns it
 * to `requires_payment_method` with `last_payment_error` populated
 * (`docs/flows/failures.md`), which is exactly the shape written here.
 */
function failedEventBody(
  id: string,
  error: unknown,
  intentId: string = INTENT,
): string {
  return eventBody(id, "payment_intent.payment_failed", intentId, {
    status: "requires_payment_method",
    last_payment_error: error,
  });
}

function deliver(
  rawBody: string,
  options: { signatureHeader?: string | null; now?: number } = {},
) {
  return handleWebhook(
    { store, secret: SECRET, now: options.now ?? NOW },
    {
      rawBody,
      signatureHeader:
        options.signatureHeader === undefined
          ? signWebhook(rawBody, SECRET, options.now ?? NOW)
          : options.signatureHeader,
    },
  );
}

describe("a verified event", () => {
  it("marks the order paid, once, and records the delivery", async () => {
    const body = eventBody("evt_1", "payment_intent.succeeded");
    const result = await deliver(body);

    expect(result.status).toBe(200);
    expect(result.body).toEqual({ received: true, outcome: "applied" });
    expect((await store.getOrder(orderId))?.status).toBe("paid");
    expect(store.recordedEvents.size).toBe(1);
    expect(store.recordedEvents.get("evt_1")).toEqual({
      type: "payment_intent.succeeded",
      orderId,
    });
  });

  it("maps payment_failed to failed and canceled to cancelled", async () => {
    expect(SETTLING_EVENTS).toEqual({
      "payment_intent.succeeded": "paid",
      "payment_intent.payment_failed": "failed",
      "payment_intent.canceled": "cancelled",
    });

    const failed = await deliver(
      eventBody("evt_f", "payment_intent.payment_failed"),
    );
    expect(failed.status).toBe(200);
    expect((await store.getOrder(orderId))?.status).toBe("failed");
  });

  it("does not read an inherited property when the type is `constructor`", async () => {
    // `SETTLING_EVENTS["constructor"]` is a truthy function on a bare index.
    const result = await deliver(eventBody("evt_c", "constructor"));
    expect(result.status).toBe(200);
    expect(result.body).toEqual({ received: true, outcome: "ignored" });
    expect(store.recordedEvents.size).toBe(0);
    expect((await store.getOrder(orderId))?.status).toBe("unpaid");
  });

  it("acknowledges an event type it does not act on, and writes nothing", async () => {
    const result = await deliver(
      eventBody("evt_p", "payment_intent.processing"),
    );
    expect(result.status).toBe(200);
    expect(result.body).toEqual({ received: true, outcome: "ignored" });
    expect(store.recordedEvents.size).toBe(0);
    expect((await store.getOrder(orderId))?.status).toBe("unpaid");
  });
});

describe("a replay", () => {
  it("writes nothing the second time and is still 2xx", async () => {
    const body = eventBody("evt_1", "payment_intent.succeeded");
    await deliver(body);
    expect(store.recordedEvents.size).toBe(1);

    const replay = await deliver(body);

    expect(replay.status).toBe(200);
    expect(replay.body).toEqual({ received: true, outcome: "duplicate" });
    expect(store.recordedEvents.size).toBe(1);
    expect((await store.getOrder(orderId))?.status).toBe("paid");
  });

  it("does not un-pay a paid order when a later failure arrives", async () => {
    await deliver(eventBody("evt_1", "payment_intent.succeeded"));
    const late = await deliver(
      eventBody("evt_2", "payment_intent.payment_failed"),
    );
    expect(late.status).toBe(200);
    expect(late.body).toEqual({ received: true, outcome: "already_settled" });
    expect((await store.getOrder(orderId))?.status).toBe("paid");
  });
});

describe("a bad signature", () => {
  it("is 400 and writes nothing", async () => {
    const body = eventBody("evt_1", "payment_intent.succeeded");
    const result = await deliver(body, {
      signatureHeader: signWebhook(body, "whsec_the_wrong_secret", NOW),
    });

    expect(result.status).toBe(400);
    expect(result.body).toEqual({ error: "invalid signature" });
    expect(store.recordedEvents.size).toBe(0);
    expect((await store.getOrder(orderId))?.status).toBe("unpaid");
  });

  it("is 400 when the body was tampered with after signing", async () => {
    const signed = eventBody("evt_1", "payment_intent.succeeded");
    const header = signWebhook(signed, SECRET, NOW);
    const tampered = eventBody("evt_1", "payment_intent.succeeded").replace(
      '"livemode":false',
      '"livemode":true',
    );
    expect(tampered).not.toBe(signed);

    const result = await deliver(tampered, { signatureHeader: header });

    expect(result.status).toBe(400);
    expect(store.recordedEvents.size).toBe(0);
    expect((await store.getOrder(orderId))?.status).toBe("unpaid");
  });

  it("is 400 with no header at all, and writes nothing", async () => {
    const result = await deliver(
      eventBody("evt_1", "payment_intent.succeeded"),
      {
        signatureHeader: null,
      },
    );
    expect(result.status).toBe(400);
    expect(result.body).toEqual({ error: "missing signature" });
    expect(store.recordedEvents.size).toBe(0);
    expect((await store.getOrder(orderId))?.status).toBe("unpaid");
  });

  it("is 400 for a correctly signed body whose timestamp is outside the tolerance", async () => {
    const body = eventBody("evt_1", "payment_intent.succeeded");
    const stale = NOW - 3600;
    const result = await deliver(body, {
      signatureHeader: signWebhook(body, SECRET, stale),
      now: NOW,
    });
    expect(result.status).toBe(400);
    expect(store.recordedEvents.size).toBe(0);
    expect((await store.getOrder(orderId))?.status).toBe("unpaid");
  });

  it("says only 'invalid signature', never which check failed", async () => {
    const body = eventBody("evt_1", "payment_intent.succeeded");
    for (const header of [
      "t=,v1=deadbeef",
      `t=${NOW}`,
      `t=${NOW},v1=00`,
      "garbage",
    ]) {
      const result = await deliver(body, { signatureHeader: header });
      expect(result.status).toBe(400);
      expect(result.body).toEqual({ error: "invalid signature" });
    }
  });
});

describe("an event for an intent no order carries", () => {
  it("is 2xx and writes nothing", async () => {
    const result = await deliver(
      eventBody("evt_other", "payment_intent.succeeded", "pi_someone_else"),
    );
    expect(result.status).toBe(200);
    expect(result.body).toEqual({ received: true, outcome: "unknown_intent" });
    expect(store.recordedEvents.size).toBe(0);
    expect((await store.getOrder(orderId))?.status).toBe("unpaid");
  });
});

describe("a settling event whose object has no id", () => {
  it("is 400 rather than a guess", async () => {
    const body = JSON.stringify({
      id: "evt_weird",
      object: "event",
      type: "payment_intent.succeeded",
      created: NOW,
      livemode: false,
      data: { object: { object: "payment_intent" } },
    });
    const result = await deliver(body);
    expect(result.status).toBe(400);
    expect(store.recordedEvents.size).toBe(0);
    expect((await store.getOrder(orderId))?.status).toBe("unpaid");
  });
});

describe("last_payment_error, carried onto the order", () => {
  it("stores the code and the message from the event that failed the order", async () => {
    const result = await deliver(
      failedEventBody("evt_lpe", {
        code: "insufficient_funds",
        message: "NOT_ENOUGH_FUNDS",
      }),
    );
    expect(result.status).toBe(200);
    const order = await store.getOrder(orderId);
    expect(order?.status).toBe("failed");
    expect(order?.failureCode).toBe("insufficient_funds");
    expect(order?.failureMessage).toBe("NOT_ENOUGH_FUNDS");
  });

  it("still fails the order when the event carries no last_payment_error", async () => {
    // "The payment failed and we cannot say why" is a true thing to tell a
    // buyer. Guessing at which of the eleven codes it might have been is not.
    for (const shape of [undefined, null, "insufficient_funds", 7, []]) {
      store = new MemoryShopStore(CATALOGUE);
      const order = await store.createOrder({
        email: null,
        currency: "xaf",
        totalMinor: 12000,
        items: [],
      });
      await store.setPaymentIntentId(order.id, INTENT);
      await deliver(failedEventBody("evt_none", shape));
      const settled = await store.getOrder(order.id);
      expect(settled?.status).toBe("failed");
      expect(settled?.failureCode).toBeNull();
      expect(settled?.failureMessage).toBeNull();
    }
  });

  it("reads neither member unless it is a non-empty string", async () => {
    await deliver(failedEventBody("evt_odd", { code: "", message: 42 }));
    const order = await store.getOrder(orderId);
    expect(order?.failureCode).toBeNull();
    expect(order?.failureMessage).toBeNull();
  });

  it("never stamps a failure onto an order that has already settled", async () => {
    await deliver(eventBody("evt_ok", "payment_intent.succeeded"));
    const late = await deliver(
      failedEventBody("evt_late", {
        code: "provider_error",
        message: "too late",
      }),
    );
    // The delivery is recorded and the order is untouched: `paid` is
    // terminal here, and a later failure must not un-pay a shipped order —
    // nor leave a failure code on one.
    expect(late.body).toEqual({ received: true, outcome: "already_settled" });
    const order = await store.getOrder(orderId);
    expect(order?.status).toBe("paid");
    expect(order?.failureCode).toBeNull();
    expect(order?.failureMessage).toBeNull();
  });

  it("writes nothing at all for a replay of the same event id", async () => {
    await deliver(
      failedEventBody("evt_dup", {
        code: "payer_timeout",
        message: "COULD_NOT_PERFORM_TRANSACTION",
      }),
    );
    const replay = await deliver(
      failedEventBody("evt_dup", { code: "provider_error", message: "other" }),
    );
    expect(replay.body).toEqual({ received: true, outcome: "duplicate" });
    const order = await store.getOrder(orderId);
    expect(order?.failureCode).toBe("payer_timeout");
    expect(order?.failureMessage).toBe("COULD_NOT_PERFORM_TRANSACTION");
  });
});
