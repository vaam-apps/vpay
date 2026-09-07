import { Alert, Heading, Stack, StatusBadge, Table, Text } from '@vpay/ui';

import { ABSENT, asPaymentStatus, formatAmount, formatInstant, formatMethods } from '../format';
import type { PaymentDetail } from '../server/api';
import { DetailTimeline } from './detail-timeline';

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
    <Stack direction="column" gap="lg">
      <section>
        {/*
          "Summary" and not "Payment": the page around this view already
          heads itself "Payment", and two <h2>Payment</h2> on one screen was
          visible in the first committed screenshot of it. Sections here are
          named for what they contain, not for the object they are about.
        */}
        <Heading level={2}>Summary</Heading>
        <Table>
          <tbody>
            <tr>
              <th scope="row">Id</th>
              <td>
                <code data-testid="detail-id">{intent.id}</code>
              </td>
            </tr>
            <tr>
              <th scope="row">Status</th>
              <td>
                {status === null ? (
                  <Text as="span">{intent.status}</Text>
                ) : (
                  <StatusBadge status={status} />
                )}
              </td>
            </tr>
            <tr>
              <th scope="row">Amount</th>
              <td>{formatAmount(intent.amount, intent.currency)}</td>
            </tr>
            <tr>
              <th scope="row">Created (UTC)</th>
              <td>{formatInstant(intent.created)}</td>
            </tr>
            <tr>
              <th scope="row">Payment methods offered</th>
              <td>{formatMethods(intent.payment_method_types)}</td>
            </tr>
            <tr>
              <th scope="row">Description</th>
              <td>{intent.description ?? ABSENT}</td>
            </tr>
            <tr>
              <th scope="row">Customer</th>
              <td>{intent.customer === null ? ABSENT : <code>{intent.customer}</code>}</td>
            </tr>
            <tr>
              <th scope="row">Livemode</th>
              <td>{intent.livemode ? 'yes' : 'no'}</td>
            </tr>
          </tbody>
        </Table>
      </section>

      {error === null ? null : (
        <section>
          <Heading level={2}>Last error</Heading>
          {/*
            The failure code AND the sentence. The code is the closed
            vocabulary a runbook is written against; the message is what a
            person reads. Showing only one of them makes the other
            unreachable from this screen.
          */}
          <Alert tone="error" role="status">
            <Text as="span" data-testid="detail-failure-code">
              <code>{error.code ?? ABSENT}</code>
            </Text>
            <Text as="span">{error.message ?? ABSENT}</Text>
          </Alert>
        </section>
      )}

      <section>
        <Heading level={2}>Charge</Heading>
        {charge === null ? (
          <Text tone="muted">
            No charge. Nobody has confirmed this payment intent — one charge per
            intent, forever, and this one has none.
          </Text>
        ) : (
          <Table>
            <tbody>
              <tr>
                <th scope="row">Id</th>
                <td>
                  <code>{charge.id}</code>
                </td>
              </tr>
              <tr>
                <th scope="row">Rail</th>
                <td data-testid="detail-rail">{charge.provider_code}</td>
              </tr>
              <tr>
                <th scope="row">State</th>
                <td>{charge.state}</td>
              </tr>
              <tr>
                <th scope="row">Amount</th>
                <td>{formatAmount(charge.amount, charge.currency)}</td>
              </tr>
              <tr>
                <th scope="row">Payer (masked)</th>
                <td data-testid="detail-payer">{charge.payer_ref_masked ?? ABSENT}</td>
              </tr>
              <tr>
                <th scope="row">vpay reference</th>
                <td>
                  <code>{charge.provider_reference_id}</code>
                </td>
              </tr>
              <tr>
                <th scope="row">Rail transaction</th>
                <td>{charge.provider_txn_id === null ? ABSENT : <code>{charge.provider_txn_id}</code>}</td>
              </tr>
              <tr>
                <th scope="row">Failure</th>
                <td>
                  {charge.failure_code === null && charge.failure_raw === null
                    ? ABSENT
                    : `${charge.failure_code ?? ABSENT} — ${charge.failure_raw ?? ABSENT}`}
                </td>
              </tr>
              <tr>
                <th scope="row">Updated (UTC)</th>
                <td>{formatInstant(charge.updated)}</td>
              </tr>
            </tbody>
          </Table>
        )}
      </section>

      {detail.refunds.length === 0 ? null : (
        <section>
          <Heading level={2}>Refunds</Heading>
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
                    <code>{refund.id}</code>
                  </td>
                  <td>{formatAmount(refund.amount, refund.currency)}</td>
                  <td>{refund.status}</td>
                  <td>{formatInstant(refund.created)}</td>
                </tr>
              ))}
            </tbody>
          </Table>
        </section>
      )}

      <section>
        <Heading level={2}>Timeline</Heading>
        <DetailTimeline
          events={detail.events.map((event) => ({
            id: event.id,
            label: event.type,
            at: formatInstant(event.created),
          }))}
        />
        {/*
          THE TIMELINE IS NOT THE HISTORY, AND SAYING SO IS THE POINT.

          `events.type` is constrained to eight documented types (migration
          `0018`, extended by `0029`), and **five of them are written by
          nothing at all** (`docs/status.md`, "Events written by the worker"):
          settlement writes two and the housekeeping sweep writes one, and
          that is the whole of it.

          So a succeeded payment's timeline has exactly one line on it. An
          operator reading a section headed "Timeline" with one entry
          reasonably concludes that is everything that happened to this
          payment — which is the same failure as an empty table that means
          "the read was refused": a true rendering of an incomplete source,
          presented as complete.

          The sentence is on the SCREEN and not only in the flow document,
          because the person who needs it is reading the screen. It names the
          five missing types rather than hedging, so the day one of them is
          written this line is wrong in a way a grep finds. It names only
          those five — the three that ARE written appear in the list above
          when they happen, and repeating them here would be two elements on
          one screen with the same text.
        */}
        <Text tone="muted" size="xs" data-testid="timeline-gap">
          Five of the eight documented event types are written by nothing —{' '}
          <code>payment_intent.created</code>,{' '}
          <code>payment_intent.processing</code>,{' '}
          <code>payment_intent.canceled</code>, <code>charge.refunded</code>{' '}
          and <code>charge.refund.updated</code> — so this is not the whole
          history of a payment.
        </Text>
      </section>
    </Stack>
  );
}
