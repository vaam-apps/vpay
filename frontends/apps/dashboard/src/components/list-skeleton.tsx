import { ScreenStack, Skeleton } from "@vaam-apps/ui";

/**
 * What a list screen shows between a click and its rows: the heading, then
 * table-row-shaped blocks, inside the shell that stays mounted.
 *
 * Rendered by each list route's `loading.tsx`. Before 2026-10-21 those
 * routes had no loading boundary at all, so following a link left the
 * previous screen on display, unchanged and still clickable, until the next
 * one had been read in full on the server — nothing said the click had
 * landed.
 */
export function ListSkeleton({ title }: { readonly title: string }) {
  return (
    <ScreenStack role="status" aria-busy="true">
      <span className="sr-only">Loading {title}…</span>
      <Skeleton className="h-lh w-32 text-title-large" />
      <div className="flex flex-col gap-2">
        {Array.from({ length: 8 }, (_, row) => (
          <Skeleton key={row} className="h-12 w-full" />
        ))}
      </div>
    </ScreenStack>
  );
}
