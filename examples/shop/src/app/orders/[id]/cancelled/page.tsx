import Link from "next/link";
import { notFound } from "next/navigation";
import { TRPCError } from "@trpc/server";
import { OrderActions } from "@/components/order-actions";
import { OrderStatusBadge } from "@/components/order-summary";
import { serverCaller } from "@/server/context";

export const dynamic = "force-dynamic";

/**
 * `cancel_url`.
 *
 * It writes nothing. A payer who clicked "cancel" on vpay's page has told the
 * *page* they gave up; the order stays `unpaid` until vpay says otherwise
 * through the webhook (a `payment_intent.canceled` event would make it
 * `cancelled`). A cancel URL is a navigation, not an authority — the same
 * reason the return page below cannot mark an order paid.
 */
export default async function OrderCancelledPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
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
      <h1>Payment cancelled</h1>
      <p data-testid="cancelled-message">
        You came back from vpay without paying. Nothing has been charged, and
        order <code>{order.id}</code> is still open.
      </p>
      <p>
        <OrderStatusBadge status={order.status} />
      </p>
      <p style={{ display: "flex", gap: "0.75rem", flexWrap: "wrap" }}>
        <Link className="button" href={`/orders/${order.id}/embedded`}>
          Try again without leaving the shop
        </Link>
        <Link className="button secondary" href="/">
          Back to the catalogue
        </Link>
      </p>
      <h2>Or close the order properly</h2>
      <p style={{ color: "var(--muted)" }}>
        Coming back here is a <em>navigation</em>, not a cancellation: this
        order is still <code>unpaid</code>, its PaymentIntent is still live at
        vpay, and a charge already submitted to a rail could still settle. The
        button below asks vpay to cancel the intent. Even then this shop writes
        nothing — the order becomes <code>cancelled</code> when the signed{" "}
        <code>payment_intent.canceled</code> event arrives.
      </p>
      <OrderActions order={order} retryable={false} />
    </>
  );
}
