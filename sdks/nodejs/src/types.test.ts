/**
 * Type-level regression test for `exactOptionalPropertyTypes`.
 *
 * This repo compiles with `exactOptionalPropertyTypes` (tsconfig.base.json),
 * and so, in all likelihood, do consumers. Under it, a property declared
 * `kid?: string` accepts a *missing* property but rejects an explicitly
 * `undefined` one — so `{ kid: maybeKid }` with `maybeKid: string | undefined`
 * does not compile, and every consumer has to write a conditional spread for
 * an option that is simply optional. Declaring `kid?: string | undefined`
 * accepts both.
 *
 * The assignments below are the actual assertion: they are checked by
 * `pnpm --filter @vaam-apps/vpay-sdk typecheck`, not by vitest (which strips types
 * without checking them). Reverting any `| undefined` in the option types
 * fails that command. The runtime `it` blocks exist so vitest does not error
 * on a file with no tests, and so the values are at least constructed.
 */
import { describe, expect, it } from "vitest";
import type { VpayClientOptions } from "./client.js";
import { generateTestRsaKeyPair } from "./testing/keys.js";
import {
  isCheckoutSessionEvent,
  isPaymentIntentEvent,
  isRefundEvent,
} from "./types.js";
import type {
  CheckoutSession,
  CreatePaymentIntentParams,
  CreateRefundParams,
  Event,
  KnownEventType,
  ListEventsParams,
  ListParams,
  RequestOptions,
} from "./types.js";

const { privateKey } = generateTestRsaKeyPair();

/**
 * Stands in for a value a consumer read from configuration or the
 * environment: typed `T | undefined`, and opaque enough that TypeScript
 * cannot narrow it back to `T` at the use site.
 */
function configured<T>(value: T | undefined): T | undefined {
  return value;
}

const maybeString = configured<string>(undefined);
const maybeNumber = configured<number>(undefined);
const maybeFetch = configured<typeof fetch>(undefined);
const maybeMetadata = configured<Record<string, string>>(undefined);

const clientOptions: VpayClientOptions = {
  baseUrl: "https://api.vpay.example",
  clientId: "merchant_a",
  privateKey,
  kid: maybeString,
  issuer: maybeString,
  tokenEndpoint: maybeString,
  audience: maybeString,
  scope: maybeString,
  assertionLifetimeSeconds: maybeNumber,
  timeoutMs: maybeNumber,
  fetch: maybeFetch,
};

const requestOptions: RequestOptions = { idempotencyKey: maybeString };

const createIntent: CreatePaymentIntentParams = {
  amount: 5000,
  currency: "xaf",
  payment_method_types: ["mtn_momo"],
  metadata: maybeMetadata,
  description: maybeString,
};

const listParams: ListParams = {
  limit: maybeNumber,
  starting_after: maybeString,
  ending_before: maybeString,
};

const listEventsParams: ListEventsParams = {
  limit: maybeNumber,
  starting_after: maybeString,
  ending_before: maybeString,
  type: maybeString,
};

const createRefund: CreateRefundParams = {
  payment_intent: "pi_123",
  amount: maybeNumber,
  reason: maybeString,
  metadata: maybeMetadata,
};

describe("public option types under exactOptionalPropertyTypes", () => {
  it("accepts an explicitly undefined value for every optional property", () => {
    // If this file compiles, the assertion has already been made. These
    // checks only confirm the declarations above were evaluated.
    expect(clientOptions.clientId).toBe("merchant_a");
    expect(requestOptions).toBeDefined();
    expect(createIntent.amount).toBe(5000);
    expect(listParams).toBeDefined();
    expect(listEventsParams).toBeDefined();
    expect(createRefund.payment_intent).toBe("pi_123");
  });
});

/**
 * The event body a merchant's webhook handler is actually handed for an
 * expired Checkout Session — `status` already `expired`, `url` null and no
 * `client_secret` member, exactly as
 * `vpay_api::model::CheckoutSessionObject::expired_snapshot` renders it.
 *
 * Built as a `CheckoutSession` rather than an untyped literal so the shape is
 * checked by `pnpm --filter @vaam-apps/vpay-sdk typecheck` as well as by vitest.
 */
const expiredSession: CheckoutSession = {
  id: "cs_1",
  object: "checkout.session",
  livemode: false,
  payment_intent: "pi_1",
  ui_mode: "hosted",
  status: "expired",
  payment_status: "unpaid",
  success_url: "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
  cancel_url: "https://shop.example/cancel",
  return_url: null,
  url: null,
  expires_at: 1_700_086_400,
  created: 1_700_000_000,
};

const sessionExpired: Event = {
  id: "evt_9",
  object: "event",
  type: "checkout.session.expired",
  created: 1_753_401_600,
  livemode: false,
  data: { object: expiredSession },
};

/**
 * `checkout.session.expired` is a member of the exported vocabulary. This is
 * a *type-level* assertion checked by `typecheck`: widen or remove the union
 * member and this assignment stops compiling.
 */
const knownType: KnownEventType = "checkout.session.expired";

describe("checkout.session.expired", () => {
  it("is a member of KnownEventType and narrows with isCheckoutSessionEvent", () => {
    expect(knownType).toBe("checkout.session.expired");
    expect(isCheckoutSessionEvent(sessionExpired)).toBe(true);
    if (!isCheckoutSessionEvent(sessionExpired)) {
      throw new Error("the guard must narrow this event");
    }
    // Inside the narrowing, `data.object` is a CheckoutSession at the type
    // level as well as at runtime — reading these members is the assertion.
    expect(sessionExpired.data.object.id).toBe("cs_1");
    expect(sessionExpired.data.object.status).toBe("expired");
  });

  it("carries no client_secret and a null url, so a webhook body holds no payer credential", () => {
    // Asserted on the serialised body, not only on the object: a delivered
    // event is bytes a merchant stores, replays and logs.
    const body = JSON.stringify(sessionExpired);
    expect(body).not.toContain("_secret_");

    if (!isCheckoutSessionEvent(sessionExpired)) {
      throw new Error("the guard must narrow this event");
    }
    const session = sessionExpired.data.object;
    expect(session.url).toBeNull();
    expect("client_secret" in session).toBe(false);
    // …and a null url does not mean the session was embedded. Reading
    // ui_mode is the only way to know that, and this is the assertion that
    // says so.
    expect(session.ui_mode).toBe("hosted");
  });

  it("is not narrowed by the payment-intent or refund guards", () => {
    // A guard that matched everything would make the one above worthless.
    expect(isPaymentIntentEvent(sessionExpired)).toBe(false);
    expect(isRefundEvent(sessionExpired)).toBe(false);
  });

  it("leaves an unknown checkout.session.* type deliverable rather than a failure", () => {
    // `Event.type` is a `string`, not `KnownEventType`, so a type this SDK
    // version predates still decodes and is still readable — and the prefix
    // guard still narrows its payload, which is why it is a prefix.
    const future: Event = { ...sessionExpired, type: "checkout.session.completed" };
    expect(isCheckoutSessionEvent(future)).toBe(true);
    expect(future.type).toBe("checkout.session.completed");
  });
});
