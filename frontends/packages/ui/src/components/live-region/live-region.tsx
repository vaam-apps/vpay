export interface LiveRegionProps extends Omit<
  React.ComponentPropsWithoutRef<"div">,
  "aria-live" | "aria-atomic" | "className"
> {
  /**
   * `polite` waits for a pause; `assertive` interrupts. Default `polite`:
   * an interrupting announcement on every state change of a payment is
   * unusable, and nothing in this product is urgent enough to earn one.
   */
  politeness?: "polite" | "assertive";
}

/**
 * The one element on a screen whose changes are announced.
 *
 * The checkout's state machine replaces the screen under the payer — from
 * "choose a rail" to "waiting for your approval" to "the payment failed" —
 * and a screen reader says nothing about a subtree React swapped out. Both
 * `checkout-view.tsx` and `return-view.tsx` carried the identical
 * `<div aria-live="polite" aria-atomic="true">` for that reason; it is one
 * component now so a third screen cannot get the attributes subtly wrong.
 *
 * `aria-atomic` is not optional and so is not a prop: a partial
 * announcement of a payment outcome ("failed" without "your payment") is
 * worse than none.
 *
 * No `className`: this element is not seen. Anything visible belongs to the
 * children.
 */
export function LiveRegion({
  politeness = "polite",
  ...rest
}: LiveRegionProps) {
  return <div aria-live={politeness} aria-atomic="true" {...rest} />;
}
