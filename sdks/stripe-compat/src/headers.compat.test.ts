/**
 * The response and request headers a Stripe SDK cares about.
 *
 * These are the assertions the server-side half of this step exists for: a
 * `request-id` response header stripe-node will actually read, and the
 * Stripe request headers vpay has to tolerate without knowing what they mean.
 */
import Stripe from "stripe";
import { describe, expect, it } from "vitest";

import { caught, createIntent, stripeClient } from "./client.js";

const stripe = stripeClient();

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

describe("headers", () => {
  it("mirrors one request id under both `request-id` and `x-request-id`", async () => {
    const created = await createIntent(stripe, {
      metadata: { case: "request-id" },
    });
    const headers = created.lastResponse.headers;

    // The equality is the assertion, not the presence: two headers minted
    // independently would also both be present, and would name two different
    // requests — a merchant quoting the one support cannot find.
    expect(headers["request-id"]).toMatch(UUID);
    expect(headers["x-request-id"]).toBe(headers["request-id"]);
    expect(created.lastResponse.requestId).toBe(headers["request-id"]);
  });

  it("advertises no dated API version", async () => {
    const created = await createIntent(stripe, {
      metadata: { case: "api-version" },
    });

    // vpay implements no dated Stripe API version and says so by echoing
    // nothing, rather than by claiming one it does not track.
    expect(created.lastResponse.headers["stripe-version"]).toBeUndefined();
    expect(created.lastResponse.apiVersion).toBeUndefined();
  });

  it("works when stripe-node is told to send Stripe-Version and Stripe-Account", async () => {
    const created = await createIntent(stripe, {
      metadata: { case: "ignored-headers" },
    });

    // stripe-node builds both as request headers when these options are set.
    // vpay knows neither (no API-version pinning, no Connect), and the sealing
    // HTTP client does not forward them at all — a signed request carries only
    // the headers its signature binds. The assertion is that the call still
    // *works*, which is what a merchant whose code sets them needs; it no
    // longer proves vpay's tolerance of the headers, because none arrive.
    // The empty `params` argument is load-bearing: with two arguments
    // TypeScript resolves the second as *query parameters*, and the two
    // options below would go out on the query string instead of as the
    // `Stripe-Version` and `Stripe-Account` request headers this case is
    // about.
    const retrieved = await stripe.paymentIntents.retrieve(
      created.id,
      {},
      {
        apiVersion: "2026-08-26.dahlia",
        stripeAccount: "acct_vpay_has_no_connect",
      },
    );

    expect(retrieved.id).toBe(created.id);
    expect(retrieved.lastResponse.statusCode).toBe(200);
  });

  it("carries the retry advisory on a rendered error, and its value is false for a permanent one", async () => {
    // The advisory is emitted from the error renderer for *every* error it
    // renders, not only the ones a Stripe SDK is likely to meet. A 404 is
    // the cheapest one to observe, and `false` is the honest answer: an id
    // that does not exist does not start existing on a second ask. Read back
    // through the SDK's error object, which is where a merchant sees it.
    const error = await caught(() =>
      stripe.paymentIntents.retrieve("pi_no_such_intent_here"),
    );
    const stripeError = error as Stripe.errors.StripeError;

    expect(stripeError.statusCode).toBe(404);
    expect(stripeError.headers?.["stripe-should-retry"]).toBe("false");
    expect(stripeError.headers?.["request-id"]).toMatch(UUID);
    expect(stripeError.requestId).toBe(stripeError.headers?.["request-id"]);
  });
});
