/**
 * The one place this suite constructs a `Stripe` client, and the one place it
 * casts.
 *
 * **The casts are the point of the file.** stripe-node's TypeScript types are
 * generated from Stripe's own OpenAPI description, so they know about `card`
 * and `sepa_debit` and nothing about `mtn_momo`. vpay's *runtime* accepts the
 * request fine — `payment_method_types` is a list of rail codes and
 * `payment_method_data` is an untyped map on the server
 * (`vpay_api::v1::payment_intents`) — so the divergence is type-level only,
 * and pretending otherwise by loosening the tests would hide a real thing a
 * merchant has to do. Every cast is named, commented, and confined here; the
 * test files stay cast-free so that a reader can tell an assertion from an
 * accommodation.
 */
import Stripe from "stripe";

import { createStripeAuthenticator } from "@vaam-apps/vpay-sdk/stripe";

import { readCompatEnv } from "./env.js";

/**
 * The rail every case in this suite confirms against.
 *
 * `mtn_momo` is a **push** rail: the payer approves on their handset, so a
 * confirm answers `processing` and never a `next_action`
 * (`docs/flows/payment-lifecycle.md`).
 */
export const RAIL = "mtn_momo";

/**
 * XAF, and it is a property of the STACK rather than a preference: `/v1`
 * refuses a confirm whose intent currency is not the rail's
 * (`vpay_api`'s `currencies_agree`), and this suite runs against the DEMO
 * stack — `just stripe-compat` and CI's `e2e` job both load the overlay
 * `just gen-demo-keys` writes, which since Step 9 settles both rails in XAF.
 *
 * It was `"eur"` until 2026-09-04, and that was equally a property of the
 * stack: the demo overlay had no `providers:` block, so `mtn_momo` inherited
 * `config/application.yml`'s EUR. That file still says EUR, because **MTN's
 * real sandbox rejects XAF** (`docs/flows/money.md`); what changed is that
 * the demo shop prices its catalogue in XAF and offers both rails, so the
 * demo overlay had to settle both in one currency. See the `providers:`
 * block `gen-demo-keys` generates.
 */
export const CURRENCY = "xaf";

/** 5 000 FCFA — XAF is zero-decimal, and the wire is integer minor units. */
export const AMOUNT = 5000;

/**
 * The MSISDN every case here confirms with — the same one
 * `examples/merchant-node` and `sdks/nodejs/README.md` print, so a merchant
 * reading either meets one number.
 *
 * Deliberately **not** one of the numbers the MTN stub keys a mapping on.
 * `237600000400` provokes a `PAYER_NOT_FOUND` decline and `237600000ce0` is
 * `just demo`'s, which enters the `mtn-e2e-poll` scenario and makes the
 * stub answer `PENDING` once before `SUCCESSFUL`
 * (`backends/tests/conformance/wiremock/mtn/mappings/requesttopay-scenario.json`).
 * With no mapping of its own this number takes the catch-all `202` on submit
 * and the catch-all `SUCCESSFUL` on the status query, so the settlement case
 * below normally lands on its first poll — and still lands, one rung later,
 * on a stack a `just demo` has already walked into that scenario.
 */
export const MSISDN = "237670000000";

/**
 * How stripe-node is configured against vpay, in the exact shape the docs
 * tell merchants to use.
 *
 * `new Stripe("", { authenticator })` is supported: stripe-node throws only
 * if *both* a key and an authenticator are supplied, or neither.
 * `host`/`port`/`protocol` are the three config properties that move every
 * request off `api.stripe.com`; `basePath` is fixed at `/v1/` and is not
 * configurable, which is moot because the generated resources hardcode
 * absolute `/v1/...` paths and those are exactly vpay's.
 */
export function stripeClient(
  overrides: Partial<Stripe.StripeConfig> = {},
): Stripe {
  const env = readCompatEnv();
  const authenticator = createStripeAuthenticator({
    baseUrl: env.baseUrl,
    clientId: env.clientId,
    privateKey: env.privateKeyPem,
  });
  return new Stripe("", {
    authenticator,
    host: env.host,
    port: env.port,
    protocol: env.protocol,
    telemetry: false,
    ...overrides,
  });
}

/**
 * A client whose authenticator writes a bearer token vpay will not accept.
 *
 * Deliberately a *well-formed request with a bad credential* rather than a
 * broken handshake: a handshake that rejects never settles through
 * stripe-node (a defect `@vaam-apps/vpay-sdk`'s README documents and pins), so it is
 * not a thing a conformance suite can await. What a merchant with an expired
 * or revoked credential actually meets is this — a `401` from `/v1` — and
 * that is what the case asserts.
 */
export function stripeClientWithBadCredential(): Stripe {
  const env = readCompatEnv();
  return new Stripe("", {
    // stripe-node's `Authenticator` type is promise-returning; this one only
    // sets a header.
    // eslint-disable-next-line @typescript-eslint/require-await
    authenticator: async (request) => {
      request.headers["Authorization"] = "Bearer not-a-vpay-access-token";
    },
    host: env.host,
    port: env.port,
    protocol: env.protocol,
    telemetry: false,
  });
}

