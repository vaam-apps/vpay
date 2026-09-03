/**
 * A **delivered** webhook, verified with the **real `stripe` package's own
 * verifier** — `stripe.webhooks.constructEvent`.
 *
 * This is the case that closes the gap three documents used to carry: vpay's
 * `Stripe-Signature` was argued to be byte-identical to Stripe's, and pinned
 * against a second copy of the HMAC and against `@vpay/sdk`'s verifier, but it
 * had never been handed to the library a merchant actually installs. An
 * argument from "the scheme is the same" is not an observation, and the whole
 * point of `Stripe-Signature` is that a copy-pasted Stripe recipe works
 * unchanged.
 *
 * # Where the delivery comes from
 *
 * Not from vpay's tables. `webhook_deliveries.state = 'succeeded'` is the
 * sender's belief about what it sent; the WireMock receiver's request journal
 * (`GET /__admin/requests`) is what a receiver actually got, headers and body,
 * byte for byte. Reading the first would be quoting the sender back to itself.
 * Same source, same reasoning and the same two filters as
 * `examples/merchant-demo`'s step 7.
 *
 * # Why it makes a payment first
 *
 * Because there is no other way to get an event. The chain is real and every
 * link has to run: confirm reaches the rail, `vpay-worker` polls it, the
 * settlement writes an `events` row in the same transaction, the
 * `fanout:events` drain turns that into a `webhook_deliveries` row and a
 * `deliver_webhook` job, and that job signs and POSTs the body. Nothing here
 * inserts a row or forges a header.
 *
 * # The bytes are never re-serialised
 *
 * The signature covers the exact body that was sent, so the journal's recorded
 * text goes into `constructEvent` verbatim. Parsing and re-printing JSON is
 * the single most common way a merchant breaks their own verification, and a
 * test that did it would be verifying a body vpay never sent.
 */
import Stripe from "stripe";
import { describe, expect, it } from "vitest";

import { confirmIntent, createIntent, stripeClient } from "./client.js";
import { readCompatEnv } from "./env.js";

const stripe = stripeClient();
const env = readCompatEnv();

/**
 * How long to wait for the settlement, and then for the delivery.
 *
 * The settlement window matches `lifecycle.compat.test.ts`'s and for the same
 * reason — see that file's `SETTLE_WINDOW_MS` for why the spread is wide. The
 * delivery window is much shorter because by then the `events` row already
 * exists: what is left is the fan-out singleton, which reschedules every 5 s,
 * and one `deliver_webhook` job that runs immediately.
 */
const SETTLE_WINDOW_MS = 120_000;
const DELIVERY_WINDOW_MS = 45_000;
const POLL_MS = 2_000;

/** One delivery as the receiver recorded it. */
interface Delivered {
  /** `Vpay-Event-Id` — the `evt_…` the delivery names. */
  readonly eventId: string;
  /** The recorded request body, verbatim. Never re-serialised. */
  readonly body: string;
  /** `Stripe-Signature`, the header a Stripe-shaped handler reads. */
  readonly stripeSignature: string;
  /** `Vpay-Signature`, which is the authoritative name for the same bytes. */
  readonly vpaySignature: string;
}

/** WireMock's journal entry shape, narrowed to what this file reads. */
interface JournalEntry {
  request?: {
    method?: string;
    body?: string;
    headers?: Record<string, unknown>;
  };
}

const sleep = (ms: number): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Case-insensitive header lookup over WireMock's journal.
 *
 * WireMock records header names as the sender wrote them, and HTTP header
 * names are case-insensitive — a lookup that assumed a casing would pass or
 * fail on a detail neither vpay nor Stripe promises.
 */
function header(
  headers: Record<string, unknown> | undefined,
  name: string,
): string | undefined {
  if (headers === undefined) return undefined;
  for (const [key, value] of Object.entries(headers)) {
    if (key.toLowerCase() === name.toLowerCase() && typeof value === "string") {
      return value;
    }
  }
  return undefined;
}

/**
 * The POST in the receiver's journal that delivered an event about
 * `paymentIntentId`, if one has arrived.
 *
 * Two filters, both load-bearing. `Vpay-Event-Id`, because the receiver
 * answers 200 to anything POSTed at it and a stray request must not be
 * mistaken for a delivery. The body's `data.object.id`, because the journal
 * survives for the life of the container and a *previous* run's delivery
 * would otherwise satisfy this one.
 */
