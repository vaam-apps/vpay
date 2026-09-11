import { cva } from "class-variance-authority";

/**
 * Every visual decision `Alert` makes, in one map.
 *
 * `tone` is the only variant on purpose: routing a status colour through
 * anything other than a `Record<Status, Tone>` from `@vpay/tokens` is the
 * 2026-09-07 defect the UI revamp fixed (plan §3, the `checkoutOutcomeTone`
 * mutation).
 */
export const alert = cva("mt-4 alert", {
  variants: {
    tone: {
      neutral: "",
      info: "alert-info",
      success: "alert-success",
      warning: "alert-warning",
      error: "alert-error",
    },
  },
  defaultVariants: { tone: "neutral" },
});
