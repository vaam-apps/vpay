import { cva, type VariantProps } from 'class-variance-authority';
import { statusLabel, statusTone, type PaymentStatus } from '@vpay/tokens';

import { cn } from '../cn';

const badge = cva('badge gap-1 whitespace-nowrap', {
  variants: {
    tone: {
      neutral: 'badge-neutral',
      info: 'badge-info',
      warning: 'badge-warning',
      success: 'badge-success',
      error: 'badge-error',
    },
    size: { sm: 'badge-sm', md: '', lg: 'badge-lg' },
  },
  defaultVariants: { tone: 'neutral', size: 'md' },
});

export interface StatusBadgeProps extends Omit<VariantProps<typeof badge>, 'tone'> {
  status: PaymentStatus;
  className?: string;
}

/**
 * Renders a PaymentIntent status.
 *
 * Tone and copy both come from `@vpay/tokens`, so a status can never be
 * coloured green in one view and grey in another.
 */
export function StatusBadge({ status, size, className }: StatusBadgeProps) {
  return (
    <span
      className={cn(badge({ tone: statusTone[status], size }), className)}
      data-status={status}
    >
      {statusLabel[status]}
    </span>
  );
}
