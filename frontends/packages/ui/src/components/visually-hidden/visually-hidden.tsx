import { cn } from "../../cn";

export type VisuallyHiddenProps = React.ComponentPropsWithoutRef<"span">;

/**
 * Text for a screen reader and not for the eye.
 *
 * `sr-only` is in plan §3's permitted set and, like every utility in it,
 * belongs inside this package rather than at a call site. Every visually
 * hidden label on the checkout page goes through here.
 */
export function VisuallyHidden({ className, ...rest }: VisuallyHiddenProps) {
  return <span className={cn("sr-only", className)} {...rest} />;
}
