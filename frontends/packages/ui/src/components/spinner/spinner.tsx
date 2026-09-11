import { type VariantProps } from "class-variance-authority";

import { cn } from "../../cn";
import { spinner } from "./spinner.variants";

export interface SpinnerProps
  extends
    Omit<React.ComponentPropsWithoutRef<"span">, "children">,
    VariantProps<typeof spinner> {
  /** Accessible label. The spinner itself is `aria-hidden`. */
  label?: string;
}

/**
 * A loading indicator.
 *
 * No Base UI primitive — a `<span aria-hidden>`. `label`, when given, is
 * rendered as visually-hidden text so a screen reader announces the wait
 * rather than nothing at all.
 */
export function Spinner({ size, label, className, ...rest }: SpinnerProps) {
  return (
    <>
      <span
        aria-hidden
        className={cn(spinner({ size }), className)}
        {...rest}
      />
      {label ? <span className="sr-only">{label}</span> : null}
    </>
  );
}
