/**
 * Shared request-shape validation, run before anything reaches the wire.
 */

/**
 * Mirrors `@vpay/api-client`'s `formatAmount` integer check
 * (frontends/packages/api-client/src/index.ts) and
 * docs/flows/money.md's rule that money is an integer count of a currency's
 * minor unit — no floating point in the money path, ever.
 *
 * `Number.isInteger` alone is not that rule: it accepts `-1`, `1e21` (which
 * `String()` renders as `1e+21`, not a decimal integer) and integers past
 * `Number.MAX_SAFE_INTEGER`, where the value the caller wrote and the value
 * JavaScript holds have already diverged. An amount must be a non-negative
 * **safe** integer or nothing goes to the wire.
 */
export function assertIntegerAmount(amount: number, field = "amount"): void {
  if (!Number.isSafeInteger(amount) || amount < 0) {
    throw new TypeError(
      `${field} must be a non-negative safe integer in minor units, got ${amount}`,
    );
  }
}
