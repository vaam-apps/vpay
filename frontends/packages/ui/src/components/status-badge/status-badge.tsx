import { statusLabel, statusTone, type PaymentStatus } from "@vpay/tokens";

import { Badge, type BadgeProps } from "../badge";

export interface StatusBadgeProps extends Omit<
  BadgeProps,
  "tone" | "children"
> {
  status: PaymentStatus;
}

/**
 * Renders a PaymentIntent status.
 *
 * Tone and copy both come from `@vpay/tokens`, so a status can never be
 * coloured green in one view and grey in another. Composes {@link Badge}
 * rather than carrying its own `cva` map — one badge variant map, not two.
 */
export function StatusBadge({
  status,
  size,
  className,
  ...rest
}: StatusBadgeProps) {
  return (
    <Badge
      tone={statusTone[status]}
      size={size}
      className={className}
      data-status={status}
      {...rest}
    >
      {statusLabel[status]}
    </Badge>
  );
}
