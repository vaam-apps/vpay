import { Card, InstrumentPanel, Skeleton } from "@vaam-apps/ui";

import { PageHeaderSkeleton, TableCardSkeleton } from "./list-skeleton";

/**
 * The Overview while its twelve reads are in flight: the header, the payments
 * panel with six tiles, and the two cards below, in the loaded layout.
 *
 * The tiles are blocks rather than copies: a count is not known until it is
 * read, so only the panel and the grid are claimed to line up. No `<h2>`.
 */
export function OverviewSkeleton() {
  return (
    <div role="status" aria-busy="true" className="flex flex-col gap-6">
      <span className="sr-only">Loading the overview…</span>
      <PageHeaderSkeleton
        title="Overview"
        description="Exact counts from vpay, read at 0000-00-00 00:00:00 UTC. Every number opens the rows it counts."
      />
      <InstrumentPanel title="Payments">
        <div className="grid grid-cols-2 gap-3 md:grid-cols-3">
          {Array.from({ length: 6 }, (_, tile) => (
            <Skeleton key={tile} instrument className="h-28 rounded-xl" />
          ))}
        </div>
      </InstrumentPanel>
      <Card>
        <div className="grid grid-cols-2 gap-3 p-4 md:grid-cols-3 xl:grid-cols-6">
          {Array.from({ length: 6 }, (_, tile) => (
            <Skeleton key={tile} className="h-24 rounded-box" />
          ))}
        </div>
      </Card>
      <TableCardSkeleton rows={6} />
    </div>
  );
}
