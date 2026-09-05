/**
 * A merchant backend talking to vpay through `@vaam-apps/vpay-sdk` (sdks/nodejs).
 *
 * CORRECTED 2026-09-03. This header used to say "/v1/* is not implemented,
 * and neither is the client_credentials + private_key_jwt token endpoint" —
 * true on 2026-09-02, false since. Both exist: the merchant OP is served and
 * so are the five payment-intent routes, and a confirm reaches a rail. What
 * is still true is that **no test in this file's package has run it against
 * a vpay**: it is written for `merchant_a` against a hypothetical
 * `api.vpay.example`, and the SDK's own tests use an HTTP stub of the wire
 * contract. `examples/merchant-demo` and `examples/merchant-stripe-node` are
 * the two examples that actually run against the compose stack. See
 * ../../docs/status.md and ../../docs/flows/merchant-auth.md.
 *
 * Why this and not the Stripe SDK: vpay's object model and idempotency
 * semantics are Stripe-shaped, and its authentication is not (ADR-0010) — a
 * Stripe SDK sends a fixed bearer string and cannot sign an RFC 7523 client
 * assertion. `@vaam-apps/vpay-sdk` does the handshake transparently on every call. A
 * Stripe SDK *can* be made to do it through `config.authenticator`, which is
 * what `@vaam-apps/vpay-sdk/stripe` and `examples/merchant-stripe-node` are for; use
 * that if you already have Stripe integration code, and this if you do not.
 *
 * Build the SDK first: `pnpm --filter @vaam-apps/vpay-sdk build`.
 */
import { readFileSync } from "node:fs";
import { VpayClient, VpayError } from "@vaam-apps/vpay-sdk";

const vpay = new VpayClient({
  baseUrl: process.env.VPAY_BASE_URL ?? "https://api.vpay.example",
  // `merchant_a` is the client_id vpay registered from your YAML config PR;
  // the private key never leaves your systems — vpay holds only the public
  // JWK. Register more than one key and you must also pass `kid`.
  clientId: process.env.VPAY_CLIENT_ID ?? "merchant_a",
  privateKey: readFileSync(
    process.env.VPAY_PRIVATE_KEY_FILE ?? "./merchant-a-private-key.pem",
    "utf8",
  ),
  kid: process.env.VPAY_KID,
});

async function main() {
  const intent = await vpay.paymentIntents.create(
    {
      amount: 5000, // 5,000 FCFA — XAF is zero-decimal, integer minor units
      currency: "xaf",
      payment_method_types: ["mtn_momo"],
      metadata: { order_id: "1234" },
    },
    { idempotencyKey: "order_1234_attempt_1" },
  );
  console.log("created", intent.id, intent.status);

  const confirmed = await vpay.paymentIntents.confirm(
    intent.id,
    {
      payment_method_data: {
        type: "mtn_momo",
        mtn_momo: { msisdn: "237670000000" },
      },
    },
    { idempotencyKey: "order_1234_confirm_1" },
  );

  // `processing` means NOT YET. The payer has a prompt on their handset;
  // wait for the payment_intent.succeeded webhook (see ../webhook-receiver).
  console.log("confirmed", confirmed.status);
}

main().catch((err) => {
  if (err instanceof VpayError) {
    console.error(`${err.name}: ${err.message}`);
  } else {
    console.error("failed:", err);
  }
  process.exitCode = 1;
});
