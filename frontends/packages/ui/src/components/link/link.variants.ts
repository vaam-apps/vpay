import { cva } from "class-variance-authority";

/**
 * Every visual decision `Link` makes, in one map.
 *
 * daisyUI 5's `link` is underline-only and inherits its colour, so a link
 * inside body copy stays readable whatever `branding.yaml` retints. `tone`
 * offers the two the product actually uses; there is deliberately no
 * `link-primary` variant for a navigation link, because a coloured link on a
 * retinted primary is the one pair `theme-contrast.test.ts` cannot vouch for.
 */
export const link = cva("link", {
  variants: {
    tone: { default: "", hover: "link-hover" },
  },
  defaultVariants: { tone: "default" },
});
