/**
 * Pointing the official Stripe SDK at a vpay host.
 *
 * NOT RUNNABLE YET — /v1/* is not implemented, and neither is the
 * client_credentials + private_key_jwt token endpoint this now needs. See
 * ../../docs/status.md and
 * ../../docs/adr/0010-merchant-auth-private-key-jwt.md.
 *
 * This file exists to pin down the compatibility claim, but that claim is
 * narrower than it used to be: the *object model* and *idempotency
 * semantics* are still Stripe-shaped; *authentication* is not
 * (ADR-0010 supersedes the part of ADR-0009 that had kept `/v1` on
 * sk_live_/sk_test_ keys). The Stripe SDK has no built-in support for OAuth2
 * client_credentials or RFC 7523 private_key_jwt — it only knows how to send
 * a fixed string as `Authorization: Bearer <value>`. A real integration
 * needs glue the SDK cannot provide on its own: sign a client assertion,
 * exchange it for a short-lived access token out of band, and hand that
 * token to the SDK in place of an API key — the one place this still lines
 * up, since the SDK sends whatever string it is given as a bearer token.
 * Because the grant issues no refresh token, that token must be refetched
 * and the client rebuilt before every expiry; the SDK has no concept of
 * doing that itself. `fetchAccessToken` below is pseudocode — it has never
 * run against a real vpay, because no vpay token endpoint exists yet.
 */
import Stripe from 'stripe';

/**
 * Sign a client assertion and exchange it for a bearer access token.
 * Pseudocode — no vpay token endpoint exists yet (ADR-0010), and no
 * vpay-provided helper library exists to build the assertion.
 */
async function fetchAccessToken() {
  throw new Error(
    'not implemented — the /v1 OAuth2 token endpoint does not exist yet, see docs/status.md',
  );
}

async function main() {
  const accessToken = await fetchAccessToken();

  const stripe = new Stripe(accessToken, {
    host: process.env.VPAY_HOST ?? 'api.vpay.example',
    protocol: 'https',
    port: 443,
  });

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
