import { cva } from "class-variance-authority";

export const radio = cva("radio", {
  variants: { size: { sm: "radio-sm", md: "" } },
  defaultVariants: { size: "md" },
});
