import { type VariantProps } from "class-variance-authority";

import { cn } from "../../cn";
import { table } from "./table.variants";

export interface TableProps
  extends React.ComponentPropsWithoutRef<"table">, VariantProps<typeof table> {}

/** A data table. No Base UI primitive — a `<table>`, daisyUI classes only. */
export function Table({ zebra, size, className, ...rest }: TableProps) {
  return <table className={cn(table({ zebra, size }), className)} {...rest} />;
}
