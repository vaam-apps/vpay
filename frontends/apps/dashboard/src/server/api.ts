/**
 * The shapes vpay answers in, the one shape a refusal becomes, and the calls
 * the sign-in makes.
 *
 * # What this file is (VPA-5a)
 *
 * It was every call the app made to vpay until VP-6a (ADR-0036) deleted the
 * sign-in routes and left the object types and {@link ApiFailure}. VPA-5a
 * restores the calls — the five staff routes, the authorization request and
 * the token exchange — and nothing else: the `/dash/v1` reads that return
 * payments are a later ticket, and the types for them stay as they were.
 *
 * Every call goes through `dash-fetch.ts`, which attaches exactly one
 * credential (a session token for the staff routes) and forwards nothing the
 * browser sent.
 *
 * **Server-side only.** `frontends/apps/dashboard` is the OAuth client
 * (ADR-0017 decision 4): it holds the session cookie on its own origin and the
 * `/dash/v1` access token passes through this process. Nothing in this file is
 * reachable from a browser bundle, and nothing it returns carries a token to
 * one. (No `server-only` import: the package throws under any condition but
 * `react-server`, which would make this module's own suite unrunnable. What
 * keeps it server-side is that every importer is a server component or a
 * server action.)
 *
 * # Why the request id is threaded through everything
 *
 * vpay emits `request-id` (Stripe's spelling) and `x-request-id` with one
 * value on every response, and `vpay-api`'s error envelope deliberately
 * carries no `request_id` field because the header already does. A staff
 * member who cannot sign in has nothing else to quote to an operator, and
 * every refusal on the sign-in path is the same sentence by design
 * (`docs/flows/dashboard-auth/human-login.md`, "Every refusal is one answer")
 * — so the id is the only thing that distinguishes one failure from another.
 * It is carried on the failure and rendered beside the message.
 */
import {
  dashFetch,
  SSO_CALLBACK_TIMEOUT_MS,
  sessionCredential,
  type Credential,
} from "./dash-fetch";

/** The two spellings vpay emits, in the order this app prefers them. */
const REQUEST_ID_HEADERS = ["request-id", "x-request-id"] as const;

/** A call that did not succeed, in the words a screen can render. */
export interface ApiFailure {
  /** HTTP status. `401` is the one every caller branches on. */
  readonly status: number;
  /** `error.message` from the envelope, or a sentence written here. */
  readonly message: string;
  /** `request-id`, or `null` if the response carried none. */
  readonly requestId: string | null;
  /**
   * `error.param` — the request parameter vpay says was unusable
   * (`ApiError::InvalidParam`) — when the envelope named one.
   *
   * Absent for every other refusal, and for a failure this app decided on its
   * own. It is how `changePassword` knows a too-short new password is about
   * the new password and not about the one in force.
   */
  readonly param?: string;
}

/** Success or {@link ApiFailure} — never a thrown error for a refusal. */
export type ApiResult<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly failure: ApiFailure };

/** `POST /dash/v1/staff/login`'s answer (`vpay_api::staff::LoginResponse`). */
export interface LoginResponse {
  readonly session: string;
  readonly next: "totp" | "totp_enrolment";
  readonly otpauth_uri?: string;
  readonly enrolment?: string;
}

/** `POST /dash/v1/staff/totp`'s answer. */
export interface TotpResponse {
  readonly password_change_required: boolean;
}

