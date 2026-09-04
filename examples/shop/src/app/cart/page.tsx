import { CartTable } from "@/components/cart-table";

export default function CartPage() {
  return (
    <>
      <h1>Cart</h1>
      <CartTable showCheckoutLink />
    </>
  );
}
