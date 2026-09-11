import { type VariantProps } from "class-variance-authority";

import { cn } from "../../cn";
import { card } from "./card.variants";

export interface CardProps
  extends React.ComponentPropsWithoutRef<"div">, VariantProps<typeof card> {}

/** A grouped surface. No Base UI primitive — a `<div>`. */
export function Card({ size, className, ...rest }: CardProps) {
  return <div className={cn(card({ size }), className)} {...rest} />;
}

export type CardBodyProps = React.ComponentPropsWithoutRef<"div">;

export function CardBody({ className, ...rest }: CardBodyProps) {
  return <div className={cn("card-body gap-1", className)} {...rest} />;
}
