import { fetchRequestHandler } from "@trpc/server/adapters/fetch";
import { shopContext } from "@/server/context";
import { appRouter } from "@/server/routers";

export const dynamic = "force-dynamic";

function handler(request: Request): Promise<Response> {
  return fetchRequestHandler({
    endpoint: "/api/trpc",
    req: request,
    router: appRouter,
    createContext: () => shopContext(),
  });
}

export { handler as GET, handler as POST };
