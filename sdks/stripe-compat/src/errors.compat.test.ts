/**
 * How vpay's failures arrive at a merchant who is holding stripe-node.
 *
 * stripe-node picks the error *class* from the status code first and the
 * envelope's `type` only inside 400/404 (`generateV1Error`). vpay derives
 * both from one classification (`vpay_core::Category`, ADR-0011), so the
 * mapping below is a property of the two designs meeting — not of any code
 * written to make this file pass.
 */
import Stripe from "stripe";
import { describe, expect, it } from "vitest";

import {
  caught,
  confirmIntent,
  createIntent,
  invalidRequest,
  stripeClient,
  stripeClientWithBadCredential,
} from "./client.js";

const stripe = stripeClient();

/** A UUID, which is what the router mints when no caller supplied an id. */
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

describe("error mapping", () => {
  it("turns an unknown id into StripeInvalidRequestError with resource_missing", async () => {
    const error = await caught(() =>
      stripe.paymentIntents.retrieve("pi_definitely_not_a_real_intent"),
    );

    expect(error).toBeInstanceOf(Stripe.errors.StripeInvalidRequestError);
    const stripeError = error as Stripe.errors.StripeInvalidRequestError;
    expect(stripeError.statusCode).toBe(404);
    expect(stripeError.code).toBe("resource_missing");
    expect(stripeError.rawType).toBe("invalid_request_error");
    expect(stripeError.message).toContain("No such payment_intent");
    // The promise `Category::Internal`'s public message makes — "contact
    // support with the request id" — is only keepable if this is populated.
    expect(stripeError.requestId).toMatch(UUID);
  });

  it("turns a rejected bearer token into StripeAuthenticationError", async () => {
    const error = await caught(() =>
      stripeClientWithBadCredential().paymentIntents.retrieve("pi_anything"),
    );

    expect(error).toBeInstanceOf(Stripe.errors.StripeAuthenticationError);
    const stripeError = error as Stripe.errors.StripeAuthenticationError;
    expect(stripeError.statusCode).toBe(401);
    expect(stripeError.rawType).toBe("authentication_error");
    expect(stripeError.code).toBe("invalid_token");
    expect(stripeError.requestId).toMatch(UUID);
  });

  it("refuses `confirm: true` on create, naming the parameter", async () => {
    const error = await caught(() => createIntent(stripe, { confirm: true }));

    expect(error).toBeInstanceOf(Stripe.errors.StripeInvalidRequestError);
    const stripeError = error as Stripe.errors.StripeInvalidRequestError;
    expect(stripeError.statusCode).toBe(400);
    // `param` is where a Stripe SDK user is pointed, and it is the field that
    // makes this a debuggable refusal rather than a silently dropped one.
    expect(stripeError.param).toBe("confirm");
    expect(stripeError.message).toContain("/v1/payment_intents/{id}/confirm");
  });

  it("names `payment_method_types` when a Stripe-shaped snippet omits it", async () => {
    // What a copied `automatic_payment_methods: {enabled: true}` snippet
    // amounts to on vpay: the field is dropped (no `deny_unknown_fields`)
    // and the required one is missing.
    const error = await caught(() =>
      createIntent(stripe, { payment_method_types: [] }),
    );

    expect(error).toBeInstanceOf(Stripe.errors.StripeInvalidRequestError);
    const stripeError = error as Stripe.errors.StripeInvalidRequestError;
    expect(stripeError.statusCode).toBe(400);
    expect(stripeError.param).toBe("payment_method_types");
  });

  /**
   * The 409, and the header that stops stripe-node re-POSTing it.
   *
   * stripe-node retries **every** 409 unconditionally unless the response
   * says otherwise (`RequestSender._shouldRetry`), and a lifecycle refusal —
   * "this intent is already `processing`" — is not something waiting fixes.
   * vpay answers `stripe-should-retry: false`, derived from
   * `Classify::retry`, and this case reads that header back **through the
   * SDK's own error object**, which is the only place a merchant can see it.
   *
   * The elapsed-time bound is the behavioural half, and it is set **just
   * under stripe-node's floor for a retried request rather than just over
   * the observed time for a non-retried one.** `initialNetworkRetryDelay` is
   * 0.5 s, it is not configurable, and `maxNetworkRetries: 2` sleeps it
   * twice: any request that was retried here costs **at least 1000 ms**. So
   * 900 ms is the largest ceiling that still cannot be met by a retry, and
   * every millisecond below it is bought with flakiness rather than
   * strength — the earlier 700 ms proved the identical fact while leaving a
   * loaded CI runner only 700 ms to complete two un-retried round trips.
   * Raise this only if 1000 ms ever stops being stripe-node's floor.
   */
  it("surfaces a 409 as StripeAPIError and does not retry it", async () => {
    const created = await createIntent(stripe, { metadata: { case: "409" } });
    await confirmIntent(stripe, created.id);

    const retrying = stripeClient({ maxNetworkRetries: 2 });
    const started = Date.now();
    const error = await caught(() => confirmIntent(retrying, created.id));
    const elapsedMs = Date.now() - started;

    expect(error).toBeInstanceOf(Stripe.errors.StripeAPIError);
    const stripeError = error as Stripe.errors.StripeAPIError;
    expect(stripeError.statusCode).toBe(409);
    expect(stripeError.code).toBe("invalid_state");
    expect(stripeError.headers?.["stripe-should-retry"]).toBe("false");
    expect(stripeError.requestId).toMatch(UUID);
    expect(elapsedMs).toBeLessThan(900);
  });

  /**
   * The two responses on this surface that are **not** rendered by
   * `ApiError::into_response`, and what a stripe-node user sees instead of
   * an error they can act on.
   *
   * `405` (a POST-only route asked with GET) is axum's own, with an empty
   * body; `413` (past `V1_BODY_LIMIT_BYTES`, 64 KiB) is tower-http's, with a
   * `text/plain` body. Neither passes through the renderer, so neither
   * carries the Stripe envelope nor `stripe-should-retry`.
   *
   * **Measured, not inferred:** stripe-node meets a non-JSON body by
   * discarding everything it knows about the response and throwing
   * `StripeAPIError: "Invalid JSON received from the Stripe API"` — with
   * `statusCode` and `headers` both `undefined`. So a merchant cannot tell a
   * 405 from a 413 from a proxy's HTML 502; the only thing that survives is
   * the request id, which stripe-node reads out of the header separately.
   * That is the actual cost of these two responses lacking an envelope, and
   * it is bigger than the missing retry advisory. See
   * `docs/flows/stripe-sdk-compat.md`.
   */
  it("collapses 405 and 413 into an opaque StripeAPIError that keeps only the request id", async () => {
    for (const provoke of [
      // `rawRequest` because no generated resource method can produce a GET
      // on a POST-only path.
      (): Promise<unknown> =>
        stripe.rawRequest("GET", "/v1/payment_intents/pi_x/confirm"),
      (): Promise<unknown> =>
        createIntent(stripe, { description: "x".repeat(70_000) }),
    ]) {
      const error = await caught(provoke);

      expect(error).toBeInstanceOf(Stripe.errors.StripeAPIError);
      const stripeError = error as Stripe.errors.StripeAPIError;
      expect(stripeError.message).toBe(
        "Invalid JSON received from the Stripe API",
      );
      expect(stripeError.statusCode).toBeUndefined();
      expect(stripeError.headers).toBeUndefined();
      // The one thing a merchant can still quote to support.
      expect(stripeError.requestId).toMatch(UUID);
    }
  });
});

