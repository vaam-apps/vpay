import { cn } from "../../cn";

export type PageShellProps = React.ComponentPropsWithoutRef<"div">;

/**
 * The one-column page frame every checkout and dashboard screen sits in.
 *
 * Replaces `mx-auto flex w-full max-w-md flex-col gap-6 p-6`, which plan
 * §4.1 found duplicated verbatim across `checkout-view.tsx` and
 * `return-view.tsx`.
 */
export function PageShell({ className, ...rest }: PageShellProps) {
  return (
    <div
      className={cn(
        "mx-auto flex w-full max-w-md flex-col gap-6 p-6",
        className,
      )}
      {...rest}
    />
  );
}
