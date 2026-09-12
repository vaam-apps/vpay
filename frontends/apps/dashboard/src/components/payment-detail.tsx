import {
  Code,
  DetailList,
  DetailRow,
  InlineBanner,
  InlineEmptyState,
  InstrumentPanel,
  ScreenStack,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@vaam-apps/ui";

import { PaymentStatusPill } from "../payment-status";
import {
  ABSENT,
  asPaymentStatus,
  formatAmount,
  formatInstant,
  formatMethods,
} from "../format";
import type { PaymentDetail } from "../server/api";
import { TimelineGap } from "./timeline-gap";

export interface PaymentDetailViewProps {
  detail: PaymentDetail;
}

/**
 * Everything `/dash/v1` knows about one payment.
 *
 * Four sections, in the order an operator asks the questions: what is this
 * and what state is it in, why did it fail if it did, which rail took it and
 * what did the rail say, and what happened in what order.
 *
 * Composition of `@vaam-apps/ui` primitives where one exists — `DetailList`/
 * `DetailRow` for the `<table><tbody><tr><th scope="row">` markup this file
 * wrote by hand twice, twenty rows between them — and plain semantic HTML
 * where none does: the four `<section>` landmarks (`Section` has no
 * `@vaam-apps/ui` counterpart) and the timeline's `<ul>`/`<li>` (there is no
 * event history to map onto `StateTimeline`'s state-machine shape — see
 * `payment-status.ts` and the timeline section below).
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
    <ScreenStack>
      {/*
        "Summary" and not "Payment": the page around this view already heads
        itself "Payment", and two <h2>Payment</h2> on one screen was visible
        in the first committed screenshot of it. Sections here are named for
        what they contain, not for the object they are about.
      */}
      {/*
        The instrument register, and the only surface on this page that is
        not diagnostic.
        `@vaam-apps/ui`'s "Surfaces and registers" doc draws the line by what
        the reader is doing: you *read* a detail list row by row looking for
        the one that is wrong, and you *scan* an instrument to find one
        number at a glance. Everything below is the former and keeps its
        hairline. The two questions an operator opens this page with — how
        much, and did it move — were rows 3 and 2 of a seventeen-row read,
        rendered identically to "Livemode: no".

        Three real fields off one object and no arithmetic. **vpay exposes no
        aggregate anywhere** (`/dash/v1` is a cursor-paged list and a
        get-by-id; there is no count, total or summary route), so a panel of
        the kind that doc illustrates — delivery rates, spend across
        providers — cannot be built honestly here, and inventing one is the
        failure `CLAUDE.md` names third. The caption says so on the screen
        rather than only here.

        No `text-subtle-foreground` inside: the doc measures it at 4.41:1 on
        this mesh, below AA, and that is the one tier the ground swallows.
        `muted` is what the panel renders its own caption at and is what the
        figures below use.
      */}
      <InstrumentPanel
        title="At a glance"
        caption="This payment only — vpay exposes no aggregates, so nothing here is a total."
      >
        <div className="flex flex-wrap gap-8" data-testid="at-a-glance">
          <div>
            <p className="text-caption text-muted-foreground">Amount</p>
            <p className="font-mono text-metric font-semibold">
              {formatAmount(intent.amount, intent.currency)}
            </p>
            <p className="text-caption text-muted-foreground">authorised</p>
          </div>
          <div>
            <p className="text-caption text-muted-foreground">Status</p>
            {status === null ? (
              <p className="text-metric">{intent.status}</p>
            ) : (
              <PaymentStatusPill state={status} />
            )}
            <p className="text-caption text-muted-foreground">
              {charge === null ? "no charge yet" : charge.provider_code}
            </p>
          </div>
          <div>
            <p className="text-caption text-muted-foreground">Charged</p>
            <p className="font-mono text-metric font-semibold">
              {charge === null
                ? ABSENT
                : formatAmount(charge.amount, charge.currency)}
            </p>
            <p className="text-caption text-muted-foreground">
              {charge === null ? "one charge per intent" : charge.state}
            </p>
          </div>
        </div>
      </InstrumentPanel>

      <section aria-label="Summary">
        <h2>Summary</h2>
        <DetailList variant="divided">
          <DetailRow label="Id" variant="divided">
            <span data-testid="detail-id">
              <Code>{intent.id}</Code>
            </span>
          </DetailRow>
          <DetailRow label="Status" variant="divided">
            {status === null ? (
              <span>{intent.status}</span>
            ) : (
              <PaymentStatusPill state={status} showLiteral />
            )}
          </DetailRow>
          <DetailRow label="Amount" variant="divided">
            {formatAmount(intent.amount, intent.currency)}
          </DetailRow>
          <DetailRow label="Created (UTC)" variant="divided">
            {formatInstant(intent.created)}
          </DetailRow>
          <DetailRow label="Payment methods offered" variant="divided">
            {formatMethods(intent.payment_method_types)}
          </DetailRow>
          <DetailRow label="Description" variant="divided">
            {intent.description ?? ABSENT}
          </DetailRow>
          <DetailRow label="Customer" variant="divided">
            {intent.customer === null ? ABSENT : <Code>{intent.customer}</Code>}
          </DetailRow>
          <DetailRow label="Livemode" variant="divided">
            {intent.livemode ? "yes" : "no"}
          </DetailRow>
        </DetailList>
      </section>

      {error === null ? null : (
        <section aria-label="Last error">
          <h2>Last error</h2>
          {/*
            The failure code AND the sentence. The code is the closed
            vocabulary a runbook is written against; the message is what a
            person reads. Showing only one of them makes the other
            unreachable from this screen.
          */}
          <div role="status">
            <InlineBanner variant="danger">
              <span data-testid="detail-failure-code">
                <Code>{error.code ?? ABSENT}</Code>
              </span>{" "}
              <span>{error.message ?? ABSENT}</span>
            </InlineBanner>
          </div>
        </section>
      )}

      <section aria-label="Charge">
        <h2>Charge</h2>
        {charge === null ? (
          <p>
            No charge. Nobody has confirmed this payment intent — one charge per
            intent, forever, and this one has none.
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
              {formatAmount(charge.amount, charge.currency)}
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
              {formatInstant(charge.updated)}
            </DetailRow>
          </DetailList>
        )}
      </section>

      {detail.refunds.length === 0 ? null : (
        <section aria-label="Refunds">
          <h2>Refunds</h2>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead scope="col">Refund</TableHead>
                <TableHead scope="col">Amount</TableHead>
                <TableHead scope="col">Status</TableHead>
                <TableHead scope="col">Created (UTC)</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {detail.refunds.map((refund) => (
                <TableRow key={refund.id}>
                  <TableCell>
                    <Code>{refund.id}</Code>
                  </TableCell>
                  <TableCell>
                    {formatAmount(refund.amount, refund.currency)}
                  </TableCell>
                  <TableCell>{refund.status}</TableCell>
                  <TableCell>{formatInstant(refund.created)}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </section>
      )}

      <section aria-label="Timeline">
        <h2>Timeline</h2>
        {detail.events.length === 0 ? (
          <InlineEmptyState message="No events yet." />
        ) : (
          <ul>
            {detail.events.map((event) => (
              <li key={event.id}>
                <span>{event.type}</span>{" "}
                <span>{formatInstant(event.created)}</span>
              </li>
            ))}
          </ul>
        )}
        <TimelineGap />
      </section>
    </ScreenStack>
  );
}
