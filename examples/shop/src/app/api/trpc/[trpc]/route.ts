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
    // A procedure that throws is answered to the browser as an error and, with
    // no hook here, reported nowhere on the server: a failed call to vpay left
    // the shop's own log empty and a payer on a page that never redirected.
    // The error's message and cause chain are the SDK's own and never carry a
    // key, a seed or a signed body.
    onError: ({ path, error }) => {
      // eslint-disable-next-line no-console -- the shop has no logger; stderr is its log
      console.error(
        `vpay shop: tRPC ${path ?? "<unknown>"} failed: ${error.name}: ${error.message}`,
        error.cause ?? "",
      );
    },
  });
}

export { handler as GET, handler as POST };
