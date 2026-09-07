'use client';

import { Checkbox as BaseCheckbox } from '@base-ui/react/checkbox';
import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from '../cn.js';

const checkbox = cva('checkbox', {
  variants: {
    tone: { default: '', primary: 'checkbox-primary' },
    size: { sm: 'checkbox-sm', md: '' },
  },
  defaultVariants: { tone: 'default', size: 'md' },
});

export interface CheckboxProps
  extends Omit<React.ComponentProps<typeof BaseCheckbox.Root>, 'className' | 'render' | 'nativeButton'>,
    VariantProps<typeof checkbox> {
  className?: string;
}

/**
 * A checkbox.
 *
 * Decision D2 (2026-09-07): renders Base UI 1.8.0's native `<button>`
 * control (`render={<button type="button" />}` + `nativeButton`) rather
 * than the default `<span role="checkbox">` with a visually-hidden native
 * input beside it. A native button gets the browser's own focus, hit-testing
 * and forced-colours behaviour for free; the ARIA the default renders
 * (`role="checkbox"`, `aria-checked`) is unchanged either way — Base UI
 * applies it to whichever element `render` produces.
 */
export function Checkbox({ tone, size, className, ...rest }: CheckboxProps) {
  return (
    <BaseCheckbox.Root
      nativeButton
      render={<button type="button" />}
      className={cn(checkbox({ tone, size }), className)}
      {...rest}
    >
      <BaseCheckbox.Indicator
        className="flex h-full w-full items-center justify-center"
        keepMounted={false}
      />
    </BaseCheckbox.Root>
  );
}
