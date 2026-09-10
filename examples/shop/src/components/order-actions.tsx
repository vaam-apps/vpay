"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { trpc } from "@/lib/trpc";
import type { OrderView } from "@/lib/order-view";

/**
 * The two things a buyer can do about an order that did not settle.
 *
 * **Retry** places a *new* order with the same lines. It has to: vpay allows
 * one charge per intent forever, enforced by a unique index, so a second
 * attempt is a second intent by construction. The button says so.
 *
 * **Cancel** asks vpay to cancel the PaymentIntent and then waits. It writes
 * nothing here — the `cancelled` status arrives as a signed
 * `payment_intent.canceled` event, like every other settled status on this
 * page. Which is why the button's message says "asked", not "done": the
 * order moves when the webhook lands, not when this request returns.
 *
 * **It used to stay "asked" for ever, and this comment and the note below
 * both said so.** That was measured on the demo stack on 2026-09-06 and was
 * true then: the cancel moved the intent to `canceled` and vpay emitted no
 * event for that transition. vpay emits one from 2026-09-10
 * (vpay issue #57), in the cancel's own transaction, so the order really
 * does reach `cancelled` — and not one line of this component had to change
 * for it to, which is the argument the button exists to make.
 * `src/server/orders.ts` carries the full note.
 */
export function OrderActions({
  order,
  retryable,
}: {
  order: OrderView;
  retryable: boolean;
}) {
  const router = useRouter();
  const [busy, setBusy] = useState<"retry" | "cancel" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  async function retry(): Promise<void> {
    setError(null);
    setNote(null);
    setBusy("retry");
    try {
      const result = await trpc.orders.retry.mutate({
        orderId: order.id,
        mode: "hosted",
      });
      if (result.url === null) {
        setError("vpay did not return a checkout URL for the new order.");
        setBusy(null);
        return;
      }
      window.location.assign(result.url);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "The retry was refused.",
      );
      setBusy(null);
    }
  }

  async function cancel(): Promise<void> {
    setError(null);
    setNote(null);
    setBusy("cancel");
    try {
      await trpc.orders.cancel.mutate({ orderId: order.id });
      setNote(
        "vpay has been asked to cancel the payment, and its PaymentIntent is now cancelled at vpay. " +
          "This order is still unpaid until the signed payment_intent.canceled event arrives — this shop " +
          "moves an order only from a signed event, never from its own request. Refresh in a moment.",
      );
      router.refresh();
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "The cancellation was refused.",
      );
    }
    setBusy(null);
  }

  const canCancel = order.status === "unpaid" && order.paymentIntentId !== null;
  if (!retryable && !canCancel) {
    return null;
  }

  return (
    <>
      {error !== null ? (
        <p
          className="mt-4 alert alert-error"
          role="alert"
          data-testid="order-action-error"
        >
          {error}
        </p>
      ) : null}
      {note !== null ? (
        <p
          className="mt-4 alert alert-info"
          role="status"
          data-testid="order-action-note"
        >
          {note}
        </p>
      ) : null}
      <p className="mt-4 flex flex-wrap gap-3">
        {retryable ? (
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy !== null}
            data-testid="order-retry"
            onClick={() => void retry()}
          >
            {busy === "retry" ? "Placing a new order…" : "Try again"}
          </button>
        ) : null}
        {canCancel ? (
          <button
            type="button"
            className="btn btn-outline"
            disabled={busy !== null}
            data-testid="order-cancel"
            onClick={() => void cancel()}
          >
            {busy === "cancel" ? "Asking vpay…" : "Cancel this payment"}
          </button>
        ) : null}
      </p>
      {retryable ? (
        <p className="mt-2 text-sm text-base-content/60">
          &ldquo;Try again&rdquo; places a <strong>new order</strong> with the
          same items, at today&rsquo;s catalogue prices. It has to: vpay allows
          one charge per PaymentIntent forever, so a retry is a new intent by
          construction. This order stays exactly as it is.
        </p>
      ) : null}
    </>
  );
}
