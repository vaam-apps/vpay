/**
 * What happens when someone buys something: validate the cart against the
 * catalogue, total it **server-side**, create the PaymentIntent, persist its
 * id, then create the Checkout Session.
 *
 * The order of those last three steps is the point, and it is the repository's
 * own rule (AGENTS.md — "never let a payer act on a transaction you cannot
 * name"): the intent id is written to the shop's database *before* a session
 * that could send a payer to a page exists. A crash between the two leaves an
 * order that names a real intent, which the webhook can still settle.
 *
 * Written against {@link ShopStore} and a `VpayClient` rather than against
 * module-level singletons, so `orders.test.ts` can point a real `VpayClient`
 * at a real local HTTP server and assert the exact bytes that go on the wire.
 */
import { TRPCError } from "@trpc/server";
import type { VpayClient } from "@vpay/sdk";
import type { PaymentMethodType } from "./config";
import type { Order, OrderLine, ShopStore } from "./store/types";

/** The most of one product a single order may carry. A shop, not a wholesaler. */
export const MAX_QUANTITY_PER_LINE = 10;
/** The most distinct products one order may carry. */
export const MAX_LINES = 20;

export interface CartLine {
  productId: string;
  quantity: number;
}

export interface OrderDeps {
  store: ShopStore;
  vpay: VpayClient;
  /** The shop's own origin, no trailing slash. */
  shopPublicUrl: string;
  /** The rails to offer on the intent. */
  paymentMethodTypes: PaymentMethodType[];
}

/**
 * The three `Idempotency-Key` values one order can ever send, derived from
 * the order id and nothing else.
 *
 * Derived rather than random on purpose: every retry of the SAME order sends
 * the same key, so a transport retry, a re-invoked handler, or a second
 * `orders.embeddedSecret` call cannot leave vpay holding two intents or two
 * sessions for one order. The order id is minted by this shop's own database
 * before the first call, so it is available and stable for every retry —
 * which is exactly the property a random UUID would not have.
 *
 * **What it does not do**, stated because the opposite is easy to assume: it
 * does not deduplicate two separate checkout submissions. Pressing "Pay"
 * twice calls {@link placeOrder} twice, which writes two orders with two ids
 * and therefore two PaymentIntents — they are two orders, and vpay is right
 * to create both. The checkout form disables its buttons while a submission
 * is in flight, which is a mitigation in the UI and not a guarantee here.
 *
 * The hosted and embedded session keys differ because they are different
 * requests: replaying one key with a different body is what vpay's idempotency
 * layer refuses.
 */
export function idempotencyKeys(orderId: string): {
  paymentIntent: string;
  hostedSession: string;
  embeddedSession: string;
} {
  return {
    paymentIntent: `shop-order-${orderId}-intent`,
    hostedSession: `shop-order-${orderId}-session-hosted`,
    embeddedSession: `shop-order-${orderId}-session-embedded`,
  };
}

/**
 * `success_url` and `return_url`. `{CHECKOUT_SESSION_ID}` is a **literal**
 * template placeholder vpay substitutes when it forwards the payer (D5) — it
 * is deliberately not percent-encoded, and the shop's return page treats what
 * comes back as a label, never as authority.
 */
export function returnUrl(shopPublicUrl: string, orderId: string): string {
  return `${shopPublicUrl}/orders/${encodeURIComponent(orderId)}/return?session_id={CHECKOUT_SESSION_ID}`;
}

/** `cancel_url`. No placeholder: the cancelled page reads the order, not the URL. */
export function cancelUrl(shopPublicUrl: string, orderId: string): string {
  return `${shopPublicUrl}/orders/${encodeURIComponent(orderId)}/cancelled`;
}

export interface PlaceOrderInput {
  email: string;
  lines: CartLine[];
  /**
   * `hosted` mints the session here and answers with vpay's `url`.
   * `embedded` stops after the intent; the session is minted by
   * {@link embeddedClientSecret} when the framed page asks for it, so that
   * an order never holds two open sessions for one intent (D1's unique
   * `payment_intent`).
   */
  mode: "hosted" | "embedded";
}

