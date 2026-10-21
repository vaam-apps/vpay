/**
 * Shared request-shape validation, run before anything reaches the wire.
 *
 * Every refusal here is a {@link VpayInvalidParamsError}, so it is caught by
 * the same `instanceof VpayError` as every other error this package throws.
 */

import { VpayInvalidParamsError } from "./errors.js";

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
 *
 * @throws {VpayInvalidParamsError} naming `field`.
 */
export function assertIntegerAmount(amount: number, field = "amount"): void {
  if (!Number.isSafeInteger(amount) || amount < 0) {
    throw new VpayInvalidParamsError(
      field,
      `must be a non-negative safe integer in minor units, got ${amount}`,
    );
  }
}

/**
 * The one message {@link pathSegment}'s refusal carries, whatever the id.
 *
 * Fixed so that it cannot quote the id, and byte-identical to `sdks/rust`'s
 * `DOT_SEGMENT_REFUSAL`. {@link VpayInvalidParamsError} puts the same
 * `invalid request parameter id: ` prefix in front of it that crate's
 * `Error::InvalidParams` displays.
 */
const DOT_SEGMENT_REFUSAL =
  "an id that is empty or only dots would address a different endpoint once the URL is parsed; nothing was sent";

/**
 * Percent-encodes one merchant-supplied id for a URL path, and refuses the
 * ids escaping cannot keep in place.
 *
 * `encodeURIComponent` keeps a `/`, `?` or `#` inside its segment, but it
 * leaves `.` alone, so `""`, `.` and `..` pass through, and as whole segments
 * `fetch`'s URL parser resolves them before the request leaves:
 * `customers.del("..")` was a `DELETE /v1/`. Escaping the dots would not
 * help, because WHATWG parsing reads `%2E%2E` as `..` too. So an empty or
 * all-dots id throws before anything is sent, the token request included —
 * `sdks/rust`'s `path_segment` refuses the same ids with
 * `Error::InvalidParams`. `...` is refused by decision rather than by the
 * standard: `docs/reference/rails.md` § "A path segment the caller chose"
 * says why.
 *
 * @throws {VpayInvalidParamsError} naming `id`.
 */
export function pathSegment(id: string): string {
  if (/^\.*$/u.test(id)) {
    throw new VpayInvalidParamsError("id", DOT_SEGMENT_REFUSAL);
  }
  return encodeURIComponent(id);
}
