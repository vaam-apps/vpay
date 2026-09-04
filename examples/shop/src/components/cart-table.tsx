"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import {
  onCartChange,
  readCart,
  setQuantity,
  type CartLine,
} from "@/components/cart";
import { formatMinor } from "@/money";
import { trpc } from "@/lib/trpc";

interface Product {
  id: string;
  name: string;
  priceMinor: number;
  currency: string;
}

/**
 * The cart, priced for display only.
 *
 * The totals shown here are computed in the browser from the catalogue the
 * browser fetched; the totals that reach vpay are computed again, server-side,
 * in `orders.create`. If the two ever disagreed the server's would win, which
 * is the correct direction for them to disagree in.
 */
export function CartTable({ showCheckoutLink }: { showCheckoutLink: boolean }) {
  const [lines, setLines] = useState<CartLine[]>([]);
  const [products, setProducts] = useState<Product[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setLines(readCart());
    return onCartChange(() => setLines(readCart()));
  }, []);

  useEffect(() => {
    let live = true;
    trpc.products.list
      .query()
      .then((rows) => {
        if (live) {
          setProducts(rows);
        }
      })
      .catch(() => {
        if (live) {
          setError("The catalogue could not be loaded.");
        }
      });
    return () => {
      live = false;
    };
  }, []);

  if (error !== null) {
    return <p className="error">{error}</p>;
  }
  if (products === null) {
    return <p>Loading the catalogue…</p>;
  }
  if (lines.length === 0) {
    return (
      <p data-testid="cart-empty">
        The cart is empty. <Link href="/">Back to the catalogue</Link>.
      </p>
    );
  }

  const byId = new Map(products.map((product) => [product.id, product]));
  const rows = lines.flatMap((line) => {
    const product = byId.get(line.productId);
    return product === undefined ? [] : [{ line, product }];
  });
  const currency = rows[0]?.product.currency ?? "xaf";
  const total = rows.reduce(
    (sum, row) => sum + row.product.priceMinor * row.line.quantity,
    0,
  );

  return (
    <>
      <table data-testid="cart-table">
        <thead>
          <tr>
            <th>Item</th>
            <th className="num">Unit</th>
            <th className="num">Qty</th>
            <th className="num">Line</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ line, product }) => (
            <tr key={product.id}>
              <td>{product.name}</td>
              <td className="num">
                {formatMinor(product.priceMinor, product.currency)}
              </td>
              <td className="num">
                <input
                  type="number"
                  min={0}
                  max={10}
                  step={1}
                  value={line.quantity}
                  aria-label={`Quantity of ${product.name}`}
                  data-testid={`qty-${product.id}`}
                  style={{ width: "5rem" }}
                  onChange={(event) =>
                    setQuantity(product.id, Number(event.target.value))
                  }
                />
              </td>
              <td className="num">
                {formatMinor(
                  product.priceMinor * line.quantity,
                  product.currency,
                )}
              </td>
            </tr>
          ))}
          <tr>
            <td colSpan={3}>
              <strong>Total</strong>
            </td>
            <td className="num" data-testid="cart-total">
              <strong>{formatMinor(total, currency)}</strong>
            </td>
          </tr>
        </tbody>
      </table>
      {showCheckoutLink ? (
        <p style={{ marginTop: "1.25rem" }}>
          <Link className="button" href="/checkout" data-testid="to-checkout">
            Checkout
          </Link>
        </p>
      ) : null}
    </>
  );
}