export interface PlaceOrderResult {
  orderId: string;
  /** vpay's hosted page, or `null` in embedded mode. */
  url: string | null;
}

/** Merges duplicate lines and rejects a cart that could not be honoured. */
function normaliseLines(lines: readonly CartLine[]): CartLine[] {
  if (lines.length === 0) {
    throw new TRPCError({ code: "BAD_REQUEST", message: "the cart is empty" });
  }
  const merged = new Map<string, number>();
  for (const line of lines) {
    if (!Number.isInteger(line.quantity) || line.quantity < 1) {
      throw new TRPCError({
        code: "BAD_REQUEST",
        message: `quantity for ${line.productId} must be a positive integer`,
      });
    }
    merged.set(
      line.productId,
      (merged.get(line.productId) ?? 0) + line.quantity,
    );
  }
  if (merged.size > MAX_LINES) {
    throw new TRPCError({
      code: "BAD_REQUEST",
      message: `an order may carry at most ${MAX_LINES} distinct products`,
    });
  }
  const normalised: CartLine[] = [];
  for (const [productId, quantity] of merged) {
    if (quantity > MAX_QUANTITY_PER_LINE) {
      throw new TRPCError({
        code: "BAD_REQUEST",
        message: `at most ${MAX_QUANTITY_PER_LINE} of ${productId} per order`,
      });
    }
    normalised.push({ productId, quantity });
  }
  return normalised;
}

/**
 * Prices the cart from the **catalogue**, never from the browser.
 *
 * The client sends product ids and quantities and nothing else; there is no
 * price field on the wire to tamper with. The unit price is copied onto the
 * order line so a later catalogue change cannot restate what was agreed.
 */
export async function priceCart(
  store: ShopStore,
  lines: readonly CartLine[],
): Promise<{ items: OrderLine[]; totalMinor: number; currency: string }> {
  const normalised = normaliseLines(lines);
  const products = await store.findProducts(normalised.map((l) => l.productId));
  const byId = new Map(products.map((product) => [product.id, product]));

  const items: OrderLine[] = [];
  let totalMinor = 0;
  let currency: string | undefined;

  for (const line of normalised) {
    const product = byId.get(line.productId);
    if (product === undefined) {
      throw new TRPCError({
        code: "BAD_REQUEST",
        message: `no such product: ${line.productId}`,
      });
    }
    if (currency === undefined) {
      currency = product.currency;
    } else if (currency !== product.currency) {
      // Refused rather than summed. One PaymentIntent carries one currency,
      // and adding 7500 XAF to 5000 EUR would be a number with no meaning.
      throw new TRPCError({
        code: "BAD_REQUEST",
        message: "an order may not mix currencies",
      });
    }
    totalMinor += product.priceMinor * line.quantity;
    items.push({
      productId: product.id,
      name: product.name,
      quantity: line.quantity,
      unitMinor: product.priceMinor,
    });
  }

  if (currency === undefined || totalMinor <= 0) {
    throw new TRPCError({
      code: "BAD_REQUEST",
      message: "the cart totals nothing",
    });
  }
  return { items, totalMinor, currency };
}

