import { PageHeaderSkeleton, TableCardSkeleton } from "./list-skeleton";
import { PaymentsFiltersSkeleton } from "./payments-filters";

/**
 * The line under the payments screen's heading. One string, used by the
 * loaded screen and by the placeholder below, so the placeholder wraps
 * where the text will.
 */
export const PAYMENTS_DESCRIPTION =
  "Every payment intent this merchant has created, newest first.";

/**
 * The payments screen while its read is in flight: the header, the filter
 * row, and a table card, each the shape of the loaded one.
 *
 * Built from the shared placeholders in `list-skeleton.tsx`, which size
 * every block by an invisible copy of the real text rather than a guessed
 * width. That is the lesson this file's earlier versions recorded at
 * length (2026-09-24, vaam-apps/vpay#258): `RouteSkeleton`'s fixed blocks
 * sat 20–40 px off the real heading and 30–116 px short of the filter row
 * at the widths measured, and the table jumped when the rows arrived.
 *
 * Held by the `PaymentsLoading` stories, which compare the header and
 * filter row box by box with the loaded screen's. No `<h2>` is rendered:
 * `dashboard.cy.ts` waits on `cy.contains("h2", "Payments")`.
 */
export function PaymentsSkeleton() {
  return (
    <div role="status" aria-busy="true" className="flex flex-col gap-6">
      <span className="sr-only">Loading…</span>
      <PageHeaderSkeleton title="Payments" description={PAYMENTS_DESCRIPTION} />
      <PaymentsFiltersSkeleton />
      <TableCardSkeleton />
    </div>
  );
}
