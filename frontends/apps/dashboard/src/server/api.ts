/**
 * Every call this app makes to vpay, and the one shape it turns a refusal
 * into.
 *
 * **Server-side only.** `frontends/apps/dashboard` is the OAuth client
 * (ADR-0017 decision 4): it holds the session cookie on its own origin and
 * keeps the `/dash/v1` access token in this process. Nothing in this file is
 * reachable from a browser bundle, and nothing it returns carries a token.
 *
 * # Why the request id is threaded through everything
 *
 * vpay emits `request-id` (Stripe's spelling) and `x-request-id` with one
 * value on every response, and `vpay-api`'s error envelope deliberately
 * carries no `request_id` field because the header already does. A staff
 * member who cannot sign in has nothing else to quote to an operator, and
 * every refusal on the sign-in path is the same sentence by design
 * (`docs/flows/dashboard-auth.md`, "Every refusal is one answer") — so the
 * id is the only thing that distinguishes one failure from another. It is
 * carried on the failure and rendered beside the message.
 */
// No `server-only` import: the package throws under any condition but
// `react-server`, which would make this module's own vitest suite
// unrunnable. What keeps it server-side is that every importer is a server
// component or a server action, and that nothing here is exported to one
// that is not.

/** The two spellings vpay emits, in the order this app prefers them. */
const REQUEST_ID_HEADERS = ['request-id', 'x-request-id'] as const;

/** A call that did not succeed, in the words a screen can render. */
export interface ApiFailure {
  /** HTTP status. `401` is the one every caller branches on. */
  readonly status: number;
  /** `error.message` from the envelope, or a sentence written here. */
  readonly message: string;
  /** `request-id`, or `null` if the response carried none. */
  readonly requestId: string | null;
}

/** Success or {@link ApiFailure} — never a thrown error for a refusal. */
export type ApiResult<T> = { readonly ok: true; readonly value: T } | { readonly ok: false; readonly failure: ApiFailure };

/** `POST /dash/v1/staff/login`'s answer (`vpay_api::staff::LoginResponse`). */
export interface LoginResponse {
  readonly session: string;
  readonly next: 'totp' | 'totp_enrolment';
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
}

/** `POST /dash/v1/oauth/token`'s answer. */
export interface TokenResponse {
  readonly access_token: string;
  readonly token_type: string;
  readonly expires_in: number;
  readonly scope: string;
}

/** One `payment_intent`, as `/dash/v1` renders it (no `client_secret`). */
export interface PaymentIntentObject {
  readonly id: string;
  readonly object: 'payment_intent';
  readonly amount: number;
  readonly currency: string;
  readonly status: string;
  readonly payment_method_types: readonly string[];
  readonly last_payment_error: { readonly code?: string; readonly message?: string } | null;
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
  readonly object: 'charge';
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
  readonly object: 'event';
  readonly id: string;
  readonly type: string;
  readonly object_id: string;
  readonly created: number;
}

/** `GET /dash/v1/payment_intents/{id}`'s envelope. */
export interface PaymentDetail {
  readonly object: 'dashboard.payment_detail';
  readonly payment_intent: PaymentIntentObject;
  readonly charge: ChargeSummary | null;
  readonly refunds: readonly RefundObject[];
  readonly events: readonly TimelineEvent[];
}

/** The header vpay takes a staff session token in (`vpay_api::staff::SESSION_HEADER`). */
export const SESSION_HEADER = 'x-vpay-staff-session';

/** The request id from either spelling, or `null`. */
function requestIdOf(response: Response): string | null {
  for (const header of REQUEST_ID_HEADERS) {
    const value = response.headers.get(header);
    if (value !== null && value.trim().length > 0) {
      return value.trim();
    }
  }
  return null;
}

/**
 * `error.message` out of vpay's envelope, or a sentence naming the status.
 *
 * A body this app cannot parse is **not** rendered raw: an upstream proxy's
 * HTML error page in an `Alert` is both unreadable and a disclosure of what
 * sits in front of vpay.
 */
