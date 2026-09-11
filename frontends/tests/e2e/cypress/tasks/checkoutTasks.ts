/**
 * Node-side (Cypress `setupNodeEvents`) helper for `checkout.cy.ts`: mints a
 * real PaymentIntent through `@vaam-apps/vpay-sdk` against the demo stack, the same
 * way `examples/checkout-browser/mint.mjs` does for a human running the
 * example by hand.
 *
 * Runs outside the browser sandbox `cy.task` exists for exactly this reason
 * — the spec needs a MERCHANT credential (the `shop-merchant` OAuth keypair
 * `just gen-demo-keys` writes to `.e2e/`) to create the intent, and that
 * credential must never reach the page under test: the whole point of
 * `/v1/browser` is that a payer's browser holds only a publishable key and a
 * `client_secret`, never the merchant's private key
 * (`docs/flows/browser-checkout.md`).
 *
 * # Why `shop-merchant` and not `demo-merchant`
 *
 * Changed 2026-09-11 (exp51). The demo stack registers two merchant clients
 * on two tenants, and the dashboard reads exactly one of them — the shop's
 * (`demo_dashboard_merchant`). The intent minted here is looked up by id in
 * `dashboard.cy.ts`, so minted as `demo-merchant` it was a payment the
 * dashboard could not show and the by-id read answered the uniform
 * cross-tenant 404. Nothing about `checkout.cy.ts` cares which tenant pays:
 * it drives a publishable key and a `client_secret`, and both merchants have
 * one.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { VpayClient } from "@vaam-apps/vpay-sdk";

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
 * A `VpayClient` bound to `shop-merchant` — same tenant `/dash/v1` reads
 * (`demo_dashboard_merchant`, exp51) — built once so both {@link
 * mintCheckoutPaymentIntent} and {@link mintPaymentIntentsForPaging} read
 * the private key exactly the same way and fail with the same message if it
 * is missing.
 */
function shopMerchantClient(): {
  vpay: VpayClient;
  publishableKey: string;
  baseUrl: string;
} {
  const baseUrl = process.env["VPAY_BASE_URL"] ?? "http://localhost:8080";
  const clientId = process.env["VPAY_MERCHANT_CLIENT_ID"] ?? "shop-merchant";
  const privateKeyPath =
    process.env["VPAY_MERCHANT_PRIVATE_KEY_PATH"] ??
    join(repoRoot, ".e2e", "shop-merchant", "oauth-signing-key.pem");
  // Fixed literal `just gen-demo-keys` writes into `.e2e/application-demo.yml`
  // — see that recipe's own comment on why it is fixed rather than generated.
  const publishableKey =
    process.env["CHECKOUT_PUBLISHABLE_KEY"] ?? "pk_test_shopmerchantsandbox1";

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

  return {
    vpay: new VpayClient({ baseUrl, clientId, privateKey: privateKeyPem }),
    publishableKey,
    baseUrl,
  };
}

/**
 * Mints a 50.00 EUR `mtn_momo` PaymentIntent and returns everything
 * `checkout.cy.ts` needs to open the example page: the URL it must visit is
 * `${CHECKOUT_BROWSER_URL}/?pk=${publishableKey}&client_secret=${clientSecret}&api=${baseUrl}`.
 */
export async function mintCheckoutPaymentIntent(): Promise<MintedCheckout> {
  const { vpay, publishableKey, baseUrl } = shopMerchantClient();

  const intent = await vpay.paymentIntents.create(
    {
      // XAF since 2026-09-04 (Step 9, lane 4): the demo overlay
      // `just gen-demo-keys` writes now settles BOTH rails in XAF, because
      // the demo shop prices its catalogue in XAF and offers both. CI's `e2e`
      // job brings this stack up with `-f compose.demo.yml`, so this spec
      // reads that overlay and not `config/application.yml` — which still
      // puts `mtn_momo` on EUR, because MTN's real sandbox rejects XAF.
      //
      // If this and the rail disagree, the failure is a real refusal at
      // confirm and not a silent mismatch: `invalid_request_error/
      // invalid_request: rail 'mtn_momo' settles in <x>; this PaymentIntent
      // is <y>`. 5 000 FCFA — XAF is zero-decimal.
      amount: 5000,
      currency: "xaf",
      payment_method_types: ["mtn_momo"],
      metadata: { source: "checkout.cy.ts" },
    },
    {
      idempotencyKey: `checkout-cy-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    },
  );

  // `@vaam-apps/vpay-sdk`'s `PaymentIntent` type (sdks/nodejs/src/types.ts) declares
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

/**
 * Mints `count` unconfirmed PaymentIntents on `shop-merchant`'s tenant —
 * the same one `/dash/v1` reads — and returns their ids.
 *
 * Exists for `dashboard.cy.ts`'s paging test (issue #88 item 3): measured on
 * 2026-09-11, the tenant carries only the three payments earlier legs of
 * this same spec run create (one from `checkout.cy.ts`, which runs first in
 * `e2e:default`'s alphabetical spec order, and two from this file's own
 * earlier tests) by the time that test runs — `shop-hosted.cy.ts`'s four and
 * `shop-embedded.cy.ts`'s six run AFTER it, in later spec files or a wholly
 * separate `cypress run` (`e2e:framed`). Against `payments-query.ts`'s
 * `PAGE_SIZE` of 25 that is not close: the "if a Next link exists" branch a
 * conditional test would take was never the forward-paging path, it was
 * always the single-page one. This mints enough that the forward path is
 * the one that runs, unconditionally.
 *
 * Sequential, not `Promise.all`: this is Node reusing one `VpayClient`
 * against a real server on `localhost`, and two dozen sequential creates are
 * a few seconds, not a bottleneck worth the concurrent-request risk against
 * a shared demo stack.
 */
export async function mintPaymentIntentsForPaging(
  count: number,
): Promise<{ ids: string[] }> {
  const { vpay } = shopMerchantClient();
  const ids: string[] = [];
  for (let i = 0; i < count; i += 1) {
    const intent = await vpay.paymentIntents.create(
      {
        amount: 5000,
        currency: "xaf",
        payment_method_types: ["mtn_momo"],
        metadata: { source: "dashboard.cy.ts:paging-seed" },
      },
      {
        idempotencyKey: `dashboard-cy-paging-${Date.now()}-${i}-${Math.random().toString(36).slice(2, 8)}`,
      },
    );
    ids.push(intent.id);
  }
  return { ids };
}
