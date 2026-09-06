import Link from "next/link";
import { notFound } from "next/navigation";
import { TRPCError } from "@trpc/server";
import { OrderActions } from "@/components/order-actions";
import {
  OrderFailureNotice,
  OrderStatusBadge,
} from "@/components/order-summary";
import { failureCopy } from "@/lib/failures";
import { serverCaller } from "@/server/context";

export const dynamic = "force-dynamic";

/**
 * `cancel_url`.
 *
 * It writes nothing. A payer who clicked "cancel" on vpay's page has told the
 * *page* they gave up; the order stays `unpaid` until vpay says otherwise
 * through the webhook. A cancel URL is a navigation, not an authority — the
 * same reason the return page cannot mark an order paid.
 *
 * **It does not follow that the order is still open when a payer arrives
 * here**, and this page used to say it was, in prose, unconditionally. It was
 * wrong the first time it was driven end to end: vpay's checkout page sends a
 * payer to `cancel_url` after a **declined** charge too (its outcome screen's
 * "Continue"), and the shop's own webhook has usually already made the order
 * `failed` by the time the browser lands. The page then asserted "still
 * unpaid" beside a badge reading *Failed*.
 *
 * So it reads the order and branches on what it actually says. The copy that
 * only makes sense for an open order is the `unpaid` branch; a settled order
 * gets the same failure notice the order page shows.
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

  const open = order.status === "unpaid";

  return (
    <>
      <h1>{open ? "Payment cancelled" : "You came back without paying"}</h1>
      {open ? (
        <p data-testid="cancelled-message">
          You came back from vpay without paying. Nothing has been charged, and
          order <code>{order.id}</code> is still open.
        </p>
      ) : (
        <p data-testid="cancelled-message">
          vpay sent you back here, and by the time this page loaded its signed
          webhook had already settled order <code>{order.id}</code>. What that
          webhook said is below — this page took no decision of its own.
        </p>
      )}
      <p>
        <OrderStatusBadge status={order.status} />
      </p>
      <OrderFailureNotice order={order} />
      <p style={{ display: "flex", gap: "0.75rem", flexWrap: "wrap" }}>
        {open ? (
          <Link className="button" href={`/orders/${order.id}/embedded`}>
            Try again without leaving the shop
          </Link>
        ) : null}
        <Link className="button secondary" href={`/orders/${order.id}`}>
          The order page
        </Link>
        <Link className="button secondary" href="/">
          Back to the catalogue
        </Link>
      </p>
      {open ? (
        <>
          <h2>Or close the order properly</h2>
          <p style={{ color: "var(--muted)" }}>
            Coming back here is a <em>navigation</em>, not a cancellation: this
            order is still <code>unpaid</code>, its PaymentIntent is still live
            at vpay, and a charge already submitted to a rail could still
            settle. The button below asks vpay to cancel the intent. Even then
            this shop writes nothing — the order would become{" "}
            <code>cancelled</code> when the signed{" "}
            <code>payment_intent.canceled</code> event arrived.{" "}
            <strong>
              It will not: vpay emits no event for that transition.
            </strong>{" "}
            The cancel really does move the intent at vpay — measured on the
            demo stack, 2026-09-06 — and this order will nevertheless stay{" "}
            <code>unpaid</code>, because moving it from anything other than a
            signed event is the one thing this example exists to argue against.
            See <code>examples/shop/README.md</code>.
          </p>
        </>
      ) : null}
      <OrderActions
        order={order}
        retryable={
          order.status === "failed" && failureCopy(order.failureCode).retryable
        }
      />
    </>
  );
}