/** The whole of `orders.create`. */
export async function placeOrder(
  deps: OrderDeps,
  input: PlaceOrderInput,
): Promise<PlaceOrderResult> {
  const priced = await priceCart(deps.store, input.lines);

  const order = await deps.store.createOrder({
    email: input.email,
    currency: priced.currency,
    totalMinor: priced.totalMinor,
    items: priced.items,
  });
  const keys = idempotencyKeys(order.id);

  const intent = await deps.vpay.paymentIntents.create(
    {
      amount: priced.totalMinor,
      currency: priced.currency,
      payment_method_types: deps.paymentMethodTypes,
      description: `vpay shop order ${order.id}`,
      metadata: { shop_order_id: order.id },
    },
    { idempotencyKey: keys.paymentIntent },
  );

  // Persisted before a session exists, and before anything could send a payer
  // anywhere. See this module's header.
  await deps.store.setPaymentIntentId(order.id, intent.id);

  if (input.mode === "embedded") {
    return { orderId: order.id, url: null };
  }

  const session = await deps.vpay.checkout.sessions.create(
    {
      payment_intent: intent.id,
      ui_mode: "hosted",
      success_url: returnUrl(deps.shopPublicUrl, order.id),
      cancel_url: cancelUrl(deps.shopPublicUrl, order.id),
    },
    { idempotencyKey: keys.hostedSession },
  );
  await deps.store.setCheckoutSessionId(order.id, session.id);

  if (typeof session.url !== "string" || session.url.length === 0) {
    // A hosted session with no `url` is not something to paper over with a
    // redirect to somewhere plausible.
    throw new TRPCError({
      code: "INTERNAL_SERVER_ERROR",
      message: `vpay returned a hosted session with no url for ${session.id}`,
    });
  }
  return { orderId: order.id, url: session.url };
}

/**
 * `orders.embeddedSecret` — the `fetchClientSecret` of
 * `initEmbeddedCheckout`, on the server side.
 *
 * Idempotent by construction: once a session exists for the order it is
 * *retrieved*, not recreated, because D1 allows one open session per
 * PaymentIntent and `initEmbeddedCheckout` may legitimately ask twice (a
 * remount, a reload).
 */
export async function embeddedClientSecret(
  deps: OrderDeps,
  orderId: string,
): Promise<{ clientSecret: string; sessionId: string }> {
  const order = await requireOrder(deps.store, orderId);
  if (order.status !== "unpaid") {
    throw new TRPCError({
      code: "CONFLICT",
      message: `order ${order.id} is ${order.status}; there is nothing to pay`,
    });
  }
  if (order.paymentIntentId === null) {
    throw new TRPCError({
      code: "CONFLICT",
      message: `order ${order.id} has no payment intent yet`,
    });
  }

  if (order.checkoutSessionId !== null) {
    const existing = await deps.vpay.checkout.sessions.retrieve(
      order.checkoutSessionId,
    );
    if (existing.ui_mode !== "embedded") {
      throw new TRPCError({
        code: "CONFLICT",
        message:
          `order ${order.id} already has a ${existing.ui_mode} checkout session; ` +
          `vpay allows one open session per payment intent`,
      });
    }
    if (existing.status !== "open") {
      // A session that has expired (24 h, D10) still answers `retrieve` and
      // still carries a `client_secret`. Handing that back would put a dead
      // credential into an iframe and leave the payer looking at a page that
      // cannot be paid, with nothing saying why.
      throw new TRPCError({
        code: "CONFLICT",
        message:
          `order ${order.id}'s checkout session ${existing.id} is ${existing.status}, ` +
          `not open`,
      });
    }
    return {
      clientSecret: requireSecret(existing.client_secret, existing.id),
      sessionId: existing.id,
    };
  }

  const session = await deps.vpay.checkout.sessions.create(
    {
      payment_intent: order.paymentIntentId,
      ui_mode: "embedded",
      return_url: returnUrl(deps.shopPublicUrl, order.id),
    },
    { idempotencyKey: idempotencyKeys(order.id).embeddedSession },
  );
  await deps.store.setCheckoutSessionId(order.id, session.id);
  return {
    clientSecret: requireSecret(session.client_secret, session.id),
    sessionId: session.id,
  };
}

function requireSecret(secret: string | undefined, sessionId: string): string {
  if (typeof secret !== "string" || secret.length === 0) {
    throw new TRPCError({
      code: "INTERNAL_SERVER_ERROR",
      message: `vpay returned session ${sessionId} without a client_secret`,
    });
  }
  return secret;
}

/** `orders.get`, and the lookup every page shares. */
export async function requireOrder(
  store: ShopStore,
  orderId: string,
): Promise<Order> {
  const order = await store.getOrder(orderId);
  if (order === null) {
    throw new TRPCError({ code: "NOT_FOUND", message: "no such order" });
  }
  return order;
}
