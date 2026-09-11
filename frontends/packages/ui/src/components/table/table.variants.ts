import { cva } from "class-variance-authority";

export const table = cva("table", {
  variants: {
    zebra: { true: "table-zebra", false: "" },
    size: { xs: "table-xs", sm: "table-sm", md: "", lg: "table-lg" },
  },
  defaultVariants: { zebra: false, size: "md" },
});
