import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from '../cn';

export const badge = cva('badge gap-1 whitespace-nowrap', {
  variants: {
    tone: {
      neutral: 'badge-neutral',
      info: 'badge-info',
      success: 'badge-success',
      warning: 'badge-warning',
      error: 'badge-error',
      ghost: 'badge-ghost',
    },
    size: { xs: 'badge-xs', sm: 'badge-sm', md: '', lg: 'badge-lg' },
  },
  defaultVariants: { tone: 'neutral', size: 'md' },
});

export interface BadgeProps
  extends React.ComponentPropsWithoutRef<'span'>,
    VariantProps<typeof badge> {}

/** A small status or count marker. No Base UI primitive — a `<span>`. */
export function Badge({ tone, size, className, ...rest }: BadgeProps) {
  return <span className={cn(badge({ tone, size }), className)} {...rest} />;
}
