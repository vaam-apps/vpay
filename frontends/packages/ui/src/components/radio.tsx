"use client";

import { Radio as BaseRadio } from "@base-ui/react/radio";
import { RadioGroup as BaseRadioGroup } from "@base-ui/react/radio-group";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "../cn";

export type RadioGroupProps = React.ComponentPropsWithoutRef<
  typeof BaseRadioGroup
>;

/** Groups a series of {@link Radio} buttons under one shared, validated value. */
export function RadioGroup({ className, ...rest }: RadioGroupProps) {
  return (
    <BaseRadioGroup
      className={cn("flex flex-col gap-2", className)}
      {...rest}
    />
  );
}

const radio = cva("radio", {
  variants: { size: { sm: "radio-sm", md: "" } },
  defaultVariants: { size: "md" },
});

export interface RadioProps
  extends
    Omit<React.ComponentProps<typeof BaseRadio.Root>, "className">,
    VariantProps<typeof radio> {
  className?: string;
}

/**
 * A single radio button.
 *
 * Renders `@base-ui/react/radio`'s default `<span role="radio">` with a
 * hidden native input beside it — daisyUI 5's `.radio` styles both
 * `:checked` (the hidden input) and `[aria-checked="true"]` (the visible
 * span), measured by compiling `styles.css` and reading the generated
 * rule, so the visible control paints correctly either way.
 */
export function Radio({ size, className, ...rest }: RadioProps) {
  return (
    <BaseRadio.Root className={cn(radio({ size }), className)} {...rest} />
  );
}
