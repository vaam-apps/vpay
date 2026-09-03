/**
 * `Idempotency-Key`, which vpay **requires** on every `/v1` POST where Stripe
 * merely accepts one.
 *
 * That stricter rule costs a stripe-node user nothing, and this file is the
 * evidence: stripe-node generates a key for every v1 POST unconditionally —
 * "including when maxNetworkRetries is 0" — so the first case here sends no
 * key of its own and still succeeds.
 */
import Stripe from "stripe";
import { describe, expect, it } from "vitest";

import { caught, createIntent, stripeClient } from "./client.js";

const stripe = stripeClient();

/** A key inside vpay's 1–255 printable-ASCII rule, unique per run. */
function key(label: string): string {
  return `stripe-compat-${label}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

describe("idempotency", () => {
  it("succeeds with no key of the caller's, because stripe-node always sends one", async () => {
    const created = await createIntent(stripe, {
      metadata: { case: "auto-key" },
    });
    expect(created.id).toMatch(/^pi_/);
  });

  it("replays the same object for a repeated key", async () => {
    const idempotencyKey = key("replay");
    const first = await createIntent(
      stripe,
      { metadata: { case: "replay" } },
      { idempotencyKey },
    );
    const second = await createIntent(
      stripe,
      { metadata: { case: "replay" } },
      { idempotencyKey },
    );

    expect(second.id).toBe(first.id);
    expect(second.created).toBe(first.created);
    expect(second.status).toBe(first.status);
    // A replay is a separate HTTP request and gets its own request id; it is
    // the *object* that is identical, not the response.
    expect(second.lastResponse.requestId).not.toBe(
      first.lastResponse.requestId,
    );
  });

  it("turns a reused key with a different body into StripeIdempotencyError", async () => {
    const idempotencyKey = key("reused");
    await createIntent(stripe, { amount: 5000 }, { idempotencyKey });

    const error = await caught(() =>
      createIntent(stripe, { amount: 6000 }, { idempotencyKey }),
    );

    // 400 + `type: idempotency_error` is the one pairing stripe-node routes
    // to this class rather than to StripeInvalidRequestError.
    expect(error).toBeInstanceOf(Stripe.errors.StripeIdempotencyError);
    const stripeError = error as Stripe.errors.StripeIdempotencyError;
    expect(stripeError.statusCode).toBe(400);
    expect(stripeError.rawType).toBe("idempotency_error");
    expect(stripeError.code).toBe("idempotency_key_in_use");
    // The key is named back only by its first characters — never in full.
    expect(stripeError.message).not.toContain(idempotencyKey);
  });
});
