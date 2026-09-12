import { cn } from "../../cn";

export type ListProps = React.ComponentPropsWithoutRef<"ul">;

/**
 * A plain vertical list. Replaces `mt-3 space-y-1 text-sm opacity-70`
 * written inline.
 *
 * The quiet is `text-muted-ink`, not `opacity-70`, since 2026-09-12.
 * Opacity compounds: this list dimmed to 0.7 and a `Text tone="muted"`
 * inside it dimmed again to 0.6, so that line rendered at 0.42 — #9d9d9d,
 * 2.71:1, below WCAG AA — while each of the two was defensible on its own.
 * Nothing at either call site could reveal that. Two colours do not
 * compound; nesting them now yields one ink. See `styles.css`.
 */
export function List({ className, ...rest }: ListProps) {
  return (
    <ul
      className={cn("mt-3 space-y-1 text-sm text-muted-ink", className)}
      {...rest}
    />
  );
}
