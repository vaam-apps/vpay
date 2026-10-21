import { PAYMENT_STATUS, statusShortLabel } from "@vpay/tokens";
import {
  Card,
  CardHeader,
  InlineEmptyState,
  InstrumentPanel,
  StatTile,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@vaam-apps/ui";
import { ArrowRight } from "lucide-react";
import NextLink from "next/link";
import type { ReactNode } from "react";

import { OutlinedLink, PageHeader } from "../../../src/components/layout";
import {
  PaymentLinkCell,
  PhoneAmount,
} from "../../../src/components/procedure-table";
import { ReadFailure } from "../../../src/components/read-failure";
import { Amount, IsoInstant } from "../../../src/components/values";
import {
  firstFailure,
  readOverview,
  type Count,
  type Overview,
} from "../../../src/dash/overview";
import { ABSENT, asPaymentStatus, formatIsoInstant } from "../../../src/format";
import { PaymentStatusPill } from "../../../src/payment-status";
import { requireStaff } from "../../../src/server/session";

/**
 * `/overview` — the home screen (`HOME_PATH`, since 2026-10-21).
 *
 * # What it answers, and the one rule it keeps
 *
 * "Is anything wrong for this merchant right now?" — without opening five
 * lists. It answers with counts, and **every count is one vpay computed**:
 * `totalCount` from the search procedures, read in `src/dash/overview.ts`.
 * Nothing here is summed, averaged or estimated, and nothing is a rate: the
 * one place this repository had said "vpay exposes no aggregates" was the
 * payment detail's caption, and that is still true of *sums* — there is no
 * amount-weighted total anywhere on this screen, because no route answers
 * one.
 *
 * Every number is a link to the list it counts, filtered by the same value
 * the count was read with, so the Overview can never show a figure an
 * operator cannot open.
 *
 * # Registers
 *
 * The payment counts are the instrument register (`InstrumentPanel` +
 * `StatTile`): six numbers scanned at a glance. "Needs a look" is diagnostic
 * — outlined tiles, and the only colour on the screen is a `danger` tone on
 * a failure count that is not zero. A wall of coloured tiles would hide the
 * one that matters (`StatTile`'s own guidance).
 */
export const dynamic = "force-dynamic";

export default async function OverviewPage() {
  const gate = await requireStaff();
  if (gate.kind === "outage") {
    return (
      <>
        <PageHeader title="Overview" />
        <ReadFailure failure={gate.failure} />
      </>
    );
  }
  const overview = await readOverview(gate.staff);
  const failure = firstFailure(overview);

  return (
    <>
      <PageHeader
        title="Overview"
        description={
          <>
            Exact counts from vpay, read at{" "}
            <span className="font-mono">
              {formatIsoInstant(overview.readAt)}
            </span>
            . Every number opens the rows it counts.
          </>
        }
      />
      {failure === null ? null : <ReadFailure failure={failure} />}
      <PaymentCounts overview={overview} />
      <NeedsALook overview={overview} />
      <LatestPayments overview={overview} />
    </>
  );
}

function PaymentCounts({ overview }: { readonly overview: Overview }) {
  return (
    <section aria-label="Payments by status">
      <InstrumentPanel
        title="Payments"
        caption="How many payment intents this merchant holds, by status."
      >
        <div className="grid grid-cols-2 gap-3 md:grid-cols-3">
          <CountLink href="/payments">
            <StatTile
              label="All payments"
              value={countText(
                overview.payments.ok
                  ? { ok: true, value: overview.payments.value.total }
                  : overview.payments,
              )}
              caption="every status"
              emphasized
            />
          </CountLink>
          {PAYMENT_STATUS.map((status) => (
            <CountLink key={status} href={`/payments?status=${status}`}>
              <StatTile
                label={statusShortLabel[status]}
                value={countText(overview.byStatus[status])}
                caption={<span className="font-mono">{status}</span>}
              />
            </CountLink>
          ))}
        </div>
      </InstrumentPanel>
    </section>
  );
}

function LatestPayments({ overview }: { readonly overview: Overview }) {
  const payments = overview.payments;
  return (
    <section aria-label="Latest payments">
      <Card className="overflow-hidden">
        <CardHeader
          title="Latest payments"
          headingLevel={3}
          action={
            <OutlinedLink href="/payments">
              View all
              <ArrowRight size={18} aria-hidden="true" />
            </OutlinedLink>
          }
        />
        {!payments.ok ? (
          <div className="px-4 pb-4">
            <InlineEmptyState message="Not read — see the refusal above." />
          </div>
        ) : payments.value.latest.length === 0 ? (
          <div className="px-4 pb-4">
            <InlineEmptyState message="No payments. This merchant has no payment intents yet." />
          </div>
        ) : (
          <Table>
            <caption className="sr-only">Latest payments</caption>
            <TableHeader>
              <TableRow>
                <TableHead scope="col">Payment</TableHead>
                <TableHead scope="col" align="end" hideBelow="sm">
                  Amount
                </TableHead>
                <TableHead scope="col">Status</TableHead>
                <TableHead scope="col" hideBelow="md">
                  Created (UTC)
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {payments.value.latest.map((row) => {
                const status = asPaymentStatus(row.status);
                return (
                  <TableRow key={row.id}>
                    <TableCell>
                      <PaymentLinkCell id={row.id} />
                      <PhoneAmount>
                        <Amount
                          minor={row.amount}
                          currency={row.currency_code}
                        />
                      </PhoneAmount>
                    </TableCell>
                    <TableCell align="end" hideBelow="sm">
                      <Amount minor={row.amount} currency={row.currency_code} />
                    </TableCell>
                    <TableCell>
                      {status === null ? (
                        row.status
                      ) : (
                        <PaymentStatusPill state={status} size="sm" />
                      )}
                    </TableCell>
                    <TableCell hideBelow="md">
                      <IsoInstant value={row.created_at} />
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        )}
      </Card>
    </section>
  );
}

function NeedsALook({ overview }: { readonly overview: Overview }) {
  return (
    <section aria-label="Needs a look" className="min-w-0">
      <Card>
        <CardHeader title="Needs a look" headingLevel={3} />
        <div className="px-4 pb-4">
          <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
            <CountLink href="/refunds?status=failed">
              <StatTile
                label="Failed refunds"
                value={countText(overview.failedRefunds)}
                tone={nonZero(overview.failedRefunds) ? "danger" : undefined}
              />
            </CountLink>
            <CountLink href="/refunds?status=pending">
              <StatTile
                label="Refunds pending"
                value={countText(overview.pendingRefunds)}
              />
            </CountLink>
            <CountLink href="/deliveries?state=failed">
              <StatTile
                label="Webhooks failed"
                value={countText(overview.failedDeliveries)}
                tone={nonZero(overview.failedDeliveries) ? "danger" : undefined}
              />
            </CountLink>
            <CountLink href="/deliveries?state=exhausted">
              <StatTile
                label="Retries exhausted"
                value={countText(overview.exhaustedDeliveries)}
                tone={
                  nonZero(overview.exhaustedDeliveries) ? "danger" : undefined
                }
              />
            </CountLink>
            <CountLink href="/checkouts?status=open">
              <StatTile
                label="Open checkouts"
                value={countText(overview.openCheckouts)}
              />
            </CountLink>
            <CountLink href="/customers">
              <StatTile
                label="Customers"
                value={countText(overview.customers)}
              />
            </CountLink>
          </div>
        </div>
      </Card>
    </section>
  );
}

/**
 * A tile that is a link. The tile keeps its own `<dl>` semantics; the link
 * around it is what a keyboard reaches and what a screen reader announces,
 * named by the tile's label and value together.
 */
function CountLink({
  href,
  children,
}: {
  readonly href: string;
  readonly children: ReactNode;
}) {
  return (
    <NextLink href={href} className="block min-w-0 rounded-xl hover:opacity-90">
      {children}
    </NextLink>
  );
}

/** A count as text, or the dash a refused read is drawn as — never a 0. */
function countText(count: Count): string {
  return count.ok ? count.value.toLocaleString("en-US") : ABSENT;
}

function nonZero(count: Count): boolean {
  return count.ok && count.value > 0;
}
