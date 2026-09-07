import { formatMinor } from "@/money";
import { failureCopy } from "@/lib/failures";
import type { OrderView } from "@/lib/order-view";

const LABEL: Readonly<Record<OrderView["status"], string>> = {
  unpaid: "Unpaid",
  paid: "Paid",
  failed: "Failed",
  cancelled: "Cancelled",
};

/**
 * The badge tone per order status. `failed` and `cancelled` share `error`,
 * exactly as the pre-daisyUI CSS this replaces did (`.status-failed,
 * .status-cancelled { color: var(--bad) }`) — the two are equally "this did
 * not settle" from a buyer's point of view.
 */
const TONE: Readonly<Record<OrderView["status"], string>> = {
  unpaid: "warning",
  paid: "success",
  failed: "error",
  cancelled: "error",
};

/** The status badge. One place decides the word and the colour for a status. */
export function OrderStatusBadge({ status }: { status: OrderView["status"] }) {
  return (
    <span className={`badge badge-${TONE[status]}`} data-testid="order-status">
      {LABEL[status]}
    </span>
  );
}

/**
 * What the shop tells a buyer about a payment that did not work.
 *
 * The words come from `src/lib/failures.ts`, keyed on vpay's own closed
 * `FailureCode` vocabulary — which is the entire point of that vocabulary
 * existing: a merchant writes one message per outcome rather than one per
 * rail, and a rail added tomorrow reuses them. What is deliberately **not**
 * shown here is the rail's own sentence; that is operator text and lives
 * under "for the runbook" below.
 */
export function OrderFailureNotice({ order }: { order: OrderView }) {
  if (order.status !== "failed") {
    return null;
  }
  const copy = failureCopy(order.failureCode);
  return (
    <section
      role="alert"
      className="alert alert-error mt-4 flex-col items-start"
      data-testid="order-failure"
    >
      <h2 className="text-lg font-semibold" data-testid="order-failure-title">
        {copy.title}
      </h2>
      <p data-testid="order-failure-detail">{copy.detail}</p>
    </section>
  );
}

/**
 * Everything the shop knows about an order — all of it read from the shop's
 * own database. Nothing on this component reaches vpay.
 */
export function OrderSummary({ order }: { order: OrderView }) {
  return (
    <>
      <p className="mb-4">
        <OrderStatusBadge status={order.status} />
      </p>
      <div className="overflow-x-auto">
        <table className="table">
          <thead>
            <tr>
              <th>Item</th>
              <th className="text-right">Unit</th>
              <th className="text-right">Qty</th>
              <th className="text-right">Line</th>
            </tr>
          </thead>
          <tbody>
            {order.items.map((item) => (
              <tr key={item.productId}>
                <td>{item.name}</td>
                <td className="text-right tabular-nums">
                  {formatMinor(item.unitMinor, order.currency)}
                </td>
                <td className="text-right tabular-nums">{item.quantity}</td>
                <td className="text-right tabular-nums">
                  {formatMinor(item.unitMinor * item.quantity, order.currency)}
                </td>
              </tr>
            ))}
            <tr>
              <td colSpan={3}>
                <strong>Total</strong>
              </td>
              <td className="text-right tabular-nums" data-testid="order-total">
                <strong>{formatMinor(order.totalMinor, order.currency)}</strong>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <h2 className="mt-6 mb-2 text-lg font-semibold">For the runbook</h2>
      <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-base-content/60">Order</dt>
        <dd className="font-mono break-all" data-testid="order-id">
          {order.id}
        </dd>
        <dt className="text-base-content/60">E-mail</dt>
        <dd className="font-mono break-all" data-testid="order-email">
          {order.email ?? "not given — optional, see the checkout page"}
        </dd>
        <dt className="text-base-content/60">PaymentIntent</dt>
        <dd className="font-mono break-all" data-testid="order-payment-intent">
          {order.paymentIntentId ?? "—"}
        </dd>
        <dt className="text-base-content/60">Checkout Session</dt>
        <dd
          className="font-mono break-all"
          data-testid="order-checkout-session"
        >
          {order.checkoutSessionId ?? "—"}
        </dd>
        <dt className="text-base-content/60">Failure code</dt>
        <dd className="font-mono break-all" data-testid="order-failure-code">
          {order.failureCode ?? "—"}
        </dd>
        <dt className="text-base-content/60">What the rail said</dt>
        <dd className="font-mono break-all" data-testid="order-failure-message">
          {order.failureMessage ?? "—"}
        </dd>
      </dl>
      <p className="mt-4 text-sm text-base-content/60">
        Those two ids are identifiers, not credentials. The credentials they
        belong to (<code>pi_…_secret_…</code> and <code>cs_…_secret_…</code>)
        never leave the shop's server.
      </p>
    </>
  );
}