/**
 * The fields that decide **where or when money moves**, refused end to end.
 *
 * `vpay-api` has a unit test for the refusal itself; what only a live stack
 * can show is that the refusal survives the whole round trip a merchant
 * actually makes — stripe-node's form encoder writing
 * `transfer_data[destination]=acct_x`, vpay's decoder reading the brackets
 * back into a nested value, the `#[serde(flatten)]`ed struct seeing it, and
 * `error.param` arriving on the SDK's own error object. Each of those four
 * could break independently and leave the unit test green.
 *
 * Refusing rather than ignoring is the whole point: an ignored
 * `transfer_data` settles the entire amount to the merchant who called,
 * which is neither what was asked for nor visible in the response.
 */
describe("parameters that move money elsewhere", () => {
  it("refuses `capture_method: manual` on create, naming the parameter", async () => {
    const error = await invalidRequest(() =>
      createIntent(stripe, { capture_method: "manual" }),
    );

    expect(error.statusCode).toBe(400);
    expect(error.param).toBe("capture_method");
    // vpay has no authorise/capture split, and the message has to say so —
    // `param` alone tells a merchant which field, not why it cannot work.
    expect(error.message).toContain("does not support");
  });

  it("refuses `transfer_data` on create, naming the parameter", async () => {
    // The nested case: stripe-node encodes this as
    // `transfer_data[destination]=acct_x`, so a `param` of `transfer_data`
    // proves vpay's bracket decoding fed the flattened struct.
    const error = await invalidRequest(() =>
      createIntent(stripe, { transfer_data: { destination: "acct_x" } }),
    );

    expect(error.statusCode).toBe(400);
    expect(error.param).toBe("transfer_data");
    expect(error.message).toContain("Connect");
  });

  it("accepts `capture_method: automatic`, which asks for what vpay does", async () => {
    // The half that fails if the refusal is widened to "any capture_method".
    const created = await createIntent(stripe, {
      capture_method: "automatic",
      metadata: { case: "capture-automatic" },
    });

    expect(created.id).toMatch(/^pi_/);
    expect(created.status).toBe("requires_payment_method");
  });

  it("refuses a money-moving parameter on confirm too, not only on create", async () => {
    // The confirm body is a second, separately-decoded surface: a merchant
    // refused on create would otherwise get the same field silently ignored
    // one request later, on the call that actually charges the payer.
    const created = await createIntent(stripe, {
      metadata: { case: "confirm-refusal" },
    });

    const error = await invalidRequest(() =>
      confirmIntent(stripe, created.id, { application_fee_amount: 250 }),
    );

    expect(error.statusCode).toBe(400);
    expect(error.param).toBe("application_fee_amount");
    expect(error.message).toContain("Connect");

    // And the refusal charged nobody: the intent is still awaiting a payment
    // method, so the corrected confirm is still available to the merchant.
    const after = await stripe.paymentIntents.retrieve(created.id);
    expect(after.status).toBe("requires_payment_method");
  });
});
