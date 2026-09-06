/**
 * The shipping implementation of {@link ShopStore}, over the ZenStack client.
 *
 * It was `PrismaShopStore` over a `PrismaClient` until the ZenStack 3 upgrade
 * (2026-09-06). v3 replaced Prisma at runtime with its own Kysely-based ORM,
 * so the client this holds is a `ClientContract` with the policy plugin on it
 * and Prisma is no longer in the request path at all — it still applies the
 * migrations, through `zen migrate`, and nothing else. The **query surface is
 * deliberately unchanged**: v3's ORM is Prisma-API-compatible, which is why
 * every method below reads as it did.
 *
 * No unit or integration test covers it — see the note in `types.ts`. It was
 * verified by hand on 2026-09-04 against a real Postgres, which is the
 * README's own account of it, and again on 2026-09-06 after the v3 upgrade.
 * `just demo` places no order; lane 6's `shop-hosted.cy.ts` and
 * `shop-embedded.cy.ts` have merged and do drive this class in a real
 * browser, but they assert on the shop's pages, never on this class.
 */
import type { ShopClient } from "../db";
import { decideWebhook } from "./webhook-decision";
import {
  PaymentIntentConflictError,
  type NewOrder,
  type Order,
  type OrderStatus,
  type Product,
  type ShopStore,
  type WebhookApplication,
  type WebhookOutcome,
} from "./types";

interface OrderItemRow {
  productId: string;
  quantity: number;
  unitMinor: number;
  product: { name: string };
}

interface OrderRow {
  id: string;
  email: string | null;
  status: string;
  totalMinor: number;
  currency: string;
  paymentIntentId: string | null;
  checkoutSessionId: string | null;
  failureCode: string | null;
  failureMessage: string | null;
  createdAt: Date;
  items: OrderItemRow[];
}

const ORDER_INCLUDE = {
  items: {
    include: { product: { select: { name: true } } },
    orderBy: { id: "asc" as const },
  },
};

function toOrder(row: OrderRow): Order {
  return {
    id: row.id,
    email: row.email,
    status: row.status as OrderStatus,
    totalMinor: row.totalMinor,
    currency: row.currency,
    paymentIntentId: row.paymentIntentId,
    checkoutSessionId: row.checkoutSessionId,
    failureCode: row.failureCode,
    failureMessage: row.failureMessage,
    createdAt: row.createdAt,
    items: row.items.map((item) => ({
      productId: item.productId,
      name: item.product.name,
      quantity: item.quantity,
      unitMinor: item.unitMinor,
    })),
  };
}

export class ZenStackShopStore implements ShopStore {
  readonly #db: ShopClient;

  constructor(db: ShopClient) {
    this.#db = db;
  }

  async listProducts(): Promise<Product[]> {
    return this.#db.product.findMany({
      orderBy: [{ priceMinor: "asc" }, { id: "asc" }],
    });
  }

  async findProducts(ids: readonly string[]): Promise<Product[]> {
    if (ids.length === 0) {
      return [];
    }
    return this.#db.product.findMany({ where: { id: { in: [...ids] } } });
  }

  async createOrder(input: NewOrder): Promise<Order> {
    const row = (await this.#db.order.create({
      data: {
        email: input.email,
        currency: input.currency,
        totalMinor: input.totalMinor,
        items: {
          create: input.items.map((item) => ({
            productId: item.productId,
            quantity: item.quantity,
            unitMinor: item.unitMinor,
          })),
        },
      },
      include: ORDER_INCLUDE,
    })) as unknown as OrderRow;
    return toOrder(row);
  }

  async getOrder(id: string): Promise<Order | null> {
    const row = (await this.#db.order.findUnique({
      where: { id },
      include: ORDER_INCLUDE,
    })) as unknown as OrderRow | null;
    return row === null ? null : toOrder(row);
  }

  async setPaymentIntentId(
    orderId: string,
    paymentIntentId: string,
  ): Promise<Order> {
    const current = await this.getOrder(orderId);
    if (
      current !== null &&
      current.paymentIntentId !== null &&
      current.paymentIntentId !== paymentIntentId
    ) {
      throw new PaymentIntentConflictError(orderId);
    }
    const row = (await this.#db.order.update({
      where: { id: orderId },
      data: { paymentIntentId },
      include: ORDER_INCLUDE,
    })) as unknown as OrderRow;
    return toOrder(row);
  }

  async setCheckoutSessionId(
    orderId: string,
    checkoutSessionId: string,
  ): Promise<Order> {
    const row = (await this.#db.order.update({
      where: { id: orderId },
      data: { checkoutSessionId },
      include: ORDER_INCLUDE,
    })) as unknown as OrderRow;
    return toOrder(row);
  }

  async applyWebhookEvent(
    application: WebhookApplication,
  ): Promise<WebhookOutcome> {
    // One interactive transaction, so the `webhook_events` insert and the
    // status write either both land or neither does. The route handler
    // answers 2xx only after this resolves.
    return this.#db.$transaction(async (tx) => {
      const seen = await tx.webhookEvent.findUnique({
        where: { id: application.eventId },
        select: { id: true },
      });
      const order = await tx.order.findUnique({
        where: { paymentIntentId: application.paymentIntentId },
        select: { id: true, status: true },
      });
      const decision = decideWebhook({
        alreadySeen: seen !== null,
        orderStatus: order === null ? null : order.status,
        nextStatus: application.nextStatus,
      });
      if (decision.recordEvent && order !== null) {
        await tx.webhookEvent.create({
          data: {
            id: application.eventId,
            type: application.type,
            orderId: order.id,
          },
        });
      }
      if (decision.writeStatus !== null && order !== null) {
        await tx.order.update({
          where: { id: order.id },
          // The failure columns are written in the same statement as the
          // status they explain, and only here — so there is no window in
          // which an order is `failed` with no code, and an order whose
          // status this delivery does not move cannot be stamped with one.
          data: {
            status: decision.writeStatus,
            failureCode: application.failureCode,
            failureMessage: application.failureMessage,
          },
        });
      }
      return decision.outcome;
    });
  }
}