/**
 * The four Stripe fields that say money should move somewhere else, or at a
 * different time, than it otherwise would — and which vpay therefore refuses
 * with a `400` naming the field rather than ignoring
 * (`vpay_api::v1::payment_intents::UnsupportedStripeParams`).
 *
 * Declared once and spread into both `CreateArgs` and {@link ConfirmArgs},
 * because vpay refuses them on **both** POST bodies and a suite that only
 * covered create would leave the confirm path asserted nowhere.
 *
 * `capture_method: "automatic"` is the one accepted value: it asks for
 * exactly what vpay does.
 */
export interface MoneyMovingArgs {
  /** Only `automatic` is accepted; vpay has no authorise/capture split. */
  capture_method?: string;
  /** Stripe Connect: a fee taken out of the payment. */
  application_fee_amount?: number;
  /** Stripe Connect: settle to a different account. */
  transfer_data?: { destination: string };
  /** Stripe Connect: act on behalf of a different account. */
  on_behalf_of?: string;
}

/** `paymentIntents.create` params, before the cast below. */
export interface CreateArgs extends MoneyMovingArgs {
  amount?: number;
  currency?: string;
  payment_method_types?: string[];
  metadata?: Record<string, string>;
  description?: string;
  /**
   * Stripe's expansion request, which vpay accepts and ignores. stripe-node
   * encodes it **indexed** (`expand[0]=…`), which is the spelling the
   * compat case proves survives the round trip.
   */
  expand?: string[];
  /** Stripe's confirm-on-create, which vpay refuses. Only the error case sends it. */
  confirm?: boolean;
}

/** `paymentIntents.confirm` params this suite varies, before the cast below. */
export interface ConfirmArgs extends MoneyMovingArgs {
  /** The payer's number. Defaults to {@link MSISDN}. */
  msisdn?: string;
}

/**
 * Creates a payment intent with this suite's defaults.
 *
 * The cast is on `payment_method_types`: stripe-node types it as a list of
 * Stripe's own method codes, and `mtn_momo` is a vpay rail code. Nothing
 * about the request changes — the wire is `payment_method_types[0]=mtn_momo`
 * either way.
 */
export function createIntent(
  stripe: Stripe,
  args: CreateArgs = {},
  options: Stripe.RequestOptions = {},
): Promise<Stripe.Response<Stripe.PaymentIntent>> {
  const params = {
    amount: AMOUNT,
    currency: CURRENCY,
    payment_method_types: [RAIL],
    ...args,
  } as unknown as Stripe.PaymentIntentCreateParams;
  return stripe.paymentIntents.create(params, options);
}

/**
 * Confirms an intent against the push rail.
 *
 * The cast is on `payment_method_data`: vpay reads
 * `payment_method_data[type]` as a rail code and
 * `payment_method_data[<type>][msisdn]` as the payer's number, neither of
 * which exists in Stripe's schema. stripe-node encodes the nested object
 * into exactly those bracketed keys, which is why no custom serialization is
 * needed — only a cast.
 */
export function confirmIntent(
  stripe: Stripe,
  id: string,
  args: ConfirmArgs = {},
  options: Stripe.RequestOptions = {},
): Promise<Stripe.Response<Stripe.PaymentIntent>> {
  const { msisdn = MSISDN, ...rest } = args;
  const params = {
    payment_method_data: { type: RAIL, [RAIL]: { msisdn } },
    ...rest,
  } as unknown as Stripe.PaymentIntentConfirmParams;
  return stripe.paymentIntents.confirm(id, params, options);
}

/**
 * Runs `fn` and returns the error it threw, failing if it did not throw.
 *
 * `expect(...).rejects` would do, but every error case here asserts four or
 * five fields on the thrown object and reads better with the error in hand.
 */
export async function caught(fn: () => Promise<unknown>): Promise<unknown> {
  try {
    await fn();
  } catch (error) {
    return error;
  }
  throw new Error("expected the call to reject, and it resolved");
}

/**
 * {@link caught}, narrowed to the class a `400` with a Stripe envelope must
 * arrive as.
 *
 * The narrowing is a real assertion, not a convenience: `generateV1Error`
 * picks the error *class* from the status first, so a refusal that lost its
 * envelope, or that came back as some other status, lands in a different
 * class and this throws instead of letting a `param` assertion read
 * `undefined` off the wrong object.
 */
export async function invalidRequest(
  fn: () => Promise<unknown>,
): Promise<Stripe.errors.StripeInvalidRequestError> {
  const error = await caught(fn);
  if (!(error instanceof Stripe.errors.StripeInvalidRequestError)) {
    throw new Error(
      `expected a StripeInvalidRequestError, got ${String(error)}`,
      { cause: error },
    );
  }
  return error;
}
