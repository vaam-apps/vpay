import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from '../cn';

const card = cva('card bg-base-200', {
  variants: { size: { sm: 'card-sm', md: '', lg: 'card-lg' } },
  defaultVariants: { size: 'md' },
});

export interface CardProps
  extends React.ComponentPropsWithoutRef<'div'>,
    VariantProps<typeof card> {}

/** A grouped surface. No Base UI primitive — a `<div>`. */
export function Card({ size, className, ...rest }: CardProps) {
  return <div className={cn(card({ size }), className)} {...rest} />;
}

export type CardBodyProps = React.ComponentPropsWithoutRef<'div'>;

export function CardBody({ className, ...rest }: CardBodyProps) {
  return <div className={cn('card-body gap-1', className)} {...rest} />;
}
