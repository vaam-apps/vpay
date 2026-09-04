"use client";

import { useState } from "react";
import { addToCart } from "@/components/cart";

export function AddToCart({
  productId,
  name,
}: {
  productId: string;
  name: string;
}) {
  const [added, setAdded] = useState(false);
  return (
    <button
      type="button"
      data-testid={`add-${productId}`}
      aria-label={`Add ${name} to the cart`}
      onClick={() => {
        addToCart(productId, 1);
        setAdded(true);
        window.setTimeout(() => setAdded(false), 1200);
      }}
    >
      {added ? "Added" : "Add to cart"}
    </button>
  );
}
