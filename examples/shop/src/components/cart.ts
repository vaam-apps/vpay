"use client";

/**
 * The cart, in the browser.
 *
 * `localStorage`, not a cookie and not a server table: D13 says guest
 * checkout with no accounts, and a cart the server never sees is a cart the
 * server can never be confused by. What the server *does* receive, at
 * `orders.create`, is product ids and quantities — no prices — so a tampered
 * cart can change what is bought and never what it costs.
 */

export interface CartLine {
  productId: string;
  quantity: number;
}

const KEY = "vpay-shop-cart";

/** Reads the cart, tolerating anything that is not the shape it expects. */
export function readCart(): CartLine[] {
  if (typeof window === "undefined") {
    return [];
  }
  let raw: string | null;
  try {
    raw = window.localStorage.getItem(KEY);
  } catch {
    // Private mode, or storage disabled. An empty cart is the honest answer.
    return [];
  }
  if (raw === null) {
    return [];
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    const lines: CartLine[] = [];
    for (const entry of parsed) {
      if (typeof entry !== "object" || entry === null) {
        continue;
      }
      const record = entry as Record<string, unknown>;
      const productId = record["productId"];
      const quantity = record["quantity"];
      if (
        typeof productId === "string" &&
        productId.length > 0 &&
        typeof quantity === "number" &&
        Number.isInteger(quantity) &&
        quantity > 0
      ) {
        lines.push({ productId, quantity });
      }
    }
    return lines;
  } catch {
    return [];
  }
}

function writeCart(lines: CartLine[]): void {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(lines));
  } catch {
    // Nothing to do: the next read will simply see an empty cart.
  }
  window.dispatchEvent(new Event("vpay-shop-cart-changed"));
}

export function addToCart(productId: string, quantity = 1): void {
  const lines = readCart();
  const existing = lines.find((line) => line.productId === productId);
  if (existing) {
    existing.quantity += quantity;
  } else {
    lines.push({ productId, quantity });
  }
  writeCart(lines);
}

export function setQuantity(productId: string, quantity: number): void {
  const lines = readCart().flatMap((line) =>
    line.productId === productId
      ? quantity > 0
        ? [{ productId, quantity }]
        : []
      : [line],
  );
  writeCart(lines);
}

export function clearCart(): void {
  writeCart([]);
}

/** Subscribes to cart changes made anywhere in this tab or another one. */
export function onCartChange(listener: () => void): () => void {
  window.addEventListener("vpay-shop-cart-changed", listener);
  window.addEventListener("storage", listener);
  return () => {
    window.removeEventListener("vpay-shop-cart-changed", listener);
    window.removeEventListener("storage", listener);
  };
}
