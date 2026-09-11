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
 *
 * The spread comes FIRST, and that is load-bearing rather than style. Written
 * the other way round — `aria-live={politeness} aria-atomic="true" {...rest}`,
 * which is how this landed on 2026-09-11 — the two attributes the component
 * exists to own are the two a later prop overwrites. `Omit` keeps a
 * *typed* call site from passing either, but a spread of a widened object does
 * not go through that check, and the failure is silent: a payment outcome
 * that is never announced looks exactly like one that is. `live-region.test.tsx`
 * pushes both attributes in through a cast and expects them ignored.
 */
export function LiveRegion({
  politeness = "polite",
  ...rest
}: LiveRegionProps) {
  return <div {...rest} aria-live={politeness} aria-atomic="true" />;
}
