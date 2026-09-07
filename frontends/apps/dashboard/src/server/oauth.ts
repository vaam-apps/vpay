/**
 * The authorization-code leg, run entirely inside this process.
 *
 * ADR-0017 decision 4, and it is a deviation from the shape a reader assumes,
 * so it is spelled out once here: in a browser-driven flow the user agent
 * follows `/authorize`'s `302` and a callback **page** exchanges the code.
 * Here the app's own server follows it. A browser never sees a code, a
 * verifier or a token.
 *
 * Everything the grant checks is unchanged — the redirect URI is matched byte
 * for byte against the registration, PKCE is mandatory, the code is single
 * use — and what changes is only who follows the redirect. **There is
 * therefore no route in this app at `redirect_uri`**, and there is nothing
 * missing: that string is an identifier the two legs must spell identically,
 * not a page. `docs/flows/dashboard.md` says so where a reader would go
 * looking for it.
 *
 * # Where the token ends up, and why this function returns none
 *
 * `vpay_api::staff::oauth::token` writes the minted token into the
 * `staff_sessions` row before answering. So this app does not keep it: every
 * render reads it back from `GET /dash/v1/staff/session`, which is exactly
 * what makes signing out a revocation — delete the row and there is nowhere
 * left to read it from (ADR-0017 decision 2).
 */
import type { DashboardConfig } from '../config/settings';
import { authorizeRedirect, postForm, type ApiFailure, type ApiResult, type TokenResponse } from './api';
import { CODE_CHALLENGE_METHOD, createPkce, createState } from './pkce';

/**
 * Turns an authenticated staff session into a `/dash/v1` access token, and
 * leaves it on the session row.
 *
 * The verifier is generated here and dies here: it is passed to the two legs
 * as an argument and written nowhere. Removing it from the token exchange
 * makes `vpay_api::staff::oauth::token`'s constant-time PKCE comparison fail
 * and this function return a refusal — `oauth.test.ts` is the guard.
 */
export async function completeAuthorizationCode(
  config: DashboardConfig,
  sessionToken: string,
): Promise<ApiResult<TokenResponse>> {
  const pkce = createPkce();
  const state = createState();

  const redirect = await authorizeRedirect(
    config.apiBaseUrl,
    {
      client_id: config.clientId,
      redirect_uri: config.redirectUri,
      response_type: 'code',
      scope: config.scope,
      state,
      code_challenge: pkce.challenge,
      code_challenge_method: CODE_CHALLENGE_METHOD,
    },
    sessionToken,
  );
  if (!redirect.ok) {
    return redirect;
  }

  const parsed = readAuthorizationResponse(redirect.value, config.redirectUri, state);
  if (!parsed.ok) {
    return parsed;
  }

  return postForm<TokenResponse>(config.apiBaseUrl, '/dash/v1/oauth/token', {
    grant_type: 'authorization_code',
    code: parsed.value,
    // The **same** URI the authorization request named. RFC 6749 §4.1.3, and
    // `token`'s fifth check compares it byte for byte against the code's own
    // record; a value assembled differently here would be refused.
    redirect_uri: config.redirectUri,
    code_verifier: pkce.verifier,
    client_id: config.clientId,
  });
}

/**
 * The `code` out of a `Location`, once the response has been checked.
 *
 * Three checks, and each of them refuses something a bare
 * `new URL(location).searchParams.get('code')` would accept:
 *
 * 1. **the redirect target is the registered URI.** `handle_authorize` never
 *    redirects to an unvalidated URI — an unknown client or an unregistered
 *    URI is `DirectError`, which `vpay_api::staff::oauth::authorize` answers
 *    as a refusal rather than a redirect — so this is belt. It is here
 *    because the alternative is this app following a `Location` it did not
 *    expect and posting a verifier at whatever is behind it.
 * 2. **`state` is the one this call generated.** The only thing that would
 *    catch a `Location` belonging to another request.
 * 3. **an `error` parameter is an error.** An authorization response can
 *    carry `error`/`error_description` instead of `code` (RFC 6749 §4.1.2.1),
 *    and reading `code` off such a URL yields `null` — a failure whose
 *    message would be about a missing code rather than about what the server
 *    actually said.
 */
export function readAuthorizationResponse(
  location: string,
  expectedRedirectUri: string,
  expectedState: string,
): ApiResult<string> {
  let url: URL;
  try {
    url = new URL(location);
  } catch {
    return { ok: false, failure: refusal('The authorization response was not a URL.') };
  }

  const withoutQuery = `${url.origin}${url.pathname}`;
  if (withoutQuery !== stripQuery(expectedRedirectUri)) {
    return {
      ok: false,
      failure: refusal('The authorization response redirected somewhere other than the registered URI.'),
    };
  }

  const error = url.searchParams.get('error');
  if (error !== null) {
    const description = url.searchParams.get('error_description');
    return {
      ok: false,
      failure: refusal(
        description === null
          ? `The authorization request was refused (${error}).`
          : `The authorization request was refused (${error}): ${description}`,
      ),
    };
  }

  if (url.searchParams.get('state') !== expectedState) {
    return { ok: false, failure: refusal('The authorization response carried the wrong state.') };
  }

  const code = url.searchParams.get('code');
  if (code === null || code.length === 0) {
    return { ok: false, failure: refusal('The authorization response carried no code.') };
  }
  return { ok: true, value: code };
}

/** Origin plus path, so a registration written with a query still compares. */
function stripQuery(uri: string): string {
  try {
    const url = new URL(uri);
    return `${url.origin}${url.pathname}`;
  } catch {
    return uri;
  }
}

/**
 * A refusal this app decided on its own, with no response behind it.
 *
 * `status: 0` for `api.ts`'s reason — there is no HTTP status to report — and
 * no request id, because the id belongs to the response this function is
 * refusing to trust.
 */
function refusal(message: string): ApiFailure {
  return { status: 0, message, requestId: null };
}
