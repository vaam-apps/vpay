import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from '../cn.js';

const table = cva('table', {
  variants: {
    zebra: { true: 'table-zebra', false: '' },
    size: { xs: 'table-xs', sm: 'table-sm', md: '', lg: 'table-lg' },
  },
  defaultVariants: { zebra: false, size: 'md' },
});

export interface TableProps
  extends React.ComponentPropsWithoutRef<'table'>,
    VariantProps<typeof table> {}

/** A data table. No Base UI primitive — a `<table>`, daisyUI classes only. */
export function Table({ zebra, size, className, ...rest }: TableProps) {
  return <table className={cn(table({ zebra, size }), className)} {...rest} />;
}
