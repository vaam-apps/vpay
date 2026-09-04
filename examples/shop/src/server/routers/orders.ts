import { z } from "zod";
import {
  MAX_LINES,
  MAX_QUANTITY_PER_LINE,
  embeddedClientSecret,
  placeOrder,
  requireOrder,
} from "../orders";
import type { OrderDeps } from "../orders";
import type { OrderView } from "../../lib/order-view";
import type { Order } from "../store/types";
import { publicProcedure, router } from "../trpc";
import type { ShopContext } from "../trpc";

export function toOrderView(order: Order): OrderView {
  return {
    id: order.id,
    email: order.email,
    status: order.status,
    totalMinor: order.totalMinor,
    currency: order.currency,
    paymentIntentId: order.paymentIntentId,
    checkoutSessionId: order.checkoutSessionId,
    createdAt: order.createdAt.getTime(),
    items: order.items,
  };
}

function deps(ctx: ShopContext): OrderDeps {
  return {
    store: ctx.store,
    vpay: ctx.vpay,
    shopPublicUrl: ctx.shopPublicUrl,
    paymentMethodTypes: ctx.paymentMethodTypes,
  };
}

const cartLine = z.object({
  productId: z.string().min(1).max(64),
  quantity: z.int().min(1).max(MAX_QUANTITY_PER_LINE),
});

export const ordersRouter = router({
  /**
   * Validates the cart against the catalogue, totals it server-side, creates
   * the PaymentIntent and — in hosted mode — the Checkout Session, and
   * answers with vpay's `url`.
   *
   * There is no price on the input schema. That is the whole defence: a
   * browser cannot send an amount, so it cannot send a wrong one.
   */
  create: publicProcedure
    .input(
      z.object({
        email: z.email().max(320),
        lines: z.array(cartLine).min(1).max(MAX_LINES),
        mode: z.enum(["hosted", "embedded"]).default("hosted"),
      }),
    )
    .mutation(async ({ ctx, input }) =>
      placeOrder(deps(ctx), {
        email: input.email,
        lines: input.lines,
        mode: input.mode,
      }),
    ),

  /**
   * The order **from this shop's database**, and nothing else. The return
   * page polls this; it never calls vpay, and it never learns anything vpay
   * has not already told the webhook endpoint.
   */
  get: publicProcedure
    .input(z.object({ id: z.string().min(1).max(64) }))
    .query(async ({ ctx, input }) =>
      toOrderView(await requireOrder(ctx.store, input.id)),
    ),

  /**
   * `fetchClientSecret` for `initEmbeddedCheckout`. Creates the embedded
   * session the first time and retrieves it thereafter.
   */
  embeddedSecret: publicProcedure
    .input(z.object({ orderId: z.string().min(1).max(64) }))
    .mutation(async ({ ctx, input }) =>
      embeddedClientSecret(deps(ctx), input.orderId),
    ),
});
