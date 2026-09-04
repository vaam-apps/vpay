import { CartTable } from "@/components/cart-table";
import { CheckoutForm } from "@/components/checkout-form";

export default function CheckoutPage() {
  return (
    <>
      <h1>Checkout</h1>
      <CartTable showCheckoutLink={false} />
      <h2 style={{ marginTop: "2rem" }}>Where to send the receipt</h2>
      <CheckoutForm />
    </>
  );
}
