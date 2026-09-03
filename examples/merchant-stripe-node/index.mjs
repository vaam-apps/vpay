/**
 * `merchant-stripe-node` — one payment, driven by the **official `stripe`
 * package** against a real vpay.
 *
 * The point of this file is the six lines that build the client. Everything
 * after them is ordinary stripe-node code that a merchant already has:
 *
 *   1. create a PaymentIntent
 *   2. confirm it against a push rail (MTN MoMo), which reaches the rail
 *   3. poll `paymentIntents.retrieve` until it settles
 *
 * **Step 4 ends in `succeeded`, and nothing here fabricates it.** The
 * `vpay-worker` container claims the `poll_charge` job the confirm committed
 * in the charge's own transaction, asks the WireMock MTN rail over HTTP, and
 * settles the charge when the rail says the payer approved. This program only watches,
 * through `paymentIntents.retrieve` — the same call a merchant integration
 * makes and the only thing it can see. It **fails** rather than hanging if
 * the window closes with the intent still `processing`, and fails on any
 * status that is not `processing` or `succeeded`.
 *
 * That assertion is the inverse of the one this file shipped with: before the
 * worker existed (Step 4) it asserted the intent *stayed* `processing`, and
 * said that the day a worker landed was the day it had to be flipped.
 *
 * Nothing here is stubbed: the requests leave this process over TCP and the
 * rail is a WireMock *host* reached over HTTP, the same mechanism a real rail
 * is reached by (ADR-0006). **MTN has never been called by this code, and no
 * money has moved.**
 *
 * README.md has the ten steps that boot a stack and run this.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import Stripe from "stripe";
import { createStripeAuthenticator } from "@vpay/sdk/stripe";

const BASE_URL = process.env.VPAY_BASE_URL ?? "http://localhost:18080";
const CLIENT_ID = process.env.VPAY_MERCHANT_CLIENT_ID ?? "demo-merchant";
// Resolved from this file rather than from the working directory: `pnpm
// --filter ... start` runs with the cwd set to *this* package, while
// `gen-demo-keys` writes into the repository root's `.e2e/`. A bare relative
// default would work from the root and fail from anywhere else — including
// from the command the README gives.
const KEY_PATH =
  process.env.VPAY_MERCHANT_PRIVATE_KEY_PATH ??
  fileURLToPath(
    new URL("../../.e2e/demo-merchant/oauth-signing-key.pem", import.meta.url),
  );

/**
 * EUR, not XAF, and it is the rail's property rather than a preference:
 * `config/application.yml` settles `mtn_momo` in EUR because MTN's sandbox
 * rejects XAF, and `/v1` refuses a confirm whose intent currency is not the
 * rail's. EUR has two decimals, so 5000 is €50.00.
 */
const AMOUNT = 5000;
const CURRENCY = "eur";
const RAIL = "mtn_momo";
/** A documentation MSISDN. Not anyone's. */
const MSISDN = "237670000000";

/**
 * How long step 4 waits for the worker to settle the charge, and how often it
 * asks.
 *
 * A ceiling, not an expectation. The poll job is enqueued with
 * `run_at = now()`, so the first poll happens as soon as the worker's idle
 * loop notices it (1 s) — `vpay_worker::poll_delay`'s 10 s / 20 s rungs
 * govern the *re*-polls after a `PENDING`. Against the compose MTN stub this
 * usually settles within a couple of seconds, and ~10 s longer if the
 * `mtn-e2e-poll` scenario has been entered by a previous `just demo` on the
 * same containers.
 *
 * 120 s is generous enough that a cold compose stack does not fail this
 * example and tight enough that a worker which is not running fails it in two
 * minutes with a message saying which.
 *
 * The same shape as `examples/merchant-demo`'s `SETTLE_TIMEOUT`, and for the
 * same reason: a bounded wait that fails is a result, an unbounded one is a
 * hang.
 */
const POLL_WINDOW_MS = 120_000;
const POLL_INTERVAL_MS = 2_000;

const url = new URL(BASE_URL);

// --- the six lines this example exists for -------------------------------
//
// vpay accepts no API key (ADR-0010): every /v1 call carries a short-lived
// bearer token minted from an RFC 7523 `private_key_jwt` client assertion.
// stripe-node has no notion of that handshake, but `config.authenticator` is
// arbitrary async code it invokes once per request attempt — so the handshake
// goes there, and nothing else about the SDK changes.
const authenticator = createStripeAuthenticator({
  baseUrl: url.origin,
  clientId: CLIENT_ID,
  privateKey: readFileSync(KEY_PATH, "utf8"),
});