async function failureOf(response: Response): Promise<ApiFailure> {
  const requestId = requestIdOf(response);
  let message = `vpay answered ${response.status}.`;
  try {
    const body: unknown = await response.json();
    const envelope = (body as { error?: { message?: unknown } } | null)?.error;
    if (typeof envelope?.message === 'string' && envelope.message.trim().length > 0) {
      message = envelope.message.trim();
    }
  } catch {
    // Left as the status sentence above.
  }
  return { status: response.status, message, requestId };
}

/** Everything a call may carry beyond its body. */
interface CallOptions {
  /** The opaque staff session token, for the five `/staff/*` routes. */
  readonly sessionToken?: string;
  /** The `/dash/v1` access token, for the two protected reads. */
  readonly bearer?: string;
}

/** The headers `options` implies, and nothing else. */
function authHeaders(options: CallOptions): Record<string, string> {
  const headers: Record<string, string> = {};
  if (options.sessionToken !== undefined) {
    headers[SESSION_HEADER] = options.sessionToken;
  }
  if (options.bearer !== undefined) {
    headers['authorization'] = `Bearer ${options.bearer}`;
  }
  return headers;
}

/**
 * A network failure, as an {@link ApiFailure} rather than a thrown error.
 *
 * `status: 0` because there was no response. Every caller already branches on
 * the failure shape, and a `fetch` that rejected inside a server component
 * would otherwise render Next's own error page — which tells a staff member
 * nothing and an operator less.
 */
function unreachable(error: unknown): ApiFailure {
  const detail = error instanceof Error ? error.message : String(error);
  return {
    status: 0,
    message: `vpay could not be reached (${detail}).`,
    requestId: null,
  };
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
    response = await fetch(`${apiBaseUrl}${path}`, {
      method: 'POST',
      headers: {
        'content-type': 'application/x-www-form-urlencoded',
        accept: 'application/json',
        ...authHeaders(options),
      },
      body: new URLSearchParams(body).toString(),
      // Every one of these is a credential step. A cached sign-in is not a
      // thing that can be allowed to exist.
      cache: 'no-store',
      redirect: 'error',
    });
  } catch (error) {
    return { ok: false, failure: unreachable(error) };
  }
  if (!response.ok) {
    return { ok: false, failure: await failureOf(response) };
  }
  try {
    return { ok: true, value: (await response.json()) as T };
  } catch (error) {
    return { ok: false, failure: unreachable(error) };
  }
}

/** `GET` a JSON document. */
export async function getJson<T>(
  apiBaseUrl: string,
  path: string,
  options: CallOptions = {},
): Promise<ApiResult<T>> {
  let response: Response;
  try {
    response = await fetch(`${apiBaseUrl}${path}`, {
      method: 'GET',
      headers: { accept: 'application/json', ...authHeaders(options) },
      // A dashboard that showed a cached payment would be worse than one
      // that showed none: an operator reads it to decide whether something
      // is still happening.
      cache: 'no-store',
      redirect: 'error',
    });
  } catch (error) {
    return { ok: false, failure: unreachable(error) };
  }
  if (!response.ok) {
    return { ok: false, failure: await failureOf(response) };
  }
  try {
    return { ok: true, value: (await response.json()) as T };
  } catch (error) {
    return { ok: false, failure: unreachable(error) };
  }
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
    response = await fetch(`${apiBaseUrl}/dash/v1/oauth/authorize?${search}`, {
      method: 'GET',
      headers: { accept: 'application/json', [SESSION_HEADER]: sessionToken },
      cache: 'no-store',
      redirect: 'manual',
    });
  } catch (error) {
    return { ok: false, failure: unreachable(error) };
  }
  const location = response.headers.get('location');
  if (response.status !== 302 || location === null) {
    if (!response.ok) {
      return { ok: false, failure: await failureOf(response) };
    }
    return {
      ok: false,
      failure: {
        status: response.status,
        message: 'The authorization endpoint did not answer with a redirect.',
        requestId: requestIdOf(response),
      },
    };
  }
  return { ok: true, value: location };
}
