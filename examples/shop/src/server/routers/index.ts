import { router } from "../trpc";
import { ordersRouter } from "./orders";
import { productsRouter } from "./products";

export const appRouter = router({
  products: productsRouter,
  orders: ordersRouter,
});

export type AppRouter = typeof appRouter;
