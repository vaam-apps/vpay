import {
  Badge,
  Code,
  CopyButton,
  DetailList,
  DetailRow,
  InlineBanner,
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

import { PaymentStatusPill } from "../payment-status";
import { ABSENT, asPaymentStatus } from "../format";
import type { PaymentDetail } from "../server/api";
import { asRefundStatus, RefundStatusPill } from "../status-systems";
import { PageHeader, Section } from "./layout";
import { TimelineGap } from "./timeline-gap";
import { Amount, Instant } from "./values";

export interface PaymentDetailViewProps {
  detail: PaymentDetail;
}

/**
 * The line under the "At a glance" panel's title. One string, used by the
 * panel and by `PaymentSkeleton`'s placeholder for it, so the placeholder
 * wraps where the caption will: one line on a desktop, two on a phone.
 */
export const AT_A_GLANCE_CAPTION =
  "This payment only — vpay exposes no aggregates, so nothing here is a total.";

/**
 * The detail page's own header: the way back to the list, then its heading.
 *
 * A plain `<h2>`, as on every screen — see `layout.tsx`. No Cypress
 * assertion finds this one by its text: `dashboard.cy.ts` waits on
 * `data-testid="detail-id"` for this page. A component rather than markup
 * in `payment-screen.tsx` so `PaymentSkeleton` is compared with the header
 * the page really renders, not with a copy of it that could drift.
 */
export function PaymentDetailHeader() {
  return (
    <PageHeader
      title="Payment"
      back={{ href: "/payments", label: "Back to payments" }}
    />
  );
}

/**
 * Everything `/dash/v1` knows about one payment.
 *
 * Four sections, in the order an operator asks the questions: what is this
 * and what state is it in, why did it fail if it did, which rail took it and
 * what did the rail say, and what happened in what order.
 *
 * # Layout (2026-10-21, the Material 3 Expressive revamp)
 *
 * The "At a glance" instrument across the top; the last error, when there is
 * one, directly under it, because it is the answer to the question most
 * people open a failed payment with; then two columns from `lg` — the
 * record (Summary, Charge, Refunds) on the left and the Timeline beside it,
 * so "what is this" and "what happened, in what order" are read side by
 * side instead of one below the other at the foot of a long page. Each
 * section is a `Section` (a named `<section>` around a `Card`), and the
 * section headings are `<h3>` under the page's `<h2>`.
 *
 * The timeline is an `<ol>` with a rule and a dot per event rather than
 * `StateTimeline`: there is no state-machine history to map onto that
 * component's shape — events are written for terminal transitions only, and
 * `TimelineGap` says so beside them.
 *
 * # `Payer` is a dash, and that is a fact rather than a placeholder
 *
 * `charges.payer_ref_masked` is **never written**: the confirm path stores
 * `None` in it and in the unmasked `payer_ref` (`vpay_api::v1::payment_intents`'
 * `open_attempt`, listed as a gap in `docs/status.md`). The field is rendered
 * from the column and shown as {@link ABSENT} when it is null — never derived
 * from anything else, because the only other value that could produce a mask
 * is the unmasked phone number, and deriving one here would read a payer's
 * full number into a staff surface to make a row look populated.
 *
 * The row is here rather than omitted so that the day the column is written
 * the value appears, instead of a field having to be added then. What must
 * not happen is this quietly becoming the unmasked value because the masked
 * one was empty.
 */
export function PaymentDetailView({ detail }: PaymentDetailViewProps) {
  const intent = detail.payment_intent;
  const status = asPaymentStatus(intent.status);
  const charge = detail.charge;
  const error = intent.last_payment_error;

  return (
    <>
      {/*
        The instrument register, and the only surface on this page that is
        not diagnostic. `@vaam-apps/ui`'s "Surfaces and registers" doc draws
        the line by what the reader is doing: you *read* a detail list row
        by row looking for the one that is wrong, and you *scan* an
        instrument to find one number at a glance. The two questions an
        operator opens this page with — how much, and did it move — were
        rows 3 and 2 of a seventeen-row read before this panel existed.

        Four real fields off one object and no arithmetic. The caption says
        on the screen that nothing here is a total.

        `StatTile`s since 2026-10-21: the panel's nested tonal containers.
        Not `emphasized`: the hero size does not fit a seven-digit amount
        and its currency in a quarter of the panel, and an amount must never
        be clipped.
      */}
      <InstrumentPanel title="At a glance" caption={AT_A_GLANCE_CAPTION}>
        <div
          className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4"
          data-testid="at-a-glance"
        >
          <StatTile
            label="Amount"
            value={<Amount minor={intent.amount} currency={intent.currency} />}
            caption="authorised"
          />
          <StatTile
            label="Status"
            value={
              status === null ? (
                intent.status
              ) : (
                <span className="font-sans">
                  <PaymentStatusPill state={status} />
                </span>
              )
            }
            caption={<span className="font-mono">{intent.status}</span>}
          />
          <StatTile
            label="Charged"
            value={
              charge === null ? (
                ABSENT
              ) : (
                <Amount minor={charge.amount} currency={charge.currency} />
              )
            }
            caption={charge === null ? "one charge per intent" : charge.state}
          />
          <StatTile
            label="Rail"
            value={charge === null ? ABSENT : charge.provider_code}
            caption={charge === null ? "no charge yet" : "took the charge"}
          />
        </div>
      </InstrumentPanel>

      {error === null ? null : (
        <section aria-label="Last error">
          {/*
            The failure code AND the sentence. The code is the closed
            vocabulary a runbook is written against; the message is what a
            person reads. Showing only one of them makes the other
            unreachable from this screen.
          */}
          <div role="status">
            <InlineBanner variant="danger">
              <span className="block text-label-large">Last error</span>
              <span data-testid="detail-failure-code">
                <Code>{error.code ?? ABSENT}</Code>
              </span>{" "}
              <span>{error.message ?? ABSENT}</span>
            </InlineBanner>
          </div>
        </section>
      )}

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="grid min-w-0 gap-6 lg:col-span-2 xl:grid-cols-2">
          {/*
            "Summary" and not "Payment": the page around this view already
            heads itself "Payment". Sections here are named for what they
            contain, not for the object they are about.
          */}
          <Section title="Summary">
            <DetailList variant="divided">
              <DetailRow label="Id" variant="divided">
                <span className="inline-flex items-center gap-2">
                  <span data-testid="detail-id">
                    <Code>{intent.id}</Code>
                  </span>
                  <CopyButton value={intent.id} label="Copy payment id" />
                </span>
              </DetailRow>
              <DetailRow label="Status" variant="divided">
                {/*
                  The wire value beside the pill rather than inside it
                  (`showLiteral`): inside, the pill cannot wrap, and at
                  375px "Awaiting payment method requires_payment_method"
                  pushed the whole page sideways.
                */}
                <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
                  {status === null ? (
                    <span>{intent.status}</span>
                  ) : (
                    <PaymentStatusPill state={status} />
                  )}
                  <span className="font-mono text-body-small text-muted-foreground">
                    {intent.status}
                  </span>
                </span>
              </DetailRow>
              <DetailRow label="Amount" variant="divided">
                <Amount minor={intent.amount} currency={intent.currency} />
              </DetailRow>
              <DetailRow label="Created (UTC)" variant="divided">
                <Instant seconds={intent.created} />
              </DetailRow>
              <DetailRow label="Payment methods offered" variant="divided">
                {intent.payment_method_types.length === 0 ? (
                  ABSENT
                ) : (
                  <span className="flex flex-wrap gap-1">
                    {intent.payment_method_types.map((method) => (
                      <Badge key={method} variant="outline">
                        {method}
                      </Badge>
                    ))}
                  </span>
                )}
              </DetailRow>
              <DetailRow label="Description" variant="divided">
                {intent.description ?? ABSENT}
              </DetailRow>
              <DetailRow label="Customer" variant="divided">
                {intent.customer === null ? (
                  ABSENT
                ) : (
                  <Code>{intent.customer}</Code>
                )}
              </DetailRow>
              <DetailRow label="Livemode" variant="divided">
                {intent.livemode ? "yes" : "no"}
              </DetailRow>
            </DetailList>
          </Section>

          <Section title="Charge">
            {charge === null ? (
              <p className="text-body text-muted-foreground">
                No charge. Nobody has confirmed this payment intent — one charge
                per intent, forever, and this one has none.
              </p>
            ) : (
              <DetailList variant="divided">
                <DetailRow label="Id" variant="divided">
                  <Code>{charge.id}</Code>
                </DetailRow>
                <DetailRow label="Rail" variant="divided">
                  <span data-testid="detail-rail">{charge.provider_code}</span>
                </DetailRow>
                <DetailRow label="State" variant="divided">
                  {charge.state}
                </DetailRow>
                <DetailRow label="Amount" variant="divided">
                  <Amount minor={charge.amount} currency={charge.currency} />
                </DetailRow>
                <DetailRow label="Payer (masked)" variant="divided">
                  <span data-testid="detail-payer">
                    {charge.payer_ref_masked ?? ABSENT}
                  </span>
                </DetailRow>
                <DetailRow label="vpay reference" variant="divided">
                  <Code>{charge.provider_reference_id}</Code>
                </DetailRow>
                <DetailRow label="Rail transaction" variant="divided">
                  {charge.provider_txn_id === null ? (
                    ABSENT
                  ) : (
                    <Code>{charge.provider_txn_id}</Code>
                  )}
                </DetailRow>
                <DetailRow label="Failure" variant="divided">
                  {charge.failure_code === null && charge.failure_raw === null
                    ? ABSENT
                    : `${charge.failure_code ?? ABSENT} — ${charge.failure_raw ?? ABSENT}`}
                </DetailRow>
                <DetailRow label="Updated (UTC)" variant="divided">
                  <Instant seconds={charge.updated} />
                </DetailRow>
              </DetailList>
            )}
          </Section>

          {detail.refunds.length === 0 ? null : (
            <div className="min-w-0 xl:col-span-2">
              <Section title="Refunds" flush>
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead scope="col">Refund</TableHead>
                      <TableHead scope="col" align="end">
                        Amount
                      </TableHead>
                      <TableHead scope="col">Status</TableHead>
                      <TableHead scope="col" hideBelow="sm">
                        Created (UTC)
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {detail.refunds.map((refund) => {
                      const refundStatus = asRefundStatus(refund.status);
                      return (
                        <TableRow key={refund.id}>
                          <TableCell>
                            <Code>{refund.id}</Code>
                          </TableCell>
                          <TableCell align="end">
                            <Amount
                              minor={refund.amount}
                              currency={refund.currency}
                            />
                          </TableCell>
                          <TableCell>
                            {refundStatus === null ? (
                              refund.status
                            ) : (
                              <RefundStatusPill
                                state={refundStatus}
                                size="sm"
                              />
                            )}
                          </TableCell>
                          <TableCell hideBelow="sm">
                            <Instant seconds={refund.created} />
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </Section>
            </div>
          )}
        </div>

        <Section title="Timeline">
          <div className="flex flex-col gap-4">
            {detail.events.length === 0 ? (
              <InlineEmptyState message="No events yet." />
            ) : (
              <ol className="ml-1 flex flex-col gap-5 border-l border-edge">
                {detail.events.map((event) => (
                  <li key={event.id} className="relative pl-5">
                    <span
                      aria-hidden="true"
                      className="absolute top-1.5 -left-[5px] flex"
                    >
                      <span className="size-2.5 rounded-full bg-primary" />
                    </span>
                    <span className="block font-mono text-body-medium">
                      {event.type}
                    </span>{" "}
                    <span className="text-body-small">
                      <Instant seconds={event.created} />
                    </span>
                  </li>
                ))}
              </ol>
            )}
            <TimelineGap />
          </div>
        </Section>
      </div>
    </>
  );
}
