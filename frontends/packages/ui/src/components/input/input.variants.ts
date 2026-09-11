import { cva } from "class-variance-authority";

export const input = cva("input w-full", {
  variants: {
    tone: { default: "", ghost: "input-ghost", error: "input-error" },
  },
  defaultVariants: { tone: "default" },
});
