/**
 * An in-memory {@link ShopStore} for this package's tests.
 *
 * **Test-only.** Nothing under `src/app` or `src/server` may import this
 * file, and `no-runtime-imports.test.ts` beside it fails the suite if
 * anything does — the TypeScript counterpart of `cargo xtask verify-no-mocks`
 * (AGENTS.md, rule 1). It is not reachable from `next build`'s module graph
 * because nothing in that graph names it.
 *
 * Order ids are `ord_1`, `ord_2`, … rather than cuids, so a test can assert
 * the exact `Idempotency-Key` the SDK put on the wire instead of matching it
 * against a pattern.
 */
import { decideWebhook } from "../server/store/webhook-decision";
import {
  PaymentIntentConflictError,
  type NewOrder,
  type Order,
  type Product,
  type ShopStore,
  type WebhookApplication,
  type WebhookOutcome,
} from "../server/store/types";

export class MemoryShopStore implements ShopStore {
  readonly #products = new Map<string, Product>();
  readonly #orders = new Map<string, Order>();
  readonly #events = new Map<string, { type: string; orderId: string }>();
  #nextOrder = 1;

  constructor(products: readonly Product[] = []) {
    for (const product of products) {
      this.#products.set(product.id, { ...product });
    }
  }

  /** Every recorded delivery, for a test that asserts a replay wrote nothing. */
  get recordedEvents(): ReadonlyMap<string, { type: string; orderId: string }> {
    return this.#events;
  }

  /** Every order, for a test that asserts nothing was written. */
  get orders(): ReadonlyMap<string, Order> {
    return this.#orders;
  }

  // implements `ShopStore`'s async contract; `ZenStackShopStore` awaits, this
  // in-memory double has nothing to await.
  // eslint-disable-next-line @typescript-eslint/require-await
  async listProducts(): Promise<Product[]> {
    return [...this.#products.values()].sort(
      (a, b) => a.priceMinor - b.priceMinor || a.id.localeCompare(b.id),
    );
  }

  // implements `ShopStore`'s async contract; `ZenStackShopStore` awaits, this
  // in-memory double has nothing to await.
  // eslint-disable-next-line @typescript-eslint/require-await
  async findProducts(ids: readonly string[]): Promise<Product[]> {
    const found: Product[] = [];
    for (const id of ids) {
      const product = this.#products.get(id);
      if (product !== undefined) {
        found.push({ ...product });
      }
    }
    return found;
  }

  // implements `ShopStore`'s async contract; `ZenStackShopStore` awaits, this
  // in-memory double has nothing to await.
  // eslint-disable-next-line @typescript-eslint/require-await
  async createOrder(input: NewOrder): Promise<Order> {
    const id = `ord_${this.#nextOrder}`;
    this.#nextOrder += 1;
    const order: Order = {
      id,
      email: input.email,
      status: "unpaid",
      totalMinor: input.totalMinor,
      currency: input.currency,
      paymentIntentId: null,
      checkoutSessionId: null,
      failureCode: null,
      failureMessage: null,
      items: input.items.map((item) => ({ ...item })),
      createdAt: new Date(0),
    };
    this.#orders.set(id, order);
    return { ...order, items: order.items.map((item) => ({ ...item })) };
  }

  // implements `ShopStore`'s async contract; `ZenStackShopStore` awaits, this
  // in-memory double has nothing to await.
  // eslint-disable-next-line @typescript-eslint/require-await
  async getOrder(id: string): Promise<Order | null> {
    const order = this.#orders.get(id);
    return order === undefined
      ? null
      : { ...order, items: order.items.map((item) => ({ ...item })) };
  }

  // implements `ShopStore`'s async contract; `ZenStackShopStore` awaits, this
  // in-memory double has nothing to await.
  // eslint-disable-next-line @typescript-eslint/require-await
  async setPaymentIntentId(
    orderId: string,
    paymentIntentId: string,
  ): Promise<Order> {
    const order = this.#mustGet(orderId);
    if (
      order.paymentIntentId !== null &&
      order.paymentIntentId !== paymentIntentId
    ) {
      throw new PaymentIntentConflictError(orderId);
    }
    for (const other of this.#orders.values()) {
      if (other.id !== orderId && other.paymentIntentId === paymentIntentId) {
        // The `@unique` on `payment_intent_id`, enforced here too so a test
        // that would violate it in Postgres fails here as well.
        throw new PaymentIntentConflictError(other.id);
      }
    }
    order.paymentIntentId = paymentIntentId;
    return { ...order, items: order.items.map((item) => ({ ...item })) };
  }

  // implements `ShopStore`'s async contract; `ZenStackShopStore` awaits, this
  // in-memory double has nothing to await.
  // eslint-disable-next-line @typescript-eslint/require-await
  async setCheckoutSessionId(
    orderId: string,
    checkoutSessionId: string,
  ): Promise<Order> {
    const order = this.#mustGet(orderId);
    order.checkoutSessionId = checkoutSessionId;
    return { ...order, items: order.items.map((item) => ({ ...item })) };
  }

  // implements `ShopStore`'s async contract; `ZenStackShopStore` awaits, this
  // in-memory double has nothing to await.
  // eslint-disable-next-line @typescript-eslint/require-await
  async applyWebhookEvent(
    application: WebhookApplication,
  ): Promise<WebhookOutcome> {
    const order = [...this.#orders.values()].find(
      (candidate) => candidate.paymentIntentId === application.paymentIntentId,
    );
    const decision = decideWebhook({
      alreadySeen: this.#events.has(application.eventId),
      orderStatus: order === undefined ? null : order.status,
      nextStatus: application.nextStatus,
    });
    if (decision.recordEvent && order !== undefined) {
      this.#events.set(application.eventId, {
        type: application.type,
        orderId: order.id,
      });
    }
    if (decision.writeStatus !== null && order !== undefined) {
      order.status = decision.writeStatus;
      // Together with the status, never on their own — the same statement
      // `ZenStackShopStore` writes them in.
      order.failureCode = application.failureCode;
      order.failureMessage = application.failureMessage;
    }
    return decision.outcome;
  }

  #mustGet(orderId: string): Order {
    const order = this.#orders.get(orderId);
    if (order === undefined) {
      throw new Error(`memory store: no order ${orderId}`);
    }
    return order;
  }
}
