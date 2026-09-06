"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import {
  CheckoutPopupBlockedError,
  loadStripe,
} from "@vaam-apps/vpay-stripe-js";
import {
  clearCart,
  onCartChange,
  readCart,
  type CartLine,
} from "@/components/cart";
import { trpc } from "@/lib/trpc";

/** The three surfaces, in the order the page offers them. */
const MODES = ["hosted", "popup", "embedded"] as const;
export type CheckoutMode = (typeof MODES)[number];

const MODE_COPY: Readonly<
  Record<CheckoutMode, { label: string; blurb: string; button: string }>
> = {
  hosted: {
    label: "Redirect",
    blurb:
      "The whole tab goes to vpay's page and comes back to this shop afterwards. Nothing to embed and nothing a popup blocker can stop — the surface to reach for unless you have a reason not to.",
    button: "Pay on vpay's page",
  },
  popup: {
    label: "Popup",
    blurb:
      "vpay's page opens in a window this shop owns, so the cart stays on screen behind it. The window is opened by your click, before this shop's server is asked for anything, because that is the only moment a browser allows it.",
    button: "Pay in a popup",
  },
  embedded: {
    label: "Embedded",
    blurb:
      "vpay's page in an iframe on this shop's own page. The payer never leaves; the frame is sandboxed without allow-top-navigation, so a redirect rail hands back to this page rather than navigating itself.",
    button: "Pay without leaving the shop",
  },
};

/**
 * The checkout. A cart, an optional e-mail, and a choice of surface.
 *
 * There is no rail selector here on purpose: which rails a payer may use is
 * the PaymentIntent's business, and choosing between them is vpay's page's
 * business. A merchant that rendered its own rail list would be maintaining a
 * second copy of vpay's capabilities.
 *
 * There **is** a surface selector, and that is a demo affordance rather than
 * something a merchant would ship: the integration mode is the developer's
 * configuration (`SHOP_CHECKOUT_MODE`), chosen once and left alone. The
 * radio group starts on the configured value and exists so a reader of this
 * example can see all three without editing an environment file.
 */