/** `GET /dash/v1/staff/session`'s answer (`vpay_api::staff::SessionResponse`). */
export interface SessionResponse {
  readonly staff_id: string;
  readonly display_name: string;
  readonly email: string;
  readonly merchant_id: string;
  readonly password_change_required: boolean;
  /** Absent until the code exchange has minted one. */
  readonly access_token?: string;
  /**
   * When {@link SessionResponse.access_token} stops being accepted, RFC 3339.
   * Absent exactly when the token is.
   *
   * Read from `staff_sessions.access_token_expires_at` (migration 0040) and
   * **not** from the JWT's own `exp`: this app holds the token and presents
   * it, it does not verify it, and reading a claim out of an unverified
   * credential is a habit worth not having here.
   */
  readonly access_token_expires_at?: string;
  /**
   * How long a freshly minted token lives, in seconds
   * (`staff_auth.access_token_ttl_seconds`).
   *
   * The denominator of the re-mint margin — see `gate.ts`'s
   * `REMINT_AFTER_FRACTION`, which is a fraction precisely because this is
   * configuration.
   */
  readonly access_token_ttl_seconds: number;
}

/**
 * `GET /dash/v1/staff/session/stage`'s answer
 * (`vpay_api::staff::SessionStageResponse`).
 *
 * The one read that answers for a session **mid-sign-in**. `/staff/session`
 * refuses one that has not presented a code — the same `401` it answers for a
 * session that is over — so this is what lets `/login/totp` tell "you mistyped
 * your code" from "your session is gone".
 *
 * One field, deliberately: a caller at this stage has proved a password and
 * no second factor, so vpay publishes nothing about the person here.
 */
export interface SessionStageResponse {
  readonly stage: "pending_totp" | "authenticated";
}

/**
 * `POST /dash/v1/oauth/token`'s answer.
 *
 * There is no `refresh_token` here and none is ever asked for: the grant
 * serves `authorization_code` alone (ADR-0037), and a token near the end of
 * its life is replaced by running the authorization request again from the
 * session (`gate.ts`).
 */
export interface TokenResponse {
  readonly access_token: string;
  readonly token_type: string;
  readonly expires_in: number;
  readonly scope: string;
}

/** One `payment_intent`, as `/dash/v1` renders it (no `client_secret`). */
export interface PaymentIntentObject {
  readonly id: string;
  readonly object: "payment_intent";
  readonly amount: number;
  readonly currency: string;
  readonly status: string;
  readonly payment_method_types: readonly string[];
  readonly last_payment_error: {
    readonly code?: string;
    readonly message?: string;
  } | null;
  readonly description: string | null;
  readonly customer: string | null;
  readonly created: number;
  readonly livemode: boolean;
}

/** `GET /dash/v1/payment_intents`' envelope. */
export interface PaymentIntentList {
  readonly object: string;
  readonly data: readonly PaymentIntentObject[];
  readonly has_more: boolean;
}

/** The charge, as `/dash/v1`'s detail read renders it. */
export interface ChargeSummary {
  readonly object: "charge";
  readonly id: string;
  readonly provider_code: string;
  readonly provider_reference_id: string;
  readonly provider_txn_id: string | null;
  readonly state: string;
  readonly amount: number;
  readonly currency: string;
  /**
   * `null` on every row this deployment has ever written — nothing writes
   * `charges.payer_ref_masked` (`docs/status.md`). Rendered from the column
   * and never derived; see `src/components/payment-detail.tsx`.
   */
  readonly payer_ref_masked: string | null;
  readonly failure_code: string | null;
  readonly failure_raw: string | null;
  readonly created: number;
  readonly updated: number;
}

/** One refund, as `/v1` renders it. */
export interface RefundObject {
  readonly id: string;
  readonly amount: number;
  readonly currency: string;
  readonly status: string;
  readonly reason: string | null;
  readonly created: number;
}

/** One line of the timeline. */
export interface TimelineEvent {
  readonly object: "event";
  readonly id: string;
  readonly type: string;
  readonly object_id: string;
  readonly created: number;
}

/** `GET /dash/v1/payment_intents/{id}`'s envelope. */
export interface PaymentDetail {
  readonly object: "dashboard.payment_detail";
  readonly payment_intent: PaymentIntentObject;
  readonly charge: ChargeSummary | null;
  readonly refunds: readonly RefundObject[];
  readonly events: readonly TimelineEvent[];
}

