import { cva } from "class-variance-authority";

/**
 * Every visual decision `Text` makes, in one map.
 *
 * `3xl` is the payment amount on the checkout page — the single number a
 * payer checks before approving. It is a size the product needs, so it is a
 * variant here rather than a `text-3xl` written at the call site.
 *
 * `numeric` selects lining figures, for amounts and any other column of
 * digits. `wrap: "anywhere"` lets a long unbroken id (a session reference)
 * wrap inside its card instead of overflowing it.
 */
export const text = cva("", {
  variants: {
    // Colours, not opacities. `muted` was `opacity-60` and `error` was
    // `text-error` until 2026-09-12; both are measured tokens now, and
    // `styles.css` carries the numbers and the reason. The short version:
    // opacity compounds with an ancestor's and colour does not, and
    // `text-error` is daisyUI's error FILL, which is 2.92:1 on white.
    tone: {
      default: "",
      muted: "text-muted-ink",
      error: "text-error-ink",
    },
    size: {
      xs: "text-xs",
      sm: "text-sm",
      md: "text-base",
      lg: "text-lg",
      "3xl": "text-3xl",
    },
    weight: { normal: "", medium: "font-medium", semibold: "font-semibold" },
    numeric: { true: "tabular-nums", false: "" },
    wrap: { normal: "", anywhere: "break-all" },
  },
  defaultVariants: {
    tone: "default",
    size: "md",
    weight: "normal",
    numeric: false,
    wrap: "normal",
  },
});
