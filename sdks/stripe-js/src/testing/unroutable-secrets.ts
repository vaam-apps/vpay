/**
 * Client secrets whose id no payer route may turn into a path segment.
 *
 * Every path this package builds — `/v1/browser/payment_intents/{id}`,
 * `…/{id}/confirm`, `/v1/browser/checkout/sessions/{id}` and the embedded
 * frame's `/e/{id}` — takes its id from `parseClientSecret` in `client.ts`,
 * and that parser is the **only** thing keeping `.` and `..` out of the
 * path. `encodeURIComponent` leaves dots alone, and the WHATWG URL parser
 * `fetch` and an `iframe` both use resolves a `..` segment — and a `%2E%2E`
 * one — before anything leaves the page (measured on Node 24, 2026-10-21;
 * the merchant SDKs had exactly this hole until #306). What closes it here
 * is the prefix rule: an id must start with `pi_`/`cs_` and be longer than
 * that prefix, so it can never be a dot segment.
 *
 * Every entry is refused by every route, whichever credential it expects:
 * a `pi_` entry is the wrong prefix on the session route and vice versa, so
 * the bare-prefix and empty-suffix entries are listed for both. Shared by
 * `client.test.ts` and `embedded.test.ts`, so the two cannot drift.
 *
 * Imported only from `*.test.ts`, and excluded from `dist` by
 * `tsconfig.build.json`, like `browser-stub.ts` beside it.
 */
const SUFFIX = "a".repeat(32);

export const UNROUTABLE_CLIENT_SECRETS: ReadonlyArray<
  readonly [name: string, secret: string]
> = [
  ["an empty string", ""],
  ["a bare ..", ".."],
  ["a bare .", "."],
  ["a bare %2e%2e", "%2e%2e"],
  ["a bare pi_", "pi_"],
  ["a bare cs_", "cs_"],
  [".. as the id", `.._secret_${SUFFIX}`],
  [". as the id", `._secret_${SUFFIX}`],
  ["%2e%2e as the id", `%2e%2e_secret_${SUFFIX}`],
  ["%2E%2E as the id", `%2E%2E_secret_${SUFFIX}`],
  ["an empty id", `_secret_${SUFFIX}`],
  ["another object's prefix", `re_123_secret_${SUFFIX}`],
  ["pi_ alone as the id", `pi__secret_${SUFFIX}`],
  ["cs_ alone as the id", "cs__secret_x"],
  ["a pi_ id with an empty suffix", "pi_x_secret_"],
  ["a cs_ id with an empty suffix", "cs_x_secret_"],
];