/** The request id from either spelling, or `null`. */
export function requestIdOf(response: Response): string | null {
  for (const header of REQUEST_ID_HEADERS) {
    const value = response.headers.get(header);
    if (value !== null && value.trim().length > 0) {
      return value.trim();
    }
  }
  return null;
}

/**
 * `error.message` out of an already-decoded vpay envelope, or the sentence
 * naming the status.
 *
 * A body this app cannot parse is **not** rendered raw: an upstream proxy's
 * HTML error page in an alert is both unreadable and a disclosure of what sits
 * in front of vpay.
 */
export function envelopeMessage(status: number, body: unknown): string {
  const envelope = (body as { error?: { message?: unknown } } | null)?.error;
  if (
    typeof envelope?.message === "string" &&
    envelope.message.trim().length > 0
  ) {
    return envelope.message.trim();
  }
  return `vpay answered ${status}.`;
}

/**
 * A network failure, as an {@link ApiFailure} rather than a thrown error.
 *
 * `status: 0` because there was no response. Every caller already branches on
 * the failure shape, and a `fetch` that rejected inside a server component
 * would otherwise render Next's own error page — which tells a staff member
 * nothing and an operator less.
 */
export function unreachable(error: unknown): ApiFailure {
  const detail = error instanceof Error ? error.message : String(error);
  return {
    status: 0,
    message: `vpay could not be reached (${detail}).`,
    requestId: null,
  };
}

/** `error.message` and the id out of a refused response. */
async function failureOf(response: Response): Promise<ApiFailure> {
  let body: unknown = null;
  try {
    body = await response.json();
  } catch {
    // Left as the status sentence `envelopeMessage` falls back to.
  }
  const param = (body as { error?: { param?: unknown } } | null)?.error?.param;
  return {
    status: response.status,
    message: envelopeMessage(response.status, body),
    requestId: requestIdOf(response),
    ...(typeof param === "string" && param.length > 0 ? { param } : {}),
  };
}

/** The JSON body of an answered call, or the failure it became. */
async function settle<T>(response: Response): Promise<ApiResult<T>> {
  if (!response.ok) {
    return { ok: false, failure: await failureOf(response) };
  }
  try {
    return { ok: true, value: (await response.json()) as T };
  } catch (error) {
    return { ok: false, failure: unreachable(error) };
  }
}

/** What a call may carry beyond its path and body: the one credential, and a deadline. */
export interface CallOptions {
  readonly credential?: Credential;
  /** A deadline other than the default; see `DashRequest.timeoutMs`. */
  readonly timeoutMs?: number;
}

/**
 * `POST` a form-encoded body.
 *
 * Form-encoded and not JSON because that is what the handlers take: every
 * staff route and the token endpoint extract `axum::Form`, and a JSON body
 * to one of them is a `415` that says nothing about the credential.
 */
export async function postForm<T>(
  apiBaseUrl: string,
  path: string,
  body: Readonly<Record<string, string>>,
  options: CallOptions = {},
): Promise<ApiResult<T>> {
  let response: Response;
  try {
    response = await dashFetch(`${apiBaseUrl}${path}`, {
      method: "POST",
      form: body,
      ...(options.credential === undefined
        ? {}
        : { credential: options.credential }),
      ...(options.timeoutMs === undefined
        ? {}
        : { timeoutMs: options.timeoutMs }),
    });
  } catch (error) {
    return { ok: false, failure: unreachable(error) };
  }
  return settle<T>(response);
}

/**
 * `POST /dash/v1/staff/logout`: deletes the `staff_sessions` row `token`
 * names, which is what revokes it (ADR-0017 decision 2).
 *
 * The one call sign-out makes, and the one the single-sign-on callback makes
 * for the session it is about to replace. Idempotent at vpay: a token that
 * was never there answers `200`.
 */
export function revokeSession(
  apiBaseUrl: string,
  token: string,
  options: Pick<CallOptions, "timeoutMs"> = {},
): Promise<ApiResult<{ signed_out: boolean }>> {
  return postForm<{ signed_out: boolean }>(
    apiBaseUrl,
    "/dash/v1/staff/logout",
    {},
    { ...options, credential: sessionCredential(token) },
  );
}

