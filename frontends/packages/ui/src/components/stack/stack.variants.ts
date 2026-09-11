import { cva } from "class-variance-authority";

/**
 * Every visual decision `Stack` makes, in one map.
 *
 * Layout and spacing utilities are the one raw-utility family §3 of
 * docs/plans/2026-09-07-ui-revamp.md permits, and only inside this package:
 * the literal `flex items-center gap-3` exists here once rather than once per
 * screen.
 */
export const stack = cva("flex", {
  variants: {
    direction: { row: "flex-row", column: "flex-col" },
    align: {
      start: "items-start",
      center: "items-center",
      end: "items-end",
      stretch: "items-stretch",
    },
    justify: {
      start: "justify-start",
      center: "justify-center",
      end: "justify-end",
      between: "justify-between",
    },
    gap: { xs: "gap-1", sm: "gap-2", md: "gap-4", lg: "gap-6" },
    wrap: { true: "flex-wrap", false: "" },
  },
  defaultVariants: {
    direction: "row",
    align: "center",
    justify: "start",
    gap: "sm",
    wrap: false,
  },
});
