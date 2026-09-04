"use client";

import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { loadStripe } from "@vpay/stripe-js";
import type { EmbeddedCheckout } from "@vpay/stripe-js";
import { trpc } from "@/lib/trpc";

/**
 * vpay's checkout page, framed on the shop's own page.
 *
 * The publishable key and the checkout origin arrive as **props from a
 * server component**, not from `NEXT_PUBLIC_*`: they are then runtime
 * configuration of the container rather than values baked into the bundle at
 * build time, which is what lets one image serve the demo stack and a real
 * deployment.
 *
 * `fetchClientSecret` calls the shop's own `orders.embeddedSecret`, which is
 * where the merchant credential lives. The browser never sees it.
 *
 * `onComplete` is a message from an iframe, so it is treated as a **cue,
 * not evidence**: it navigates to the return page, which polls the shop's
 * database, which is only written by the verified webhook.
 */
export function EmbeddedCheckoutPanel({
  orderId,
  publishableKey,
  checkoutBaseUrl,
  apiBaseUrl,
}: {
  orderId: string;
  publishableKey: string;
  checkoutBaseUrl: string;
  apiBaseUrl: string;
}) {
  const router = useRouter();
  const [error, setError] = useState<string | null>(null);
  const handleRef = useRef<EmbeddedCheckout | null>(null);

  useEffect(() => {
    let live = true;
    async function start(): Promise<void> {
      const stripe = await loadStripe(publishableKey, {
        baseUrl: apiBaseUrl,
        checkoutBaseUrl,
      });
      if (stripe === null) {
        throw new Error("the vpay browser SDK failed to load");
      }
      const checkout = await stripe.initEmbeddedCheckout({
        fetchClientSecret: async () => {
          const result = await trpc.orders.embeddedSecret.mutate({ orderId });
          return result.clientSecret;
        },
        onComplete: () => {
          router.push(`/orders/${orderId}/return`);
        },
      });
      if (!live) {
        checkout.destroy();
        return;
      }
      handleRef.current = checkout;
      checkout.mount("#vpay-embedded-checkout");
    }

    start().catch((cause: unknown) => {
      if (live) {
        setError(
          cause instanceof Error
            ? cause.message
            : "the embedded checkout could not be started",
        );
      }
    });

    return () => {
      live = false;
      handleRef.current?.destroy();
      handleRef.current = null;
    };
  }, [orderId, publishableKey, checkoutBaseUrl, apiBaseUrl, router]);

  return (
    <>
      {error !== null ? (
        <p className="error" role="alert" data-testid="embedded-error">
          {error}
        </p>
      ) : null}
      <div id="vpay-embedded-checkout" data-testid="embedded-mount" />
    </>
  );
}
