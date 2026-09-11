import { cva } from "class-variance-authority";

/**
 * Every visual decision `Code` makes, in one map.
 *
 * `base-200`/`base-content` is a daisyUI theme token pair, so the tint moves
 * with the theme rather than being a palette colour written at a call site.
 * `wrap: "anywhere"` is not decoration: a payment intent id is 27 unbroken
 * characters and a charge's `provider_reference_id` is longer, and inside a
 * table cell on a phone an unbreakable run overflows its column silently.
 */
export const code = cva("rounded bg-base-200 px-1 font-mono text-sm", {
  variants: { wrap: { normal: "", anywhere: "break-all" } },
  defaultVariants: { wrap: "normal" },
});
