"use client";

/**
 * The browser's tRPC client.
 *
 * `createTRPCClient` from `@trpc/client`, not `@trpc/react-query`: the shop
 * has three call sites and one poll loop, and a query cache would be a
 * dependency and a provider for no behaviour. The types are the same ones —
 * `AppRouter` is imported as a **type**, so no server module reaches the
 * bundle.
 */
import { createTRPCClient, httpBatchLink } from "@trpc/client";
import type { AppRouter } from "@/server/routers";

export const trpc = createTRPCClient<AppRouter>({
  links: [httpBatchLink({ url: "/api/trpc" })],
});
