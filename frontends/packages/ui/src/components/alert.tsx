import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from '../cn.js';

const alert = cva('mt-4 alert', {
  variants: {
    tone: {
      neutral: '',
      info: 'alert-info',
      success: 'alert-success',
      warning: 'alert-warning',
      error: 'alert-error',
    },
  },
  defaultVariants: { tone: 'neutral' },
});

export interface AlertProps
  extends React.ComponentPropsWithoutRef<'div'>,
    VariantProps<typeof alert> {}

/**
 * A status message.
 *
 * No Base UI primitive owns this — an alert is a `<div role="alert">`, not
 * a piece of interactive behaviour — so this component is daisyUI classes
 * and nothing else. `tone` is the only variant on purpose: routing a status
 * colour through anything other than a `Record<Status, Tone>` from
 * `@vpay/tokens` is the 2026-09-07 defect this revamp fixes (plan §3, the
 * `checkoutOutcomeTone` mutation).
 */
export function Alert({ tone, className, role = 'alert', ...rest }: AlertProps) {
  return <div role={role} className={cn(alert({ tone }), className)} {...rest} />;
}
