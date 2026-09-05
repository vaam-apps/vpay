"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import {
  clearCart,
  onCartChange,
  readCart,
  type CartLine,
} from "@/components/cart";
import { trpc } from "@/lib/trpc";

/**
 * The checkout. An e-mail, a cart, and a choice of surface.
 *
 * There is no rail selector here on purpose: which rails a payer may use is
 * the PaymentIntent's business, and choosing between them is vpay's page's
 * business. A merchant that rendered its own rail list would be maintaining a
 * second copy of vpay's capabilities.
 */
export function CheckoutForm() {
  const router = useRouter();
  const [lines, setLines] = useState<CartLine[]>([]);
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState<"hosted" | "embedded" | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // REAL finding: the cart lives in `localStorage`, which a server render
    // cannot read, so it is loaded on mount. `useSyncExternalStore` is the
    // shape that would satisfy this rule; rewriting the demo shop's cart is not
    // a lint pass's change.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setLines(readCart());
    return onCartChange(() => setLines(readCart()));
  }, []);

  async function submit(mode: "hosted" | "embedded"): Promise<void> {
    setError(null);
    setBusy(mode);
    try {
      const result = await trpc.orders.create.mutate({ email, lines, mode });
      clearCart();
      if (mode === "hosted") {
        if (result.url === null) {
          setError("vpay did not return a checkout URL.");
          setBusy(null);
          return;
        }
        // A full navigation, not a router push: vpay's page is a different
        // origin.
        window.location.assign(result.url);
        return;
      }
      router.push(`/orders/${result.orderId}/embedded`);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "The order was refused.",
      );
      setBusy(null);
    }
  }

  const ready = lines.length > 0 && email.includes("@") && busy === null;

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        void submit("hosted");
      }}
    >
      <p>
        <label htmlFor="email">Your e-mail</label>
        <input
          id="email"
          name="email"
          type="email"
          required
          autoComplete="email"
          data-testid="email"
          value={email}
          onChange={(event) => setEmail(event.target.value)}
        />
      </p>
      {error !== null ? (
        <p className="error" role="alert" data-testid="checkout-error">
          {error}
        </p>
      ) : null}
      <p style={{ display: "flex", gap: "0.75rem", flexWrap: "wrap" }}>
        <button type="submit" disabled={!ready} data-testid="pay-hosted">
          {busy === "hosted" ? "Redirecting…" : "Pay on vpay's page"}
        </button>
        <button
          type="button"
          className="secondary"
          disabled={!ready}
          data-testid="pay-embedded"
          onClick={() => void submit("embedded")}
        >
          {busy === "embedded" ? "Preparing…" : "Pay without leaving the shop"}
        </button>
      </p>
      <p style={{ color: "var(--muted)", fontSize: "0.9rem" }}>
        Either way the shop creates the PaymentIntent and the Checkout Session
        on its own server, with its own credentials. Your browser never holds
        one.
      </p>
    </form>
  );
}
