'use client';

import { Button as BaseButton } from '@base-ui/react/button';
import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from '../cn';

/**
 * Every visual decision this component makes, in one map.
 *
 * Nothing below this constant writes a class name, and nothing above it
 * writes a colour: `btn-primary` resolves through daisyUI's
 * `--color-primary`, which `branding.yaml` retints at runtime. See
 * docs/flows/hosted-checkout.md.
 */
const button = cva('btn', {
  variants: {
    variant: {
      primary: 'btn-primary',
      ghost: 'btn-ghost',
      outline: 'btn-outline',
      danger: 'btn-error',
    },
    size: { xs: 'btn-xs', sm: 'btn-sm', md: '', lg: 'btn-lg' },
    block: { true: 'btn-block', false: '' },
  },
  defaultVariants: { variant: 'primary', size: 'md', block: false },
});

export interface ButtonProps
  extends Omit<React.ComponentProps<typeof BaseButton>, 'className'>,
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
export function Button({ variant, size, block, className, ...rest }: ButtonProps) {
  return <BaseButton className={cn(button({ variant, size, block }), className)} {...rest} />;
}
