/**
 * A merchant backend talking to vpay through `@vpay/sdk` (sdks/nodejs).
 *
 * NOT RUNNABLE AGAINST A REAL VPAY YET — /v1/* is not implemented, and
 * neither is the client_credentials + private_key_jwt token endpoint this
 * SDK speaks to. See ../../docs/status.md and
 * ../../docs/flows/merchant-auth.md (the wire contract the SDK implements).
 * What this file shows is the shape of a real integration; the SDK itself is
 * tested against an HTTP stub of that contract, not against vpay.
 *
 * Why not the Stripe SDK: vpay's object model and idempotency semantics are
 * Stripe-shaped, but its authentication is not (ADR-0010) — a Stripe SDK can
 * only send a fixed bearer string, and cannot sign an RFC 7523 client
 * assertion or refresh a short-lived access token. `@vpay/sdk` does both,
 * transparently, on every call.
 *
 * Build the SDK first: `pnpm --filter @vpay/sdk build`.
 */
import { readFileSync } from "node:fs";
import { VpayClient, VpayError } from "@vpay/sdk";

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
