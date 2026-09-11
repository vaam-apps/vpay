"use client";

import { Input as BaseInput } from "@base-ui/react/input";
import { type VariantProps } from "class-variance-authority";

import { cn } from "../../cn";
import { input } from "./input.variants";

export interface InputProps
  extends
    Omit<React.ComponentProps<typeof BaseInput>, "className">,
    VariantProps<typeof input> {
  className?: string;
}

/**
 * A text input.
 *
 * `@base-ui/react/input`'s `Input` self-registers with an enclosing
 * `Field.Root` — id, `aria-describedby`, `aria-invalid` are Base UI's job.
 * daisyUI 5 makes a bordered look the default (`input-bordered` is gone,
 * plan §6.3); `tone="ghost"` is the borderless variant.
 */
export function Input({ tone, className, ...rest }: InputProps) {
  return <BaseInput className={cn(input({ tone }), className)} {...rest} />;
}
