/**
 * A merchant backend talking to vpay through `@vaam-apps/vpay-sdk` (sdks/nodejs).
 *
 * Why this and not the Stripe SDK: vpay's object model and idempotency
 * semantics are Stripe-shaped, and its authentication is not — every call is a
 * COSE_Sign1 message signed with your Ed25519 key, and every answer is one
 * signed by vpay (`docs/signed-transport.md`). A Stripe SDK sends a fixed
 * bearer string and cannot do that. `@vaam-apps/vpay-sdk` signs and verifies
 * transparently on every call. A Stripe SDK *can* be made to do it through
 * `config.httpClient`, which is what `@vaam-apps/vpay-sdk/stripe` and
 * `examples/merchant-stripe-node` are for; use that if you already have Stripe
 * integration code, and this if you do not.
 *
 * Written for `merchant_a` against a hypothetical `api.vpay.example`;
 * `examples/merchant-demo` and `examples/merchant-stripe-node` are the two
 * examples that actually run against the compose stack. See
 * ../../docs/flows/merchant-auth.md.
 *
 * Build the SDK first: `pnpm --filter @vaam-apps/vpay-sdk build` (it compiles
 * the sealing module from `sdks/rust/wasm`, so it needs Rust and
 * `wasm-bindgen-cli`; `just wasm-build` explains what is missing).
 */
import { readFileSync } from "node:fs";
import { VpayClient, VpayError } from "@vaam-apps/vpay-sdk";

const vpay = new VpayClient({
  baseUrl: process.env.VPAY_BASE_URL ?? "https://api.vpay.example",
  // `merchant_a` is the client_id vpay registered from your YAML config PR.
  // It is not sent: vpay finds you by the key that signed (`cose_keys`).
  clientId: process.env.VPAY_CLIENT_ID ?? "merchant_a",
  // Your Ed25519 signing seed (32 bytes, unpadded base64url — what
  // `cargo xtask gen-envelope-key` writes). It never leaves your systems; vpay
  // holds only the public half.
  privateKey: readFileSync(
    process.env.VPAY_SIGNING_SEED_FILE ?? "./merchant-a.seed",
    "utf8",
  ),
  // The response keys vpay publishes (it logs them at boot): the one it signs
  // answers with now, and the one it will after the next rotation. Answers
  // sealed by anything else are refused.
  serverKeys: {
    current: readFileSync(
      process.env.VPAY_SERVER_KEY_CURRENT_FILE ?? "./vpay-current.pub",
      "utf8",
    ).trim(),
    next: readFileSync(
      process.env.VPAY_SERVER_KEY_NEXT_FILE ?? "./vpay-next.pub",
      "utf8",
    ).trim(),
  },
  // What the signature is addressed to: vpay's configured public base URL.
  // Only needed when you reach vpay by another name than payers do.
  audience: process.env.VPAY_AUDIENCE,
});

// Defaults are this example's original merchant_a values. Override them to
// drive a real sandbox, as docs/runbooks/notchpay-sandbox-test.md does:
// VPAY_RAIL=notchpay VPAY_PAYER_MSISDN=237670000000 VPAY_ORDER=<unique>.
// VPAY_ORDER keeps the idempotency keys fresh, so a second run creates a new
// intent instead of replaying the first.
const rail = process.env.VPAY_RAIL ?? "mtn_momo";
const currency = process.env.VPAY_CURRENCY ?? "xaf";
const msisdn = process.env.VPAY_PAYER_MSISDN ?? "237670000000";
const order = process.env.VPAY_ORDER ?? "order_1234";
const amount = Number(process.env.VPAY_AMOUNT ?? "5000");

async function main() {
  const intent = await vpay.paymentIntents.create(
    {
      amount, // 5,000 FCFA by default — XAF is zero-decimal, integer minor units
      currency,
      payment_method_types: [rail],
      metadata: { order_id: order },
    },
    { idempotencyKey: `${order}_attempt_1` },
  );
  console.log("created", intent.id, intent.status);

  const confirmed = await vpay.paymentIntents.confirm(
    intent.id,
    {
      payment_method_data: {
        type: rail,
        [rail]: { msisdn },
      },
    },
    { idempotencyKey: `${order}_confirm_1` },
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
