/**
 * Pointing the official Stripe SDK at a vpay host.
 *
 * NOT RUNNABLE YET — /v1/* is not implemented. See ../../docs/STATUS.md.
 *
 * This file exists to pin down the compatibility claim: if this script ever
 * needs a vpay-specific workaround, the API is not Stripe-shaped enough.
 */
import Stripe from 'stripe';

const stripe = new Stripe(process.env.VPAY_SECRET_KEY ?? 'sk_test_placeholder', {
  host: process.env.VPAY_HOST ?? 'api.vpay.example',
  protocol: 'https',
  port: 443,
});

async function main() {
  const intent = await stripe.paymentIntents.create(
    {
      amount: 5000, // 5,000 FCFA — XAF is zero-decimal
      currency: 'xaf',
      payment_method_types: ['mtn_momo'],
      metadata: { order_id: '1234' },
    },
    { idempotencyKey: 'order_1234_attempt_1' },
  );

  console.log('created', intent.id, intent.status);

  const confirmed = await stripe.paymentIntents.confirm(intent.id, {
    // @ts-expect-error — mtn_momo is a vpay rail, absent from Stripe's types
    payment_method_data: { type: 'mtn_momo', mtn_momo: { msisdn: '237670000000' } },
  });

  // `processing` means NOT YET. Wait for payment_intent.succeeded.
  console.log('confirmed', confirmed.status);
}

main().catch((err) => {
  console.error('failed:', err.message);
  process.exitCode = 1;
});
