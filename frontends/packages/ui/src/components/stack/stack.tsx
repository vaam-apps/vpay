import { type VariantProps } from "class-variance-authority";

import { cn } from "../../cn";
import { stack } from "./stack.variants";

export interface StackProps
  extends React.ComponentPropsWithoutRef<"div">, VariantProps<typeof stack> {
  /**
   * The element to render. A `<div>` by default, because most stacks are
   * grouping and nothing else — but a stack that IS the page header or a
   * section is a landmark, and replacing it with a `<div>` silently removes
   * that landmark from the accessibility tree. `checkout-view.tsx` and
   * `return-view.tsx` each lost their `<header>` that way.
   */
  as?: "div" | "header" | "footer" | "section" | "nav" | "aside";
}

/** A flex row or column. Replaces `flex items-center gap-*` written inline per screen. */
export function Stack({
  as: Tag = "div",
  direction,
  align,
  justify,
  gap,
  wrap,
  className,
  ...rest
}: StackProps) {
  return (
    <Tag
      className={cn(stack({ direction, align, justify, gap, wrap }), className)}
      {...rest}
    />
  );
}
