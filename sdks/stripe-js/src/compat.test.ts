/**
 * What `@vpay/stripe-js` is and is not assignable to, against the real
 * `@stripe/stripe-js` type definitions (a devDependency, pinned exactly, and
 * used for nothing else — this package has zero runtime dependencies).
 *
 * The README claims "drop-in compatible" for the payment-intent half of
 * Stripe.js. This file is where that claim is made precise, in both
 * directions, so it cannot quietly become false: every `true` and every
 * `false` below is a **compile-time** assertion — `pnpm --filter
 * @vpay/stripe-js typecheck` fails if either package's types move.
 *
 * The short version, all four results pinned below:
 *
 * - Stripe's `StripeError` **is** assignable to ours. Ours is a widening of
 *   Stripe's, so a merchant's existing error-rendering code keeps working.
 * - Ours is **not** assignable to Stripe's, for two deliberate reasons: our
 *   `type` is an open `string` (so a `type` a future vpay version introduces
 *   is not a compile error at the merchant), and our optional members are
 *   written `?: T | undefined`, which `exactOptionalPropertyTypes`
 *   distinguishes from Stripe's `?: T`.
 * - Our `PaymentIntent` is **not** assignable to Stripe's. Stripe's requires
 *   a dozen fields that only exist because it settles cards
 *   (`capture_method`, `confirmation_method`, `canceled_at`,
 *   `payment_method`, …) and its `last_payment_error` is a card-decline
 *   shape. This is the type-level statement of the README's "not compatible,
 *   ever" list.
 * - The two fields a checkout page actually branches on — `status` and
 *   `next_action` — **are** assignable to Stripe's, and the
 *   `PaymentIntentResult` union has the same two-member discriminated shape
 *   in both packages, which is what makes the narrowing idiom portable.
 */
import type {
  PaymentIntent as TheirIntent,
  PaymentIntentResult as Theirs,
  StripeError as TheirError,
  StripeErrorType,
} from "@stripe/stripe-js";
import { describe, expect, it } from "vitest";
import type { StripeError as OurError } from "./errors.js";
import type {
  PaymentIntent as OurIntent,
  PaymentIntentResult as Ours,
} from "./types.js";

/** `true` when a value of `A` may be used where `B` is expected. */
type Is<A, B> = [A] extends [B] ? true : false;

/** Stripe's `PaymentIntentResult` shape, parameterised over its two payloads. */
type ResultShape<Intent, Error> =
  | { paymentIntent: Intent; error?: undefined }
  | { paymentIntent?: undefined; error: Error };

// --- the direction that holds -------------------------------------------

const stripeErrorIsAVpayError: Is<TheirError, OurError> = true;

// --- the directions that do not, and exactly why ------------------------

const vpayErrorIsNotAStripeError: Is<OurError, TheirError> = false;
// Not merely the open `type`: narrowing `type` to Stripe's own union still
// leaves `?: T | undefined` against Stripe's `?: T`.
const evenWithStripesTypeUnion: Is<
  Omit<OurError, "type"> & { type: StripeErrorType },
  TheirError
> = false;
const vpayIntentIsNotAStripeIntent: Is<OurIntent, TheirIntent> = false;
const stripeIntentIsNotAVpayIntent: Is<TheirIntent, OurIntent> = false;
const vpayResultIsNotAStripeResult: Is<Ours, Theirs> = false;

// --- the parts that are compatible, field by field ----------------------

const statusIsAStripeStatus: Is<OurIntent["status"], TheirIntent["status"]> =
  true;
const nextActionIsAStripeNextAction: Is<
  OurIntent["next_action"],
  TheirIntent["next_action"]
> = true;
const clientSecretIsAStripeClientSecret: Is<
  OurIntent["client_secret"],
  TheirIntent["client_secret"]
> = true;

// --- the union shape is identical, which is what makes narrowing portable

const theirsIsTheShape: Is<Theirs, ResultShape<TheirIntent, TheirError>> = true;
const oursIsTheSameShape: Is<Ours, ResultShape<OurIntent, OurError>> = true;

/**
 * The idiom a merchant already has, unchanged, compiling against our result.
 * Written as a real function rather than a type assertion so that the
 * runtime half of this file exercises the same narrowing.
 */
function describeResult(result: Ours): string {
  if (result.error) {
    return `error:${result.error.type}`;
  }
  return `intent:${result.paymentIntent.status}`;
}

describe("@stripe/stripe-js compatibility", () => {
  it("pins every assignability claim the README makes", () => {
    // The assertions are the `const` declarations above; typecheck is what
    // enforces them. Referencing them here keeps them from being dead code
    // and makes the failing test name meaningful when a type moves.
    expect([
      stripeErrorIsAVpayError,
      vpayErrorIsNotAStripeError,
      evenWithStripesTypeUnion,
      vpayIntentIsNotAStripeIntent,
      stripeIntentIsNotAVpayIntent,
      vpayResultIsNotAStripeResult,
      statusIsAStripeStatus,
      nextActionIsAStripeNextAction,
      clientSecretIsAStripeClientSecret,
      theirsIsTheShape,
      oursIsTheSameShape,
    ]).toEqual([
      true,
      false,
      false,
      false,
      false,
      false,
      true,
      true,
      true,
      true,
      true,
    ]);
  });

  it("narrows the same way Stripe.js's result does", () => {
    expect(
      describeResult({
        error: { type: "invalid_request_error", code: "resource_missing" },
      }),
    ).toBe("error:invalid_request_error");
    expect(
      describeResult({
        paymentIntent: {
          id: "pi_1",
          object: "payment_intent",
          amount: 5000,
          currency: "xaf",
          status: "succeeded",
          payment_method_types: ["mtn_momo"],
          next_action: null,
          last_payment_error: null,
          metadata: {},
          description: null,
          created: 1,
          livemode: false,
          client_secret: "pi_1_secret_x",
        },
      }),
    ).toBe("intent:succeeded");
  });

  it("accepts a Stripe error object wherever a vpay one is expected", () => {
    // The runtime companion to `stripeErrorIsAVpayError`.
    const fromStripe: TheirError = {
      type: "card_error",
      code: "card_declined",
      message: "Your card was declined.",
    };
    const asVpay: OurError = fromStripe;
    expect(asVpay.code).toBe("card_declined");
  });
});
