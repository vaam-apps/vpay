import {
  Alert,
  Code,
  DataList,
  DataListRow,
  Section,
  Stack,
  StatusBadge,
  Table,
  Text,
  Timeline,
} from "@vpay/ui";

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
 * Composition only — every element here is a `@vpay/ui` primitive. The
 * `<table><tbody><tr><th scope="row">` markup this file wrote by hand twice,
 * twenty rows between them, is `DataList`/`DataListRow` now; the bare
 * `<section>`s that were landmarks in name only are `Section`, which names
 * itself.
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
    <Stack direction="column" align="stretch" gap="lg">
      {/*
        "Summary" and not "Payment": the page around this view already heads
        itself "Payment", and two <h2>Payment</h2> on one screen was visible
        in the first committed screenshot of it. Sections here are named for
        what they contain, not for the object they are about.
      */}
      <Section title="Summary">
        <DataList>
          <DataListRow label="Id">
            <Code wrap="anywhere" data-testid="detail-id">
              {intent.id}
            </Code>
          </DataListRow>
          <DataListRow label="Status">
            {status === null ? (
              <Text as="span">{intent.status}</Text>
            ) : (
              <StatusBadge status={status} />
            )}
          </DataListRow>
          <DataListRow label="Amount">
            {formatAmount(intent.amount, intent.currency)}
          </DataListRow>
          <DataListRow label="Created (UTC)">
            {formatInstant(intent.created)}
          </DataListRow>
          <DataListRow label="Payment methods offered">
            {formatMethods(intent.payment_method_types)}
          </DataListRow>
          <DataListRow label="Description">
            {intent.description ?? ABSENT}
          </DataListRow>
          <DataListRow label="Customer">
            {intent.customer === null ? ABSENT : <Code>{intent.customer}</Code>}
          </DataListRow>
          <DataListRow label="Livemode">
            {intent.livemode ? "yes" : "no"}
          </DataListRow>
        </DataList>
      </Section>

      {error === null ? null : (
        <Section title="Last error">
          {/*
            The failure code AND the sentence. The code is the closed
            vocabulary a runbook is written against; the message is what a
            person reads. Showing only one of them makes the other
            unreachable from this screen.
          */}
          <Alert tone="error" role="status">
            <Text as="span" data-testid="detail-failure-code">
              <Code>{error.code ?? ABSENT}</Code>
            </Text>
            <Text as="span">{error.message ?? ABSENT}</Text>
          </Alert>
        </Section>
      )}

      <Section title="Charge">
        {charge === null ? (
          <Text tone="muted">
            No charge. Nobody has confirmed this payment intent — one charge per
            intent, forever, and this one has none.
          </Text>
        ) : (
          <DataList>
            <DataListRow label="Id">
              <Code wrap="anywhere">{charge.id}</Code>
            </DataListRow>
            <DataListRow label="Rail">
              <span data-testid="detail-rail">{charge.provider_code}</span>
            </DataListRow>
            <DataListRow label="State">{charge.state}</DataListRow>
            <DataListRow label="Amount">
              {formatAmount(charge.amount, charge.currency)}
            </DataListRow>
            <DataListRow label="Payer (masked)">
              <span data-testid="detail-payer">
                {charge.payer_ref_masked ?? ABSENT}
              </span>
            </DataListRow>
            <DataListRow label="vpay reference">
              <Code wrap="anywhere">{charge.provider_reference_id}</Code>
            </DataListRow>
            <DataListRow label="Rail transaction">
              {charge.provider_txn_id === null ? (
                ABSENT
              ) : (
                <Code wrap="anywhere">{charge.provider_txn_id}</Code>
              )}
            </DataListRow>
            <DataListRow label="Failure">
              {charge.failure_code === null && charge.failure_raw === null
                ? ABSENT
                : `${charge.failure_code ?? ABSENT} — ${charge.failure_raw ?? ABSENT}`}
            </DataListRow>
            <DataListRow label="Updated (UTC)">
              {formatInstant(charge.updated)}
            </DataListRow>
          </DataList>
        )}
      </Section>

      {detail.refunds.length === 0 ? null : (
        <Section title="Refunds">
          <Table zebra>
            <thead>
              <tr>
                <th scope="col">Refund</th>
                <th scope="col">Amount</th>
                <th scope="col">Status</th>
                <th scope="col">Created (UTC)</th>
              </tr>
            </thead>
            <tbody>
              {detail.refunds.map((refund) => (
                <tr key={refund.id}>
                  <td>
                    <Code wrap="anywhere">{refund.id}</Code>
                  </td>
                  <td>{formatAmount(refund.amount, refund.currency)}</td>
                  <td>{refund.status}</td>
                  <td>{formatInstant(refund.created)}</td>
                </tr>
              ))}
            </tbody>
          </Table>
        </Section>
      )}

      <Section title="Timeline">
        <Timeline
          emptyMessage="No events yet."
          items={detail.events.map((event) => ({
            id: event.id,
            label: event.type,
            at: formatInstant(event.created),
          }))}
        />
        <TimelineGap />
      </Section>
    </Stack>
  );
}