const stripe = new Stripe("", {
  authenticator,
  host: url.hostname,
  port: url.port,
  protocol: url.protocol === "https:" ? "https" : "http",
  telemetry: false,
});
// -------------------------------------------------------------------------

/**
 * The handshake, once, before anything else.
 *
 * Not decoration: when the vpay handshake fails, stripe-node builds the right
 * error and then throws it inside a detached promise chain that never calls
 * its own callback, so the promise you awaited never settles at all
 * (measured against stripe@22.6.1 — see sdks/nodejs/README.md). Calling the
 * authenticator directly is the one place a bad key surfaces as a normal
 * rejection you can catch.
 */
async function verifyHandshake() {
  const probe = { headers: {} };
  await authenticator(probe);
  if (typeof probe.headers["Authorization"] !== "string") {
    throw new Error("the authenticator set no Authorization header");
  }
  console.log(`1. authenticated as ${CLIENT_ID} at ${url.origin}`);
}

async function main() {
  await verifyHandshake();

  const intent = await stripe.paymentIntents.create({
    amount: AMOUNT,
    currency: CURRENCY,
    // `mtn_momo` is a vpay rail code, not one of Stripe's payment methods, so
    // stripe-node's generated TypeScript types do not know it. This file is
    // JavaScript and does not care; a TypeScript merchant needs a cast. The
    // wire is `payment_method_types[0]=mtn_momo` either way.
    payment_method_types: [RAIL],
    metadata: { order_id: "1234" },
  });
  console.log(
    `2. created ${intent.id} — ${intent.status}, ${intent.amount} ${intent.currency}` +
      ` (request-id ${intent.lastResponse.requestId})`,
  );

  const confirmed = await stripe.paymentIntents.confirm(intent.id, {
    // `payment_method_data[type]` is the rail code and
    // `payment_method_data[<type>][msisdn]` is the payer's number; stripe-node
    // encodes this nested object into exactly those bracketed form keys.
    payment_method_data: { type: RAIL, [RAIL]: { msisdn: MSISDN } },
  });
  if (confirmed.status !== "processing") {
    throw new Error(
      `3. confirm answered ${confirmed.status}; a push rail's one success ` +
        `state is "processing"`,
    );
  }
  console.log(
    `3. confirmed — ${confirmed.status}. The rail has the request; the payer ` +
      `would now approve it on their handset.`,
  );

  const startedAt = Date.now();
  const deadline = startedAt + POLL_WINDOW_MS;
  let status = confirmed.status;
  let polls = 0;
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
    const polled = await stripe.paymentIntents.retrieve(intent.id);
    polls += 1;
    status = polled.status;
    if (status === "succeeded") {
      break;
    }
    // Anything that is neither the state we started in nor the one we are
    // waiting for is a real answer about this payment and must not be
    // polled past: `canceled` and `requires_payment_method` (a decline, with
    // `last_payment_error` set) are both terminal here.
    if (status !== "processing") {
      throw new Error(
        `4. ${intent.id} settled as "${status}" rather than "succeeded"` +
          (polled.last_payment_error
            ? ` (${polled.last_payment_error.code}: ${polled.last_payment_error.message})`
            : "") +
          `. The MSISDN this example confirms with (${MSISDN}) selects a rail ` +
          `stub that approves the payment; a different answer means the stub ` +
          `mappings or the settlement path changed.`,
      );
    }
  }

  if (status !== "succeeded") {
    throw new Error(
      `4. ${intent.id} was still "${status}" after ${POLL_WINDOW_MS / 1000}s ` +
        `and ${polls} polls. Nothing drove the charge to a terminal state — ` +
        `the usual cause is that the vpay-worker container is not running. ` +
        `Try \`docker compose logs vpay-worker\`.`,
    );
  }

  console.log(
    `4. settled — "${status}" after ${polls} polls ` +
      `(~${Math.round((Date.now() - startedAt) / 1000)}s). The vpay-worker ` +
      `asked the rail whether the payer approved, and it said yes.`,
  );
  console.log(
    "   Nothing in this file made that happen: it only called " +
      "paymentIntents.retrieve, which is all a merchant integration can see. " +
      "The rail is a WireMock host, MTN has never been called by this code, " +
      "and no money has moved.",
  );
}

main().catch((error) => {
  if (error instanceof Stripe.errors.StripeError) {
    console.error(
      `${error.type}: ${error.message}` +
        (error.param ? ` (param: ${error.param})` : "") +
        (error.requestId ? ` [request-id ${error.requestId}]` : ""),
    );
  } else {
    console.error("failed:", error);
  }
  process.exitCode = 1;
});
