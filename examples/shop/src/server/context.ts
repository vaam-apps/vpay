/**
 * Builds the tRPC context from this process's singletons, and a server-side
 * caller for React Server Components.
 *
 * Server components call the router through `serverCaller()` rather than over
 * HTTP: the same procedures, the same validation, one fewer round trip, and
 * — the reason that matters here — a page that renders an order's status is
 * reading the same code path the browser's poll reads.
 */
import { shopConfig } from "./config";
import { db } from "./db";
import { appRouter } from "./routers/index";
import { PrismaShopStore } from "./store/prisma-store";
import type { ShopContext } from "./trpc";
import { vpay } from "./vpay";

export function shopContext(): ShopContext {
  const config = shopConfig();
  return {
    store: new PrismaShopStore(db()),
    // A getter, so the merchant private key is read the first time a
    // procedure actually needs vpay. The catalogue and the order pages do
    // not, and a shop whose product list fell over because of a key file it
    // never uses would be reporting the wrong fault.
    get vpay() {
      return vpay();
    },
    shopPublicUrl: config.shopPublicUrl,
    rails: config.rails,
    checkoutMode: config.checkoutMode,
  };
}

export function serverCaller(): ReturnType<typeof appRouter.createCaller> {
  return appRouter.createCaller(shopContext());
}
