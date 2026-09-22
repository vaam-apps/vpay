/**
 * D6: "`toString()` is overridden on every type that holds a session URL, a
 * session secret or an intent secret, rendering `[N chars redacted]`."
 *
 * One function, so the rendering cannot drift between call sites — the same
 * reason `sdks/flutter/vpay_checkout_flutter/lib/src/redaction.dart` exists
 * and the same reason `sdks/stripe-js/src/client.ts` builds its
 * `[INSPECT_CUSTOM]`/`toJSON` pair from one shared shape.
 *
 * This package's own types hold no secret — a {@link VpayCheckoutResult}
 * carries two public ids and, at most, a server-built error — so nothing
 * here needs it internally. It is exported because the merchant's code is
 * where the session URL actually lives: a Tauri front end that wants to log
 * "started checkout for …" has exactly one safe way to render the string it
 * was handed, and this is it.
 */

/**
 * `[N chars redacted]`, or the literal `null`.
 *
 * `null` and `undefined` both render as `null` rather than as `[0 chars
 * redacted]`, because "there is no secret here" and "there is a zero-length
 * one" are different facts and a reader of a log line should not have to
 * guess which.
 */
export function redacted(value: string | null | undefined): string {
  if (value === null || value === undefined) {
    return "null";
  }
  return `[${String(value.length)} chars redacted]`;
}
