import { publicProcedure, router } from "../trpc";

export const productsRouter = router({
  /** The whole catalogue. Five rows, seeded by a migration (D13). */
  list: publicProcedure.query(async ({ ctx }) => ctx.store.listProducts()),
});
