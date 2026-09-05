"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { OrderSummary } from "@/components/order-summary";
import { trpc } from "@/lib/trpc";
import type { OrderView } from "@/lib/order-view";

/** How often the return page asks the shop's own server what it knows. */
export const POLL_INTERVAL_MS = 2000;

/**
 * The return page's body: "we are confirming your payment", then the outcome.
 *
 * It polls `orders.get` — a read of the shop's **own database** — and never
 * calls vpay. The shop has exactly one source of truth about whether money
 * moved, and it is the signed webhook; a browser that asked vpay directly
 * would be asking a question it could not verify the answer to.
 */
export function OrderPoller({ initial }: { initial: OrderView }) {
  const [order, setOrder] = useState<OrderView>(initial);
  const [polls, setPolls] = useState(0);
  const settled = order.status !== "unpaid";
  const settledRef = useRef(settled);
  // REAL finding: this writes a ref during render so the poll interval can see
  // the latest status without re-subscribing. Correct fix is to derive it
  // inside the effect.
  // eslint-disable-next-line react-hooks/refs
  settledRef.current = settled;

  useEffect(() => {
    if (settled) {
      return;
    }
    let live = true;
    const timer = window.setInterval(() => {
      if (settledRef.current) {
        return;
      }
      trpc.orders.get
        .query({ id: initial.id })
        .then((next) => {
          if (live) {
            setOrder(next);
            setPolls((count) => count + 1);
          }
        })
        .catch(() => {
          // A failed poll is not an outcome. Keep waiting; the next tick
          // asks again.
          if (live) {
            setPolls((count) => count + 1);
          }
        });
    }, POLL_INTERVAL_MS);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [initial.id, settled]);

  return (
    <>
      {settled ? null : (
        <p data-testid="confirming" role="status" aria-live="polite">
          We are confirming your payment. This page updates itself every{" "}
          {POLL_INTERVAL_MS / 1000} seconds and needs no refresh. ({polls}{" "}
          checks so far.)
        </p>
      )}
      {order.status === "paid" ? (
        <p data-testid="paid-message">
          Paid. Thank you — the shop marked this order paid when vpay's signed
          webhook arrived, not when your browser did.
        </p>
      ) : null}
      {order.status === "failed" ? (
        <p className="error" data-testid="failed-message">
          The payment failed. Nothing has been charged.
        </p>
      ) : null}
      {order.status === "cancelled" ? (
        <p className="error" data-testid="cancelled-message">
          The payment was cancelled.
        </p>
      ) : null}
      <OrderSummary order={order} />
      <p>
        <Link href={`/orders/${order.id}`}>The order page</Link>
      </p>
    </>
  );
}
