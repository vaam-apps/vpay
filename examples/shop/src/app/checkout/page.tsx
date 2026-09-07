import { CartTable } from "@/components/cart-table";
import { CheckoutForm } from "@/components/checkout-form";
import { TestNumbersPanel } from "@/components/test-numbers-panel";
import { allSelectedRails, shopConfig } from "@/server/config";

export const dynamic = "force-dynamic";

/**
 * The checkout.
 *
 * Everything the browser needs is read here, on the server, and passed down
 * as props: the publishable key and the two origins are runtime
 * configuration of the container rather than values baked into the bundle by
 * `NEXT_PUBLIC_*` at build time, which is what lets one image serve the demo
 * stack and a real deployment. `VPAY_PRIVATE_KEY_FILE` and
 * `VPAY_WEBHOOK_SECRET` are read by the same module and stay on this side.
 */
export default function CheckoutPage() {
  const config = shopConfig();
  return (
    <>
      <h1 className="mb-4 text-2xl font-bold">Checkout</h1>
      <CartTable showCheckoutLink={false} />
      <h2 className="mt-8 mb-2 text-lg font-semibold">
        Where to send the receipt
      </h2>
      <CheckoutForm
        defaultMode={config.checkoutMode}
        publishableKey={config.vpayPublishableKey}
        checkoutBaseUrl={config.vpayCheckoutUrl}
        apiBaseUrl={config.vpayBrowserApiUrl}
      />
      <TestNumbersPanel rails={allSelectedRails(config.rails)} />
    </>
  );
}
