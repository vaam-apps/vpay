import Link from "next/link";
import { notFound } from "next/navigation";
import { TRPCError } from "@trpc/server";
import { EmbeddedCheckoutPanel } from "@/components/embedded-checkout";
import { OrderStatusBadge } from "@/components/order-summary";
import { TestNumbersPanel } from "@/components/test-numbers-panel";
import { formatMinor } from "@/money";
import { allSelectedRails, shopConfig } from "@/server/config";
import { serverCaller } from "@/server/context";

export const dynamic = "force-dynamic";

/**
 * The same unpaid order, paid without leaving the shop.
 *
 * The two values the browser needs — the publishable key and the checkout
 * app's origin — are read from the server's environment here and passed down
 * as props. `VPAY_PRIVATE_KEY_FILE` and `VPAY_WEBHOOK_SECRET` are read by the
 * same module and stay on this side of the boundary.
 */
export default async function OrderEmbeddedPage({
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

  if (order.status !== "unpaid") {
    return (
      <>
        <h1>Nothing to pay</h1>
        <p>
          Order <code>{order.id}</code> is already settled.
        </p>
        <p>
          <OrderStatusBadge status={order.status} />
        </p>
        <p>
          <Link href={`/orders/${order.id}`}>The order page</Link>
        </p>
      </>
    );
  }

  const config = shopConfig();
  return (
    <>
      <h1>Pay {formatMinor(order.totalMinor, order.currency)}</h1>
      <p style={{ color: "var(--muted)" }}>
        The panel below is vpay's own checkout page, in an iframe served from{" "}
        <code>{config.vpayCheckoutUrl}</code>. It is allowed to frame here
        because this shop's origin is in the merchant's{" "}
        <code>checkout_origins</code>.
      </p>
      <EmbeddedCheckoutPanel
        orderId={order.id}
        publishableKey={config.vpayPublishableKey}
        checkoutBaseUrl={config.vpayCheckoutUrl}
        apiBaseUrl={config.vpayBrowserApiUrl}
      />
      <p style={{ marginTop: "1rem" }}>
        <Link href={`/orders/${order.id}`}>The order page</Link>
      </p>
      <TestNumbersPanel rails={allSelectedRails(config.rails)} />
    </>
  );
}
