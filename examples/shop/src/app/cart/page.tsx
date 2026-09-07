import { CartTable } from "@/components/cart-table";

export default function CartPage() {
  return (
    <>
      <h1 className="mb-4 text-2xl font-bold">Cart</h1>
      <CartTable showCheckoutLink />
    </>
  );
}
