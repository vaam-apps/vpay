import { Card, InstrumentPanel, Skeleton } from "@vaam-apps/ui";

import { PageHeaderSkeleton } from "./list-skeleton";
import { AT_A_GLANCE_CAPTION } from "./payment-detail";

/**
 * One payment while its read is in flight, in the loaded page's layout: the
 * back link and heading (`PaymentDetailHeader`'s, sized by invisible copies
 * of the same strings), the "At a glance" panel with its real title and
 * caption, and the record and timeline cards side by side from `lg`.
 *
 * The panel's title and caption are the real ones, rendered: they are fixed
 * strings, and the panel only lines up with the loaded one if its header is
 * the same height — the caption wraps to two lines on a phone. The four
 * tiles and the cards below are blocks; their contents are not known until
 * the read returns.
 *
 * No `<h2>`, no `<h3>`, and nothing carries `data-testid="detail-id"`:
 * `dashboard.cy.ts` waits on that as the sign the payment has loaded.
 * `screens.test.tsx` fails if the page's loading branch renders
 * `RouteSkeleton` again.
 */
export function PaymentSkeleton() {
  return (
    <div role="status" aria-busy="true" className="flex flex-col gap-6">
      <span className="sr-only">Loading…</span>
      <PageHeaderSkeleton title="Payment" back="Back to payments" />
      <InstrumentPanel title="At a glance" caption={AT_A_GLANCE_CAPTION}>
        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          {Array.from({ length: 4 }, (_, tile) => (
            <Skeleton key={tile} instrument className="h-28 rounded-xl" />
          ))}
        </div>
      </InstrumentPanel>
      <div className="grid gap-6 lg:grid-cols-3">
        <div className="min-w-0 lg:col-span-2">
          <Card>
            <div className="flex flex-col gap-3 p-4">
              <Skeleton className="h-5 w-28" />
              {Array.from({ length: 8 }, (_, row) => (
                <Skeleton key={row} className="h-9 w-full" />
              ))}
            </div>
          </Card>
        </div>
        <Card>
          <div className="flex flex-col gap-3 p-4">
            <Skeleton className="h-5 w-24" />
            {Array.from({ length: 3 }, (_, row) => (
              <Skeleton key={row} className="h-10 w-full" />
            ))}
          </div>
        </Card>
      </div>
    </div>
  );
}
