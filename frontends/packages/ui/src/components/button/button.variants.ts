import { cva } from "class-variance-authority";

/**
 * Every visual decision this component makes, in one map.
 *
 * Nothing below this constant writes a class name, and nothing above it
 * writes a colour: `btn-primary` resolves through daisyUI's
 * `--color-primary`, which `branding.yaml` retints at runtime. See
 * docs/flows/hosted-checkout.md.
 */
export const button = cva("btn", {
  variants: {
    variant: {
      primary: "btn-primary",
      ghost: "btn-ghost",
      outline: "btn-outline",
      danger: "btn-error",
    },
    size: { xs: "btn-xs", sm: "btn-sm", md: "", lg: "btn-lg" },
    block: { true: "btn-block", false: "" },
  },
  defaultVariants: { variant: "primary", size: "md", block: false },
});
