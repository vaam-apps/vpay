import { cva } from "class-variance-authority";

export const spinner = cva("loading loading-dots", {
  variants: {
    size: {
      xs: "loading-xs",
      sm: "loading-sm",
      md: "loading-md",
      lg: "loading-lg",
    },
  },
  defaultVariants: { size: "md" },
});