export function CheckoutForm({
  defaultMode,
  publishableKey,
  checkoutBaseUrl,
  apiBaseUrl,
}: {
  defaultMode: CheckoutMode;
  publishableKey: string;
  checkoutBaseUrl: string;
  apiBaseUrl: string;
}) {
  const router = useRouter();
  const [lines, setLines] = useState<CartLine[]>([]);
  const [email, setEmail] = useState("");
  const [mode, setMode] = useState<CheckoutMode>(defaultMode);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    // REAL finding: the cart lives in `localStorage`, which a server render
    // cannot read, so it is loaded on mount. `useSyncExternalStore` is the
    // shape that would satisfy this rule; rewriting the demo shop's cart is not
    // a lint pass's change.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setLines(readCart());
    return onCartChange(() => setLines(readCart()));
  }, []);

  /**
   * The popup path, and the one place in this shop where CALL ORDER MATTERS.
   *
   * `openCheckoutPopup` is called synchronously from the click handler and
   * opens its window **before** it awaits `fetchCheckoutUrl` — this shop's
   * own `orders.create`. Awaiting the order first and opening afterwards is
   * the usual way a popup integration gets silently blocked.
   *
   * Which is also why the blocked-window fallback creates the order at that
   * point and not before: a refused window is refused before this shop's
   * server has been asked for anything, so there is no order yet to fall
   * back *to*, and creating one eagerly "just in case" would leave an
   * unpayable row behind every time the popup worked.
   */
  async function payInPopup(): Promise<void> {
    const stripe = await loadStripe(publishableKey, {
      baseUrl: apiBaseUrl,
      checkoutBaseUrl,
    });
    let createdOrderId: string | null = null;
    try {
      await stripe.openCheckoutPopup({
        fetchCheckoutUrl: async () => {
          const result = await trpc.orders.create.mutate({
            email: email === "" ? null : email,
            lines,
            mode: "popup",
          });
          createdOrderId = result.orderId;
          clearCart();
          if (result.url === null) {
            throw new Error("vpay did not return a checkout URL.");
          }
          return result.url;
        },
        onComplete: () => {
          // A message from a window, not proof of payment. It sends the
          // browser to the return page, which polls this shop's database —
          // which only the verified webhook writes.
          if (createdOrderId !== null) {
            router.push(`/orders/${createdOrderId}/return`);
          }
        },
        onCancel: () => {
          // Not a cancellation of the charge: the payer closed a window. The
          // order exists and may still settle, so the shop sends them to it
          // rather than pretending nothing happened.
          if (createdOrderId !== null) {
            router.push(`/orders/${createdOrderId}`);
          }
        },
      });
      setNote(
        "Paying in a popup. If you cannot see it, look behind this window.",
      );
    } catch (cause) {
      if (!(cause instanceof CheckoutPopupBlockedError)) {
        throw cause;
      }
      // The fallback the SDK's error exists for: a full-page navigation,
      // which no browser blocks.
      const result = await trpc.orders.create.mutate({
        email: email === "" ? null : email,
        lines,
        mode: "hosted",
      });
      clearCart();
      if (result.url === null) {
        setError("vpay did not return a checkout URL.");
        return;
      }
      setError(
        "Your browser blocked the payment window, so we are sending you to vpay's page instead.",
      );
      window.location.assign(result.url);
    }
  }

  async function submit(chosen: CheckoutMode): Promise<void> {
    setError(null);
    setNote(null);
    setBusy(true);
    try {
      if (chosen === "popup") {
        await payInPopup();
        setBusy(false);
        return;
      }
      const result = await trpc.orders.create.mutate({
        email: email === "" ? null : email,
        lines,
        mode: chosen,
      });
      clearCart();
      if (chosen === "hosted") {
        if (result.url === null) {
          setError("vpay did not return a checkout URL.");
          setBusy(false);
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
      setBusy(false);
    }
  }

  const ready = lines.length > 0 && !busy;

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        void submit(mode);
      }}
    >
      <p>
        <label htmlFor="email">
          Your e-mail <span style={{ color: "var(--muted)" }}>(optional)</span>
        </label>
        <input
          id="email"
          name="email"
          type="email"
          autoComplete="email"
          data-testid="email"
          aria-describedby="email-why"
          value={email}
          onChange={(event) => setEmail(event.target.value)}
        />
      </p>
      <p
        id="email-why"
        data-testid="email-why"
        style={{ color: "var(--muted)", fontSize: "0.9rem" }}
      >
        For your receipt only. You can pay without it: on a mobile-money rail
        the identity is the <strong>phone number</strong> you give the rail,
        which this shop never sees.
      </p>

      <fieldset data-testid="mode-switch">
        <legend>How to pay</legend>
        <p style={{ color: "var(--muted)", fontSize: "0.9rem" }}>
          A real shop picks one of these once, in its own configuration (
          <code>SHOP_CHECKOUT_MODE</code>, currently{" "}
          <code data-testid="configured-mode">{defaultMode}</code>), and never
          shows a switch. This one shows all three so you can see each.
        </p>
        {MODES.map((candidate) => (
          <label key={candidate} className="mode-option">
            <input
              type="radio"
              name="mode"
              value={candidate}
              data-testid={`mode-${candidate}`}
              checked={mode === candidate}
              onChange={() => setMode(candidate)}
            />
            <span>
              <strong>{MODE_COPY[candidate].label}</strong>{" "}
              <span style={{ color: "var(--muted)" }}>
                {MODE_COPY[candidate].blurb}
              </span>
            </span>
          </label>
        ))}
      </fieldset>

      {error !== null ? (
        <p className="error" role="alert" data-testid="checkout-error">
          {error}
        </p>
      ) : null}
      {note !== null ? (
        <p role="status" data-testid="checkout-note">
          {note}
        </p>
      ) : null}
      <p>
        <button type="submit" disabled={!ready} data-testid="pay">
          {busy ? "Working…" : MODE_COPY[mode].button}
        </button>
      </p>
      <p style={{ color: "var(--muted)", fontSize: "0.9rem" }}>
        Whichever you pick, the shop creates the PaymentIntent and the Checkout
        Session on its own server, with its own credentials. Your browser never
        holds one.
      </p>
    </form>
  );
}
