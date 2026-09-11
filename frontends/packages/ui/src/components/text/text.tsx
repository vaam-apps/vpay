import { type VariantProps } from "class-variance-authority";

import { cn } from "../../cn";
import { text } from "./text.variants";

export interface TextProps
  extends React.ComponentPropsWithoutRef<"p">, VariantProps<typeof text> {
  as?: "p" | "span";
}

/** Inline or block copy. `tone="muted"` / `tone="error"` replace ad hoc `opacity-*` / `text-error`. */
export function Text({
  as: Tag = "p",
  tone,
  size,
  weight,
  numeric,
  wrap,
  className,
  ...rest
}: TextProps) {
  return (
    <Tag
      className={cn(text({ tone, size, weight, numeric, wrap }), className)}
      {...rest}
    />
  );
}
