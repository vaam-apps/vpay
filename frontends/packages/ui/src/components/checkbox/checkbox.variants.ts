import { cva } from "class-variance-authority";

export const checkbox = cva("checkbox", {
  variants: {
    tone: { default: "", primary: "checkbox-primary" },
    size: { sm: "checkbox-sm", md: "" },
  },
  defaultVariants: { tone: "default", size: "md" },
});
