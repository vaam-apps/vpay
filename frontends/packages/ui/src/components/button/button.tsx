"use client";

import { Button as BaseButton } from "@base-ui/react/button";
import { type VariantProps } from "class-variance-authority";

import { cn } from "../../cn";
import { button } from "./button.variants";

export interface ButtonProps
  extends
    Omit<React.ComponentProps<typeof BaseButton>, "className">,
    VariantProps<typeof button> {
  className?: string;
}

/**
 * A button.
 *
 * Wraps `@base-ui/react/button`'s `Button` rather than reimplementing
 * composition: it already carries the `render` escape hatch —
 * `render={<Link href="…" />} nativeButton={false}` makes this a link that
 * looks and behaves like a button, which is what `examples/shop` needs —
 * and the disabled/focus-visible behaviour a hand-rolled version would have
 * to reproduce. Pass `nativeButton={false}` whenever `render` produces
 * anything other than a real `<button>` — Base UI warns in development if
 * it does not match what actually got rendered.
 */
export function Button({
  variant,
  size,
  block,
  className,
  ...rest
}: ButtonProps) {
  return (
    <BaseButton
      className={cn(button({ variant, size, block }), className)}
      {...rest}
    />
  );
}
