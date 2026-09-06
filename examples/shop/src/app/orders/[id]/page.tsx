import Link from "next/link";
import { notFound } from "next/navigation";
import { TRPCError } from "@trpc/server";
import { OrderActions } from "@/components/order-actions";
import { OrderFailureNotice, OrderSummary } from "@/components/order-summary";
import { failureCopy } from "@/lib/failures";
import { serverCaller } from "@/server/context";

export const dynamic = "force-dynamic";

export default async function OrderPage({
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
      <h1>Order {order.id}</h1>
      <p style={{ color: "var(--muted)" }}>
        This page reads the shop's database and nothing else. It says{" "}
        <em>paid</em> only once vpay's webhook has been received, verified and
        written — not because a payer came back through a redirect.
      </p>
      <OrderFailureNotice order={order} />
      <OrderSummary order={order} />
      {order.status === "unpaid" && order.paymentIntentId !== null ? (
        <p>
          <Link
            className="button secondary"
            href={`/orders/${order.id}/embedded`}
          >
            Pay this order without leaving the shop
          </Link>
        </p>
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