/** `GET` a JSON document. */
export async function getJson<T>(
  apiBaseUrl: string,
  path: string,
  options: CallOptions = {},
): Promise<ApiResult<T>> {
  let response: Response;
  try {
    response = await dashFetch(`${apiBaseUrl}${path}`, {
      method: "GET",
      ...(options.credential === undefined
        ? {}
        : { credential: options.credential }),
    });
  } catch (error) {
    return { ok: false, failure: unreachable(error) };
  }
  return settle<T>(response);
}

/**
 * `GET /dash/v1/oauth/authorize`, **without following the redirect**.
 *
 * `redirect: 'manual'` is the whole of ADR-0017 decision 4 in one option:
 * the `302`'s `Location` is read here, in this process, and the code in it
 * never reaches a browser. `fetch`'s default would follow it to a URL nothing
 * serves.
 *
 * Returns the `Location` on a `302`. Anything else — including a `200`, which
 * would mean the endpoint answered something other than an authorization
 * response — is a failure.
 */
export async function authorizeRedirect(
  apiBaseUrl: string,
  query: Readonly<Record<string, string>>,
  sessionToken: string,
): Promise<ApiResult<string>> {
  const search = new URLSearchParams(query).toString();
  let response: Response;
  try {
    response = await dashFetch(
      `${apiBaseUrl}/dash/v1/oauth/authorize?${search}`,
      {
        method: "GET",
        credential: sessionCredential(sessionToken),
        redirect: "manual",
      },
    );
  } catch (error) {
    return { ok: false, failure: unreachable(error) };
  }
  const location = response.headers.get("location");
  if (response.status !== 302 || location === null) {
    if (!response.ok) {
      return { ok: false, failure: await failureOf(response) };
    }
    return {
      ok: false,
      failure: {
        status: response.status,
        message: "The authorization endpoint did not answer with a redirect.",
        requestId: requestIdOf(response),
      },
    };
  }
  return { ok: true, value: location };
}

/**
 * `GET /dash/v1/staff/sso`'s answer, as the sign-in page reads it
 * (`vpay_api::staff::sso::SsoStatusResponse`).
 *
 * **Three outcomes, and the third is not the second.** `disabled` is vpay
 * saying this deployment writes no `staff_auth.sso` block; `unreadable` is
 * this app not knowing. The page renders a button for the first, nothing for
 * the second, and a sentence saying SSO could not be checked for the third —
 * hiding the button because a read failed would send a staff member who
 * should be using it to a password they may not have (VPA-5b).
 */
export type SsoStatus =
  | { readonly kind: "enabled" }
  | { readonly kind: "disabled" }
  | { readonly kind: "unreadable"; readonly failure: ApiFailure };

/**
 * Whether the login page should offer the single sign-on button.
 *
 * Unauthenticated: nobody is signed in yet, and the request carries no
 * credential at all. Only a literal boolean `enabled` is an answer —
 * `"true"`, `1`, an array and a missing field are a body this app cannot
 * read, which is `unreadable` and never `disabled`. A `404` is `unreadable`
 * too: the page only asks after `human_login` was seen served, and a vpay that
 * serves that method mounts this route, so a `404` is a proxy or a version
 * mismatch worth telling the person about.
 */
export async function getSsoStatus(apiBaseUrl: string): Promise<SsoStatus> {
  const result = await getJson<unknown>(apiBaseUrl, "/dash/v1/staff/sso");
  if (!result.ok) {
    return { kind: "unreadable", failure: result.failure };
  }
  const enabled = (result.value as { enabled?: unknown } | null)?.enabled;
  if (typeof enabled !== "boolean") {
    return {
      kind: "unreadable",
      failure: malformed("an SSO status in a shape"),
    };
  }
  return enabled ? { kind: "enabled" } : { kind: "disabled" };
}

