import { formatMinor } from "@/money";
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
        <dd>{order.email}</dd>
        <dt>PaymentIntent</dt>
        <dd data-testid="order-payment-intent">
          {order.paymentIntentId ?? "—"}
        </dd>
        <dt>Checkout Session</dt>
        <dd data-testid="order-checkout-session">
          {order.checkoutSessionId ?? "—"}
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
