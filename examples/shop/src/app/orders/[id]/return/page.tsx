import { notFound } from "next/navigation";
import { TRPCError } from "@trpc/server";
import { OrderPoller } from "@/components/order-poller";
import { PopupReturnNotifier } from "@/components/popup-return-notifier";
import { serverCaller } from "@/server/context";

export const dynamic = "force-dynamic";

/**
 * `success_url` — where vpay sends a payer who finished on its page.
 *
 * `session_id` arrives in the query string because the shop asked for it
 * (`?session_id={CHECKOUT_SESSION_ID}`, D5). It is **displayed, and passed to
 * the popup notifier as a label** — this page does not look it up at vpay,
 * and above all does not use it to mark anything paid. Arriving here means a
 * browser followed a redirect; it does not mean money moved.
 *
 * The same page serves all three surfaces. In the popup integration it is
 * loaded *inside the popup*, where {@link PopupReturnNotifier} tells the
 * opener and closes the window; on every other path that component finds no
 * opener and does nothing.
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
      <PopupReturnNotifier
        sessionId={typeof sessionId === "string" ? sessionId : null}
        status="complete"
      />
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