async function recordedDelivery(
  paymentIntentId: string,
): Promise<Delivered | undefined> {
  const response = await fetch(`${env.receiverUrl}/__admin/requests`, {
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) {
    throw new Error(
      `the receiver's journal answered ${response.status}; is ${env.receiverUrl} the ` +
        `port wiremock-webhook is published on (VPAY_RECEIVER_URL)?`,
    );
  }
  const journal = (await response.json()) as { requests?: JournalEntry[] };
  for (const entry of journal.requests ?? []) {
    const request = entry.request;
    if (request?.method !== "POST") continue;
    const eventId = header(request.headers, "vpay-event-id");
    const stripeSignature = header(request.headers, "stripe-signature");
    const vpaySignature = header(request.headers, "vpay-signature");
    if (
      eventId === undefined ||
      stripeSignature === undefined ||
      vpaySignature === undefined
    ) {
      continue;
    }
    const body = request.body ?? "";
    let namesThisIntent = false;
    try {
      const parsed = JSON.parse(body) as { data?: { object?: { id?: string } } };
      namesThisIntent = parsed.data?.object?.id === paymentIntentId;
    } catch {
      namesThisIntent = false;
    }
    if (!namesThisIntent) continue;
    return { eventId, body, stripeSignature, vpaySignature };
  }
  return undefined;
}

/** Confirms a fresh intent and waits for the worker to settle it. */
async function aSettledPayment(): Promise<string> {
  const created = await createIntent(stripe, {
    metadata: { case: "webhook" },
  });
  await confirmIntent(stripe, created.id);

  const deadline = Date.now() + SETTLE_WINDOW_MS;
  let status = "processing";
  while (Date.now() < deadline && status === "processing") {
    await sleep(POLL_MS);
    status = (await stripe.paymentIntents.retrieve(created.id)).status;
  }
  expect(
    status,
    "no event can exist until a charge settles — check that vpay-worker is running",
  ).toBe("succeeded");
  return created.id;
}

/** Waits for the delivery of an event naming `intentId`. */
async function aDeliveredWebhook(intentId: string): Promise<Delivered> {
  const deadline = Date.now() + DELIVERY_WINDOW_MS;
  let found: Delivered | undefined;
  while (Date.now() < deadline && found === undefined) {
    found = await recordedDelivery(intentId);
    if (found === undefined) await sleep(POLL_MS);
  }
  if (found === undefined) {
    throw new Error(
      `no webhook was delivered for ${intentId} within ${DELIVERY_WINDOW_MS / 1000}s. ` +
        `The intent already reached "succeeded", so the events row exists and this is the ` +
        `fan-out or the delivery failing — try \`docker compose logs vpay-worker\`.`,
    );
  }
  return found;
}

describe("a delivered webhook, through the real stripe package", () => {
  it(
    "verifies with stripe.webhooks.constructEvent, and rejects a tampered body",
    async () => {
      const intentId = await aSettledPayment();
      const delivered = await aDeliveredWebhook(intentId);

      // The duplicate header exists so a Stripe-shaped handler keeps working
      // unedited; one that drifted from the header it mirrors would verify in
      // @vpay/sdk and fail in the merchant's own code.
      expect(delivered.stripeSignature).toBe(delivered.vpaySignature);

      // The call a merchant's handler makes, verbatim, against the real
      // library. `constructEvent` throws on a bad signature, so reaching the
      // next line is the assertion.
      const event = stripe.webhooks.constructEvent(
        delivered.body,
        delivered.stripeSignature,
        env.webhookSecret,
      );

      expect(event.id).toBe(delivered.eventId);
      expect(event.type).toBe("payment_intent.succeeded");
      expect(event.livemode).toBe(false);
      expect((event.data.object as { id?: string }).id).toBe(intentId);

      // And the other direction, which is the half that makes the first one
      // worth anything: a verifier that accepted everything would also have
      // accepted the delivery above. One byte of the payload is changed and
      // the same header must now be refused.
      const tampered = delivered.body.replace(
        '"amount"',
        '"amount_tampered"',
      );
      expect(tampered).not.toBe(delivered.body);
      expect(() =>
        stripe.webhooks.constructEvent(
          tampered,
          delivered.stripeSignature,
          env.webhookSecret,
        ),
      ).toThrow(Stripe.errors.StripeSignatureVerificationError);

      // A wrong secret is refused for the same reason, and it is the failure a
      // merchant meets in practice — a rotated endpoint secret, not an
      // attacker.
      expect(() =>
        stripe.webhooks.constructEvent(
          delivered.body,
          delivered.stripeSignature,
          `${env.webhookSecret}-not-the-one`,
        ),
      ).toThrow(Stripe.errors.StripeSignatureVerificationError);
    },
    SETTLE_WINDOW_MS + DELIVERY_WINDOW_MS + 15_000,
  );
});