/**
 * `POST /dash/v1/staff/sso/start`'s answer.
 *
 * **`session` is a pending-flow token, not a session.** Only the callback
 * accepts it; every other route refuses it, so a cookie holding it signs
 * nobody in.
 */
export interface SsoStartResponse {
  readonly session: string;
  readonly authorization_url: string;
}

/**
 * `POST /dash/v1/staff/sso/callback`'s answer — one of two outcomes.
 *
 * Either way `session` is a **new** token, and the cookie must be replaced by
 * it: after `signed_in` it is the authenticated session, after
 * `reauthenticate` it is the pending-flow token for the single `prompt=login`
 * retry vpay started. The token the callback was called with is never reused.
 */
export type SsoCallbackResponse =
  | { readonly outcome: "signed_in"; readonly session: string }
  | {
      readonly outcome: "reauthenticate";
      readonly session: string;
      readonly authorization_url: string;
    };

/** A body this app could not trust, as a failure rather than a thrown error. */
function malformed(what: string): ApiFailure {
  return {
    status: 200,
    message: `vpay answered ${what} this app could not read.`,
    requestId: null,
  };
}

/** `true` for a non-empty string; the only thing a token is allowed to be. */
function nonEmpty(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

/**
 * Starts a federated sign-in: asks vpay for a pending-flow token and the
 * identity provider's authorization URL.
 *
 * No body and **no credential** — nobody is signed in yet, and a session
 * header here would be a second door for no reason. A `401` is vpay's uniform
 * refusal, `429` the rate limit, `404` "SSO is not configured"; all three
 * arrive as the ordinary {@link ApiFailure} with `status` set, and the caller
 * turns every one of them into the same generic redirect.
 */
export async function ssoStart(
  apiBaseUrl: string,
): Promise<ApiResult<SsoStartResponse>> {
  const result = await postForm<unknown>(
    apiBaseUrl,
    "/dash/v1/staff/sso/start",
    {},
  );
  if (!result.ok) {
    return result;
  }
  const body = result.value as {
    session?: unknown;
    authorization_url?: unknown;
  } | null;
  if (!nonEmpty(body?.session) || !nonEmpty(body?.authorization_url)) {
    return { ok: false, failure: malformed("a start response") };
  }
  return {
    ok: true,
    value: {
      session: body.session,
      authorization_url: body.authorization_url,
    },
  };
}

/**
 * Hands the identity provider's `code` and `state` to vpay, with the
 * pending-flow token as the one credential, a session.
 *
 * This is the **only** place those two values are sent anywhere. The body is
 * form-encoded like every other staff call; nothing is logged and the values
 * are not echoed into the failure — `failureOf` reads vpay's own envelope,
 * never the request.
 */
export async function ssoCallback(
  apiBaseUrl: string,
  pendingToken: string,
  code: string,
  state: string,
): Promise<ApiResult<SsoCallbackResponse>> {
  const result = await postForm<unknown>(
    apiBaseUrl,
    "/dash/v1/staff/sso/callback",
    { code, state },
    {
      credential: sessionCredential(pendingToken),
      timeoutMs: SSO_CALLBACK_TIMEOUT_MS,
    },
  );
  if (!result.ok) {
    return result;
  }
  const body = result.value as {
    outcome?: unknown;
    session?: unknown;
    authorization_url?: unknown;
  } | null;
  if (body?.outcome === "signed_in" && nonEmpty(body.session)) {
    return { ok: true, value: { outcome: "signed_in", session: body.session } };
  }
  if (
    body?.outcome === "reauthenticate" &&
    nonEmpty(body.session) &&
    nonEmpty(body.authorization_url)
  ) {
    return {
      ok: true,
      value: {
        outcome: "reauthenticate",
        session: body.session,
        authorization_url: body.authorization_url,
      },
    };
  }
  return { ok: false, failure: malformed("a callback response") };
}
