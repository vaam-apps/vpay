import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { ABSENT } from '../format';
import { CHARGE, DETAIL, INTENT } from '../testing/fixtures';
import { PaymentDetailView } from './payment-detail';

describe('the payment detail', () => {
  it('renders the intent, the rail and the timeline', () => {
    render(<PaymentDetailView detail={DETAIL} />);
    expect(screen.getByTestId('detail-id')).toHaveTextContent('pi_example_1');
    expect(screen.getByTestId('detail-rail')).toHaveTextContent('mtn_momo');
    expect(screen.getByText('payment_intent.succeeded')).toBeInTheDocument();
  });

  it('renders the masked payer as a dash — the column is never written', () => {
    // THE case this test exists for. `charges.payer_ref_masked` is NULL on
    // every row this deployment has ever stored (docs/status.md); the value
    // is rendered from the column and never derived, because the only other
    // value that could produce a mask is the payer unmasked phone number.
    render(<PaymentDetailView detail={DETAIL} />);
    expect(screen.getByTestId('detail-payer')).toHaveTextContent(ABSENT);
  });

  it('renders a masked payer the day the column is written', () => {
    // The field is here so that value appears, rather than a field having to
    // be added then.
    render(
      <PaymentDetailView
        detail={{ ...DETAIL, charge: { ...CHARGE, payer_ref_masked: '+237 6•• ••• •89' } }}
      />,
    );
    expect(screen.getByTestId('detail-payer')).toHaveTextContent('+237 6•• ••• •89');
  });

  it('says plainly that an unconfirmed intent has no charge', () => {
    render(<PaymentDetailView detail={{ ...DETAIL, charge: null }} />);
    expect(screen.getByText(/No charge/)).toBeInTheDocument();
    expect(screen.queryByTestId('detail-rail')).toBeNull();
  });

  it('renders the failure code AND the sentence', () => {
    // The code is the closed vocabulary a runbook is written against; the
    // message is what a person reads. Showing one makes the other
    // unreachable from this screen.
    render(
      <PaymentDetailView
        detail={{
          ...DETAIL,
          payment_intent: {
            ...INTENT,
            status: 'requires_payment_method',
            last_payment_error: { code: 'insufficient_funds', message: 'The payer had no balance.' },
          },
        }}
      />,
    );
    expect(screen.getByTestId('detail-failure-code')).toHaveTextContent('insufficient_funds');
    expect(screen.getByText('The payer had no balance.')).toBeInTheDocument();
  });

  it('shows no error section when there was no error', () => {
    render(<PaymentDetailView detail={DETAIL} />);
    expect(screen.queryByTestId('detail-failure-code')).toBeNull();
  });

  it('shows no refunds section when there are none', () => {
    render(<PaymentDetailView detail={DETAIL} />);
    expect(screen.queryByRole('heading', { name: 'Refunds' })).toBeNull();
  });

  it('heads its first section "Summary", not "Payment"', () => {
    // The page around this view already heads itself "Payment", and two
    // <h2>Payment</h2> on one screen was visible in the first committed
    // screenshot of it — the kind of thing only looking at the render finds.
    render(<PaymentDetailView detail={DETAIL} />);
    expect(screen.getByRole('heading', { name: 'Summary' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Payment' })).toBeNull();
  });

  it('renders refunds when there are some', () => {
    render(
      <PaymentDetailView
        detail={{
          ...DETAIL,
          refunds: [
            { id: 're_example_1', amount: 5000, currency: 'xaf', status: 'succeeded', reason: null, created: 1_757_000_300 },
          ],
        }}
      />,
    );
    expect(screen.getByText('re_example_1')).toBeInTheDocument();
  });
});
