/**
 * Node-side (Cypress `setupNodeEvents`) helper for `checkout.cy.ts`: mints a
 * real PaymentIntent through `@vpay/sdk` against the demo stack, the same
 * way `examples/checkout-browser/mint.mjs` does for a human running the
 * example by hand.
 *
 * Runs outside the browser sandbox `cy.task` exists for exactly this reason
 * — the spec needs a MERCHANT credential (the `demo-merchant` OAuth keypair
 * `just gen-demo-keys` writes to `.e2e/`) to create the intent, and that
 * credential must never reach the page under test: the whole point of
 * `/v1/browser` is that a payer's browser holds only a publishable key and a
 * `client_secret`, never the merchant's private key
 * (`docs/flows/browser-checkout.md`).
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { VpayClient } from "@vpay/sdk";

const here = dirname(fileURLToPath(import.meta.url));
// frontends/tests/e2e/cypress/tasks -> repo root is four levels up.
const repoRoot = join(here, "..", "..", "..", "..", "..");

export interface MintedCheckout {
  id: string;
  status: string;
  clientSecret: string;
  publishableKey: string;
  baseUrl: string;
}

/**
 * Mints a 50.00 EUR `mtn_momo` PaymentIntent and returns everything
 * `checkout.cy.ts` needs to open the example page: the URL it must visit is
 * `${CHECKOUT_BROWSER_URL}/?pk=${publishableKey}&client_secret=${clientSecret}&api=${baseUrl}`.
 */
export async function mintCheckoutPaymentIntent(): Promise<MintedCheckout> {
  const baseUrl = process.env["VPAY_BASE_URL"] ?? "http://localhost:8080";
  const clientId = process.env["VPAY_MERCHANT_CLIENT_ID"] ?? "demo-merchant";
  const privateKeyPath =
    process.env["VPAY_MERCHANT_PRIVATE_KEY_PATH"] ??
    join(repoRoot, ".e2e", "demo-merchant", "oauth-signing-key.pem");
  // Fixed literal `just gen-demo-keys` writes into `.e2e/application-demo.yml`
  // — see that recipe's own comment on why it is fixed rather than generated.
  const publishableKey =
    process.env["CHECKOUT_PUBLISHABLE_KEY"] ?? "pk_test_demomerchantsandbox01";

  let privateKeyPem: string;
  try {
    privateKeyPem = readFileSync(privateKeyPath, "utf8");
  } catch (cause) {
    throw new Error(
      `checkout.cy.ts: cannot read the merchant private key at ${privateKeyPath}. ` +
        `Run \`just gen-demo-keys\` (or \`just demo\`, which does it for you) before ` +
        `running this spec, or set VPAY_MERCHANT_PRIVATE_KEY_PATH.`,
      { cause },
    );
  }

  const vpay = new VpayClient({ baseUrl, clientId, privateKey: privateKeyPem });

  const intent = await vpay.paymentIntents.create(
    {
      // EUR, not XAF: the demo overlay's `mtn_momo` rail settles in EUR
      // (matches `examples/merchant-demo`'s `DEMO_CURRENCY`) — confirmed by
      // running `examples/checkout-browser/mint.mjs` against `just demo` and
      // getting back a real `invalid_request_error/invalid_request: rail
      // 'mtn_momo' settles in EUR; this PaymentIntent is XAF` when this used
      // to say `"xaf"`. See that script's own comment.
      amount: 5000,
      currency: "eur",
      payment_method_types: ["mtn_momo"],
      metadata: { source: "checkout.cy.ts" },
    },
    {
      idempotencyKey: `checkout-cy-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    },
  );

  // `@vpay/sdk`'s `PaymentIntent` type (sdks/nodejs/src/types.ts) declares
  // `client_secret?: string`, matching the server's `create()` response
  // (migration 0026, `vpay_api::model::PaymentIntentWithSecret`, decision
  // D2) — a typed check, no cast needed.
  if (
    typeof intent.client_secret !== "string" ||
    intent.client_secret.length === 0
  ) {
    throw new Error(
      "checkout.cy.ts: the create() response has no client_secret. Either the server no " +
        "longer implements Step 5c's PaymentIntentWithSecret, or sdks/nodejs's response " +
        "handling changed in a way that now filters unknown properties.",
    );
  }

  return {
    id: intent.id,
    status: intent.status,
    clientSecret: intent.client_secret,
    publishableKey,
    baseUrl,
  };
}
