/**
 * tRPC's initialisation, and the context every procedure receives.
 *
 * The context carries the store, the vpay client and the two config values
 * the procedures need — never the whole {@link ShopConfig}, which holds the
 * webhook secret and the path to the signing key. A procedure that cannot
 * reach a secret cannot leak one.
 */
import { initTRPC } from "@trpc/server";
import type { VpayClient } from "@vaam-apps/vpay-sdk";
import type { CheckoutMode, RailSelection } from "./config";
import type { ShopStore } from "./store/types";

export interface ShopContext {
  store: ShopStore;
  vpay: VpayClient;
  shopPublicUrl: string;
  rails: RailSelection;
  /** The surface the checkout page opens on. Configuration, not a code path. */
  checkoutMode: CheckoutMode;
}

const t = initTRPC.context<ShopContext>().create();

export const router = t.router;
export const publicProcedure = t.procedure;
