import { z } from "zod";
import {
  MAX_LINES,
  MAX_QUANTITY_PER_LINE,
  cancelOrder,
  embeddedClientSecret,
  placeOrder,
  requireOrder,
  retryOrder,
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
    failureCode: order.failureCode,
    failureMessage: order.failureMessage,
    createdAt: order.createdAt.getTime(),
    items: order.items,
  };
}

function deps(ctx: ShopContext): OrderDeps {
  return {
    store: ctx.store,
    vpay: ctx.vpay,
    shopPublicUrl: ctx.shopPublicUrl,
    rails: ctx.rails,
  };
}

const cartLine = z.object({
  productId: z.string().min(1).max(64),
  quantity: z.int().min(1).max(MAX_QUANTITY_PER_LINE),
});

/**
 * The three surfaces, on the wire.
 *
 * `popup` is accepted here even though it produces exactly the session
 * `hosted` does, because the *shop* wants to know which one a buyer chose —
 * and because a client that sent `hosted` and then opened a popup would be
 * relying on the two staying the same forever.
 */
const checkoutMode = z.enum(["hosted", "embedded", "popup"]);

/**
 * `null` and an absent value both mean "no e-mail", and an empty string does
 * too: an HTML form posts `""` for a field the buyer left blank, and
 * refusing that with "not a valid e-mail" would be the shop failing an
 * optional field for being unfilled.
 */
const optionalEmail = z
  .union([z.email().max(320), z.literal(""), z.null()])
  .optional()
  .transform((value) =>
    value === undefined || value === null || value === "" ? null : value,
  );

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
        email: optionalEmail,
        lines: z.array(cartLine).min(1).max(MAX_LINES),
        mode: checkoutMode.default("hosted"),
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

  /**
   * A **new** order carrying the failed one's lines. One charge per intent,
   * forever — so a retry is a new order, a new intent and a new session, and
   * the prices are re-read from the catalogue rather than copied.
   */
  retry: publicProcedure
    .input(
      z.object({
        orderId: z.string().min(1).max(64),
        mode: checkoutMode.default("hosted"),
      }),
    )
    .mutation(async ({ ctx, input }) =>
      retryOrder(deps(ctx), input.orderId, input.mode),
    ),

  /**
   * Cancels the order's PaymentIntent at vpay. Writes nothing: the
   * `cancelled` status arrives as a `payment_intent.canceled` event, like
   * every other settled status this shop shows.
   */
  cancel: publicProcedure
    .input(z.object({ orderId: z.string().min(1).max(64) }))
    .mutation(async ({ ctx, input }) => cancelOrder(deps(ctx), input.orderId)),
});
