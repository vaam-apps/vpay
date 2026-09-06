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
 * nothing here — the `cancelled` status would arrive as a signed
 * `payment_intent.canceled` event, like every other settled status on this
 * page. Which is why the button's message says "asked", not "done".
 *
 * **It stays "asked" for ever on today's vpay**, and the button says so
 * rather than leaving a buyer refreshing: the cancel really does move the
 * intent to `canceled`, and vpay emits no event for that transition (it
 * writes three types, and this is not one of them). Measured on the demo
 * stack, 2026-09-06. `src/server/orders.ts` carries the full note.
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
          "This order will nevertheless stay unpaid: vpay does not yet emit a payment_intent.canceled event, " +
          "and this shop moves an order only from a signed one. Measured 2026-09-06 — see examples/shop/README.md.",
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
        <p className="error" role="alert" data-testid="order-action-error">
          {error}
        </p>
      ) : null}
      {note !== null ? (
        <p role="status" data-testid="order-action-note">
          {note}
        </p>
      ) : null}
      <p style={{ display: "flex", gap: "0.75rem", flexWrap: "wrap" }}>
        {retryable ? (
          <button
            type="button"
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
            className="secondary"
            disabled={busy !== null}
            data-testid="order-cancel"
            onClick={() => void cancel()}
          >
            {busy === "cancel" ? "Asking vpay…" : "Cancel this payment"}
          </button>
        ) : null}
      </p>
      {retryable ? (
        <p style={{ color: "var(--muted)", fontSize: "0.9rem" }}>
          &ldquo;Try again&rdquo; places a <strong>new order</strong> with the
          same items, at today&rsquo;s catalogue prices. It has to: vpay allows
          one charge per PaymentIntent forever, so a retry is a new intent by
          construction. This order stays exactly as it is.
        </p>
      ) : null}
    </>
  );
}
