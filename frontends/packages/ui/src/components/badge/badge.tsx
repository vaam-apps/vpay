import { type VariantProps } from "class-variance-authority";

import { cn } from "../../cn";
import { badge } from "./badge.variants";

export interface BadgeProps
  extends React.ComponentPropsWithoutRef<"span">, VariantProps<typeof badge> {}

/** A small status or count marker. No Base UI primitive — a `<span>`. */
export function Badge({ tone, size, className, ...rest }: BadgeProps) {
  return <span className={cn(badge({ tone, size }), className)} {...rest} />;
}
