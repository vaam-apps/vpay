import { formatMinor } from "@/money";
import { failureCopy } from "@/lib/failures";
import type { OrderView } from "@/lib/order-view";

const LABEL: Readonly<Record<OrderView["status"], string>> = {
  unpaid: "Unpaid",
  paid: "Paid",
  failed: "Failed",
  cancelled: "Cancelled",
};

/** The status badge. One place decides the word and the colour for a status. */
export function OrderStatusBadge({ status }: { status: OrderView["status"] }) {
  return (
    <span className={`status status-${status}`} data-testid="order-status">
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
    <section className="error" data-testid="order-failure">
      <h2 data-testid="order-failure-title">{copy.title}</h2>
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
      <p>
        <OrderStatusBadge status={order.status} />
      </p>
      <table>
        <thead>
          <tr>
            <th>Item</th>
            <th className="num">Unit</th>
            <th className="num">Qty</th>
            <th className="num">Line</th>
          </tr>
        </thead>
        <tbody>
          {order.items.map((item) => (
            <tr key={item.productId}>
              <td>{item.name}</td>
              <td className="num">
                {formatMinor(item.unitMinor, order.currency)}
              </td>
              <td className="num">{item.quantity}</td>
              <td className="num">
                {formatMinor(item.unitMinor * item.quantity, order.currency)}
              </td>
            </tr>
          ))}
          <tr>
            <td colSpan={3}>
              <strong>Total</strong>
            </td>
            <td className="num" data-testid="order-total">
              <strong>{formatMinor(order.totalMinor, order.currency)}</strong>
            </td>
          </tr>
        </tbody>
      </table>
      <h2>For the runbook</h2>
      <dl className="facts">
        <dt>Order</dt>
        <dd data-testid="order-id">{order.id}</dd>
        <dt>E-mail</dt>
        <dd data-testid="order-email">
          {order.email ?? "not given — optional, see the checkout page"}
        </dd>
        <dt>PaymentIntent</dt>
        <dd data-testid="order-payment-intent">
          {order.paymentIntentId ?? "—"}
        </dd>
        <dt>Checkout Session</dt>
        <dd data-testid="order-checkout-session">
          {order.checkoutSessionId ?? "—"}
        </dd>
        <dt>Failure code</dt>
        <dd data-testid="order-failure-code">{order.failureCode ?? "—"}</dd>
        <dt>What the rail said</dt>
        <dd data-testid="order-failure-message">
          {order.failureMessage ?? "—"}
        </dd>
      </dl>
      <p style={{ color: "var(--muted)", fontSize: "0.9rem" }}>
        Those two ids are identifiers, not credentials. The credentials they
        belong to (<code>pi_…_secret_…</code> and <code>cs_…_secret_…</code>)
        never leave the shop's server.
      </p>
    </>
  );
}
