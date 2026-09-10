/**
 * Reading `/dash/v1` with a token that expires long before the session does.
 *
 * # The fifteen minutes
 *
 * The `/dash/v1` access token lives `staff_auth.access_token_ttl_seconds`,
 * **900** by default — `vpay_api::op::ACCESS_TOKEN_TTL_SECS` until 2026-09-10,
 * when it became configuration so an end-to-end run could cross an expiry at
 * all (issue #88 item 1). A staff session's bounds are ADR-0017 decision 2's:
 * thirty minutes idle, twelve hours absolute. So the credential a page reads
 * with dies a quarter of an hour into a session that has eleven and three
 * quarter hours left, and `requireStaff` minted a token only when the row
 * carried **none** — once one was there it was used until sign-out.
 *
 * Measured on the real stack (exp28 review): a session row holding a token
 * `/dash/v1` refuses renders `/payments` with
 *
 *     The bearer token is invalid, expired, or was not issued for this endpoint.
 *     Request 4c8e220c-86ea-46f3-9731-6c020b863889
 *
 * and the row still holds the same dead token afterwards. That is what a
 * staff member saw fifteen minutes after signing in, on every render, until
 * they signed out and back in. `dashboard.cy.ts` runs in under thirty seconds
 * and never reached it.
 *
 * # Why a re-mint rather than a longer token or a shorter session
 *
 * The authorization-code leg is not a formality this would be routing around:
 * `vpay_api::staff::oauth::authorize` re-reads the staff row and re-checks
 * that the account is active, that `merchant_id` is still the dashboard
 * client's binding, and that `password_change_required` is not set, on **every
 * mint**. Re-minting is therefore *more* checking than carrying one token for
 * twelve hours, not less — and it is the same call `requireStaff` already
 * makes on the first render of a session.
 *
 * Lengthening the token would weaken exactly those checks; shortening the
 * session would sign people out four times an hour. Both are the maintainer's
 * to take if they disagree; this is the option that changes no security
 * property.
 *
 * # This is the fallback now, not the whole answer
 *
 * Since 2026-09-10 `gate.ts` replaces the token **before** it expires, when a
 * fifth of its life is left, so a render normally never reaches the `401`
 * below. This path stays for what a margin cannot see: a clock that disagrees
 * with vpay's, a token revoked mid-render, a render that arrives late.
 *
 * # Once, and then the failure is the answer
 *
 * A second `401` after a fresh mint is not an expiry — the scope was revoked,
 * the registration changed, the clock is wrong — and retrying it again would
 * be a loop with a credential operation in it. The original refusal is
 * rendered, with its request id.
 *
 * `403` is never retried. `/dash/v1` answers `403` for a token that is valid
 * and not allowed (wrong scope, wrong merchant claim), which a new token from
 * the same registration would answer identically.
 */
import { getJson, type ApiResult } from "./api";
import type { DashboardConfig } from "../config/settings";
import { completeAuthorizationCode } from "./oauth";

/**
 * `GET` a `/dash/v1` document, minting a fresh access token once if the one
 * we hold has expired.
 *
 * Takes the two tokens as arguments rather than reading them, so the whole of
 * it is exercisable against a stubbed `fetch` — `dash-read.test.ts` is what
 * would have caught the fifteen minutes.
 *
 * @param config the settings this container booted with
 * @param path the `/dash/v1` path, query string included
 * @param bearer the access token read from the `staff_sessions` row
 * @param sessionToken the staff session token, to mint a replacement with
 */
export async function readDash<T>(
  config: DashboardConfig,
  path: string,
  bearer: string,
  sessionToken: string,
): Promise<ApiResult<T>> {
  const first = await getJson<T>(config.apiBaseUrl, path, { bearer });
  if (first.ok || first.failure.status !== 401) {
    return first;
  }

  const exchanged = await completeAuthorizationCode(config, sessionToken);
  if (!exchanged.ok) {
    // The session cannot obtain a token at all any more — it was signed out,
    // it went idle, the account was disabled. The read's own refusal is the
    // truer sentence to render than the exchange's, and the next render of
    // any page will take `requireStaff`'s refusal path properly.
    return first;
  }

  return getJson<T>(config.apiBaseUrl, path, {
    bearer: exchanged.value.access_token,
  });
}
