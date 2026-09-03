#!/usr/bin/env node
/**
 * Mints a PaymentIntent with `@vpay/sdk` and prints a URL that opens this
 * example ready to confirm it — the "server" half of the integration this
 * package's own README (`../../sdks/stripe-js/README.md`) describes:
 *
 *   const intent = await vpay.paymentIntents.create({...});
 *   res.render("checkout", { publishableKey, clientSecret: intent.client_secret });
 *
 * Deliberately plain JavaScript, not TypeScript — same reason
 * `examples/merchant-node/index.mjs` is plain JS: no build step for a
 * standalone example. `@vpay/sdk`'s `PaymentIntent` type
 * (`sdks/nodejs/src/types.ts`) now declares `client_secret?: string`,
 * matching the server's `create`/`retrieve` responses since migration
 * `0026` (`vpay_api::model::PaymentIntentWithSecret`, decision D2) — so
 * `intent.client_secret` below is typed, not a cast, for a TypeScript
 * caller too. It is absent (`undefined`), not `null`, on a `list()` item
 * or `event.data.object`, which never carry it.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { VpayClient, VpayError } from "@vpay/sdk";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..");

const baseUrl = process.env.VPAY_BASE_URL ?? "http://localhost:8080";
const clientId = process.env.VPAY_MERCHANT_CLIENT_ID ?? "demo-merchant";
const privateKeyPath =
  process.env.VPAY_MERCHANT_PRIVATE_KEY_PATH ??
  join(repoRoot, ".e2e", "demo-merchant", "oauth-signing-key.pem");
// Fixed, not read off the config file: `just gen-demo-keys` writes this
// exact literal into `.e2e/application-demo.yml` (see that recipe's
// comment on why it is fixed rather than generated), and this script exists
// for the same demo stack that overlay registers.
const publishableKey =
  process.env.CHECKOUT_PUBLISHABLE_KEY ?? "pk_test_demomerchantsandbox01";
const checkoutPort = process.env.CHECKOUT_BROWSER_PORT ?? "4180";

let privateKeyPem;
try {
  privateKeyPem = readFileSync(privateKeyPath, "utf8");
} catch (cause) {
  console.error(
    `checkout-browser/mint: cannot read the merchant private key at ${privateKeyPath}.\n` +
      `Run \`just gen-demo-keys\` first (or \`just demo\`, which does it for you), or point\n` +
      `VPAY_MERCHANT_PRIVATE_KEY_PATH at a different merchant's key.`,
  );
  process.exitCode = 1;
  throw cause;
}

const vpay = new VpayClient({
  baseUrl,
  clientId,
  privateKey: privateKeyPem,
});

async function main() {
  const idempotencyKey = `checkout-browser-mint-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const intent = await vpay.paymentIntents.create(
    {
      // 50.00 EUR — matches `examples/merchant-demo`'s own `DEMO_AMOUNT`/
      // `DEMO_CURRENCY`. Confirmed the hard way, running this script against
      // `just demo`: the `mtn_momo` rail in the demo overlay's provider
      // config settles in EUR, and a `currency: "xaf"` create here produced
      // a real refusal on confirm — `invalid_request_error/invalid_request:
      // rail 'mtn_momo' settles in EUR; this PaymentIntent is XAF` — rather
      // than a silent mismatch. `docs/flows/money.md` covers XAF vs. EUR
      // minor units generally; this constant just has to match the rail this
      // demo stack actually configures.
      amount: 5000,
      currency: "eur",
      payment_method_types: ["mtn_momo"],
      metadata: { source: "examples/checkout-browser" },
    },
    { idempotencyKey },
  );

  const clientSecret = intent.client_secret;
  if (typeof clientSecret !== "string" || clientSecret.length === 0) {
    throw new Error(
      "checkout-browser/mint: the create() response has no client_secret. Either the server " +
        "does not implement Step 5c's PaymentIntentWithSecret, or this is a list/event " +
        "payload rather than a create()/retrieve() response (see this file's header comment).",
    );
  }

  const url = new URL(`http://localhost:${checkoutPort}/`);
  url.searchParams.set("pk", publishableKey);
  url.searchParams.set("client_secret", clientSecret);
  url.searchParams.set("api", baseUrl);

  console.log(`created ${intent.id} (${intent.status})`);
  console.log();
  console.log("open this in a browser:");
  console.log(`  ${url.toString()}`);
}

main().catch((err) => {
  if (err instanceof VpayError) {
    console.error(`${err.name}: ${err.message}`);
  } else {
    console.error("failed:", err);
  }
  process.exitCode = 1;
});
