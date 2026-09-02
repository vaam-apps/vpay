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
 * `pnpm --filter @vpay/sdk typecheck`, not by vitest (which strips types
 * without checking them). Reverting any `| undefined` in the option types
 * fails that command. The runtime `it` blocks exist so vitest does not error
 * on a file with no tests, and so the values are at least constructed.
 */
import { describe, expect, it } from "vitest";
import type { VpayClientOptions } from "./client.js";
import { generateTestRsaKeyPair } from "./testing/keys.js";
import type {
  CreatePaymentIntentParams,
  CreateRefundParams,
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
