import { cva } from "class-variance-authority";

export const card = cva("card bg-base-200", {
  variants: { size: { sm: "card-sm", md: "", lg: "card-lg" } },
  defaultVariants: { size: "md" },
});
