import { cn } from "../../cn";

export type ListProps = React.ComponentPropsWithoutRef<"ul">;

/** A plain vertical list. Replaces `mt-3 space-y-1 text-sm opacity-70` written inline. */
export function List({ className, ...rest }: ListProps) {
  return (
    <ul
      className={cn("mt-3 space-y-1 text-sm opacity-70", className)}
      {...rest}
    />
  );
}
