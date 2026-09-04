/**
 * tRPC's initialisation, and the context every procedure receives.
 *
 * The context carries the store, the vpay client and the two config values
 * the procedures need — never the whole {@link ShopConfig}, which holds the
 * webhook secret and the path to the signing key. A procedure that cannot
 * reach a secret cannot leak one.
 */
import { initTRPC } from "@trpc/server";
import type { VpayClient } from "@vpay/sdk";
import type { PaymentMethodType } from "./config";
import type { ShopStore } from "./store/types";

export interface ShopContext {
  store: ShopStore;
  vpay: VpayClient;
  shopPublicUrl: string;
  paymentMethodTypes: PaymentMethodType[];
}

const t = initTRPC.context<ShopContext>().create();

export const router = t.router;
export const publicProcedure = t.procedure;
