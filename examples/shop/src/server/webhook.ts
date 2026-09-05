/**
 * `POST /api/vpay/webhook`, minus the Next.js plumbing.
 *
 * This is the only thing in the shop that may move an order out of `unpaid`.
 * The return page does not, the cancel page does not, and no browser call
 * does — D11: "the order page shows `paid` only from the webhook". A payer
 * who reaches `success_url` has been told by a redirect that they paid; a
 * signature-verified event from vpay is the shop's own evidence that they
 * did.
 *
 * Four rules, each of them a test in `webhook.test.ts`:
 *
 * 1. A bad or missing signature is **400 and nothing is written**.
 * 2. A verified event settles the order **once**.
 * 3. A replay of the same `evt_…` writes nothing and is still 2xx.
 * 4. An event naming a `pi_…` no order carries writes nothing and is still
 *    2xx — the delivery is not this shop's, and a non-2xx would make vpay
 *    retry it until the endpoint's failure budget ran out.
 *
 * The 2xx is answered **after** the write, never before: `applyWebhookEvent`
 * is awaited and its outcome is what the response reports.
 */
import { verifyWebhook, WebhookSignatureError } from "@vaam-apps/vpay-sdk";
import type { Event } from "@vaam-apps/vpay-sdk";
import type { OrderStatus, ShopStore, WebhookOutcome } from "./store/types";

/**
 * The event types this shop acts on, and what each one makes an order.
 *
 * `payment_intent.processing` and `payment_intent.created` are deliberately
 * absent: they are progress, not an outcome, and an order that showed
 * "processing" from a webhook would be showing a state the shop cannot then
 * clear if delivery stops. Anything not listed here is acknowledged and
 * ignored.
 */
export const SETTLING_EVENTS: Readonly<Record<string, OrderStatus>> = {
  "payment_intent.succeeded": "paid",
  "payment_intent.payment_failed": "failed",
  "payment_intent.canceled": "cancelled",
};

/** The header vpay signs with (docs/flows/webhooks.md). */
export const SIGNATURE_HEADER = "vpay-signature";

export interface WebhookDeps {
  store: ShopStore;
  /** The endpoint's signing secret — `VPAY_WEBHOOK_SECRET`. */
  secret: string;
  /** Unix seconds, injectable so a test can pin the tolerance window. */
  now?: number | undefined;
}

export interface WebhookRequest {
  /** The exact bytes vpay sent. Re-serialising the parsed body breaks the HMAC. */
  rawBody: string;
  signatureHeader: string | null;
}

export interface WebhookResult {
  status: number;
  body:
    | { received: boolean; outcome: WebhookOutcome | "ignored" }
    | { error: string };
  /** For the caller's log line. `undefined` when the signature failed. */
  eventId?: string;
  eventType?: string;
}

/** Reads the `pi_…` an event's object names, or `undefined` if it names none. */
function paymentIntentIdOf(event: Event): string | undefined {
  const object: unknown = event.data.object;
  if (typeof object !== "object" || object === null) {
    return undefined;
  }
  const id: unknown = (object as Record<string, unknown>)["id"];
  return typeof id === "string" && id.length > 0 ? id : undefined;
}

export async function handleWebhook(
  deps: WebhookDeps,
  request: WebhookRequest,
): Promise<WebhookResult> {
  if (request.signatureHeader === null) {
    return { status: 400, body: { error: "missing signature" } };
  }

  let event: Event;
  try {
    event = verifyWebhook({
      rawBody: request.rawBody,
      signatureHeader: request.signatureHeader,
      secret: deps.secret,
      now: deps.now,
    });
  } catch (err) {
    if (err instanceof WebhookSignatureError) {
      // Deliberately not the verifier's message: it names which check failed
      // (tolerance, no matching v1, malformed `t`), which is a hint an
      // attacker would otherwise get for free.
      return { status: 400, body: { error: "invalid signature" } };
    }
    throw err;
  }

  // `Object.hasOwn`, not a bare index: `event.type` is attacker-chosen text
  // (a holder of the signing secret, but still text this code did not
  // produce), and `SETTLING_EVENTS["constructor"]` is a truthy function
  // rather than `undefined`. The bare form would have carried that value on
  // as a status.
  const nextStatus = Object.hasOwn(SETTLING_EVENTS, event.type)
    ? SETTLING_EVENTS[event.type]
    : undefined;
  if (nextStatus === undefined) {
    return {
      status: 200,
      body: { received: true, outcome: "ignored" },
      eventId: event.id,
      eventType: event.type,
    };
  }

  const paymentIntentId = paymentIntentIdOf(event);
  if (paymentIntentId === undefined) {
    // A settling event whose object has no id is not something to guess at.
    return {
      status: 400,
      body: { error: "event object carries no id" },
      eventId: event.id,
      eventType: event.type,
    };
  }

  const outcome = await deps.store.applyWebhookEvent({
    eventId: event.id,
    type: event.type,
    paymentIntentId,
    nextStatus,
  });

  return {
    status: 200,
    body: { received: true, outcome },
    eventId: event.id,
    eventType: event.type,
  };
}
