import { cva } from "class-variance-authority";

export const badge = cva("badge gap-1 whitespace-nowrap", {
  variants: {
    tone: {
      neutral: "badge-neutral",
      info: "badge-info",
      success: "badge-success",
      warning: "badge-warning",
      error: "badge-error",
      ghost: "badge-ghost",
    },
    size: { xs: "badge-xs", sm: "badge-sm", md: "", lg: "badge-lg" },
  },
  defaultVariants: { tone: "neutral", size: "md" },
});
