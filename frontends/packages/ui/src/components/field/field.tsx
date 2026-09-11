"use client";

import { Field as BaseField } from "@base-ui/react/field";

import { cn } from "../../cn";

export type FieldProps = React.ComponentPropsWithoutRef<typeof BaseField.Root>;

/**
 * Groups a label, a control, an optional hint and a validation error.
 *
 * Sits on `@base-ui/react/field`: the label/hint/error association is Base
 * UI's job (`aria-describedby`, `aria-invalid`, id generation), not
 * something this wrapper reimplements. daisyUI 5 removed `form-control` and
 * `label-text` (plan §6.3, §8.3) — this replaces both with the `fieldset` /
 * `legend` / `label` markup daisyUI 5 actually styles.
 */
export function Field({ className, ...rest }: FieldProps) {
  return (
    <BaseField.Root className={cn("fieldset gap-1", className)} {...rest} />
  );
}

export type FieldLabelProps = React.ComponentPropsWithoutRef<
  typeof BaseField.Label
>;

export function FieldLabel({ className, ...rest }: FieldLabelProps) {
  return <BaseField.Label className={cn("label", className)} {...rest} />;
}

export type FieldDescriptionProps = React.ComponentPropsWithoutRef<
  typeof BaseField.Description
>;

export function FieldDescription({
  className,
  ...rest
}: FieldDescriptionProps) {
  return (
    <BaseField.Description
      className={cn("text-xs opacity-70", className)}
      {...rest}
    />
  );
}

export type FieldErrorProps = React.ComponentPropsWithoutRef<
  typeof BaseField.Error
>;

export function FieldError({ className, ...rest }: FieldErrorProps) {
  return (
    <BaseField.Error
      className={cn("text-xs text-error", className)}
      {...rest}
    />
  );
}
