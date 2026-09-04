import { notFound } from "next/navigation";
import { TRPCError } from "@trpc/server";
import { OrderPoller } from "@/components/order-poller";
import { serverCaller } from "@/server/context";

export const dynamic = "force-dynamic";

/**
 * `success_url` — where vpay sends a payer who finished on its page.
 *
 * `session_id` arrives in the query string because the shop asked for it
 * (`?session_id={CHECKOUT_SESSION_ID}`, D5). It is **displayed and nothing
 * else**: this page does not read it, does not look it up at vpay, and above
 * all does not use it to mark anything paid. Arriving here means a browser
 * followed a redirect; it does not mean money moved.
 */
export default async function OrderReturnPage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const { id } = await params;
  const query = await searchParams;
  const sessionId = query["session_id"];

  let order;
  try {
    order = await serverCaller().orders.get({ id });
  } catch (err) {
    if (err instanceof TRPCError && err.code === "NOT_FOUND") {
      notFound();
    }
    throw err;
  }

  return (
    <>
      <h1>Thank you</h1>
      <OrderPoller initial={order} />
      <p style={{ color: "var(--muted)", fontSize: "0.9rem" }}>
        vpay sent you back with{" "}
        <code data-testid="return-session-id">
          {typeof sessionId === "string" ? sessionId : "no session id"}
        </code>
        . The shop shows it for the runbook and takes no decision from it.
      </p>
    </>
  );
}
