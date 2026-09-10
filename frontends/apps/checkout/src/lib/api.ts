/**
 * The three `/v1/browser/checkout/*` routes this page reads.
 *
 * Deliberately **not** `@vpay/api-client`: that package is the dashboard's
 * client for `/dash/v1` under an OIDC session (ADR-0008), and this app never
 * holds a merchant credential of any kind. The two payment-intent routes
 * (`confirm` and the poll) are not here either — `@vaam-apps/vpay-stripe-js` owns
 * those, and re-implementing them would be a second client for the same
 * wire contract.
 *
 * Nothing in this module throws or rejects. Every call answers a
 * {@link Result}, and every failure is one of the closed
 * {@link CheckoutErrorCode} values, because a payment page that renders a
 * thrown value renders whatever the network stack happened to put in a
 * message — which, on this surface, is a URL carrying a credential.
 */
import type {
  CheckoutError,
  CheckoutErrorCode,
  CheckoutReturnView,
  CheckoutSessionView,
  Result,
} from "./types";

/** `"/".charCodeAt(0)` — see {@link stripTrailingSlashes}. */
const SLASH_CHAR_CODE = 47;

function stripTrailingSlashes(value: string): string {
  let end = value.length;
  while (end > 0 && value.charCodeAt(end - 1) === SLASH_CHAR_CODE) {
    end -= 1;
  }
  return value.slice(0, end);
}

function failure(
  code: CheckoutErrorCode,
  serverCode?: string,
): { ok: false; error: CheckoutError } {
  const error: CheckoutError = { code };
  if (serverCode !== undefined) {
    error.serverCode = serverCode;
  }
  return { ok: false, error };
}

/** Reads `{ "error": { "code": … } }` — `vpay_api::error_envelope_with_param`. */
function serverErrorCode(body: unknown): string | undefined {
  if (typeof body !== "object" || body === null || !("error" in body)) {
    return undefined;
  }
  const raw: unknown = body.error;
  if (typeof raw !== "object" || raw === null) {
    return undefined;
  }
  const code: unknown = (raw as { code?: unknown }).code;
  return typeof code === "string" ? code : undefined;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/**
 * Narrows a decoded 2xx body to the session envelope.
 *
 * Structural, not exhaustive: it checks the members this page actually
 * reaches for, so a server that adds a key does not break the page and a
 * server that renders something else entirely is reported as
 * `error.unexpected` rather than rendered as a payment.
 *
 * **`merchant` is not among them**, and that is the point of this comment.
 * The check must name only what the page *cannot proceed without*: the
 * session's identity and an expanded intent, because without those there is
 * no amount to show and no rail to drive. The merchant's display name is a
 * nicety — a server that omits it, renames it (`merchant_name`) or renders
 * something that is not `{ name: string }` must not turn a working payment
 * into `error.unexpected`, which is a dead end for the payer and a support
 * ticket for the merchant. `merchantOf` in `machine.ts` reads it
 * defensively instead, and the page shows a neutral heading when there is
 * no name to show.
 */
function isSessionEnvelope(body: unknown): body is CheckoutSessionView {
  if (!isObject(body)) {
    return false;
  }
  const intent: unknown = body["payment_intent"];
  return (
    body["object"] === "checkout.session" &&
    typeof body["id"] === "string" &&
    // Expanded, not an id string. A server that answered with the id would
    // leave this page with an amount it never read, so it is reported as an
    // unexpected response rather than rendered.
    isObject(intent) &&
    intent["object"] === "payment_intent" &&
    typeof intent["id"] === "string"
  );
}

export interface BrowserApiOptions {
  /** The origin `/v1/browser/...` is appended to. Trailing slashes are stripped. */
  baseUrl: string;
  /** Injected `fetch`, for tests and for a host with its own instrumentation. */
  fetch?: typeof fetch | undefined;
}

/**
 * The publishable key and the session credential, as this page received
 * them. Neither is minted, stored or derived here.
 */
export interface SessionCredentials {
  key: string;
  clientSecret: string;
}

export interface ReturnCredentials {
  key: string;
  returnToken: string;
}

export class BrowserCheckoutApi {
  readonly #baseUrl: string;
  readonly #fetchImpl: typeof fetch;

  constructor(options: BrowserApiOptions) {
    this.#baseUrl = stripTrailingSlashes(options.baseUrl.trim());
    const injected = options.fetch;
    this.#fetchImpl =
      injected ??
      (typeof globalThis.fetch === "function"
        ? // Bound: an unbound native `fetch` called as a method of anything
          // but the global throws `Illegal invocation` in a browser.
          globalThis.fetch.bind(globalThis)
        : (() => {
            throw new TypeError(
              "BrowserCheckoutApi: no global fetch is available",
            );
          })());
  }

  /** `GET /v1/browser/checkout/sessions/{id}` — the session, its intent (with the intent's secret) and the merchant name. */
  async readSession(
    sessionId: string,
    credentials: SessionCredentials,
  ): Promise<Result<CheckoutSessionView>> {
    const query = new URLSearchParams({
      key: credentials.key,
      client_secret: credentials.clientSecret,
    });
    return this.#readEnvelope(
      `${this.#sessionUrl(sessionId)}?${query.toString()}`,
    );
  }

  /**
   * `GET /v1/browser/checkout/sessions/{id}/return?t=…` — the return page's
   * only read, and its poll.
   *
   * The response carries the intent **without** its `client_secret`, so this
   * is also the return page's polling endpoint: there is no credential on
   * the return trip that would let it call the payment-intent routes.
   */
  async readReturn(
    sessionId: string,
    credentials: ReturnCredentials,
  ): Promise<Result<CheckoutReturnView>> {
    const query = new URLSearchParams({
      key: credentials.key,
      t: credentials.returnToken,
    });
    return this.#readEnvelope(
      `${this.#sessionUrl(sessionId)}/return?${query.toString()}`,
    );
  }

  #sessionUrl(sessionId: string): string {
    return `${this.#baseUrl}/v1/browser/checkout/sessions/${encodeURIComponent(sessionId)}`;
  }

  async #readEnvelope<T extends CheckoutSessionView | CheckoutReturnView>(
    url: string,
  ): Promise<Result<T>> {
    let ok: boolean;
    let text: string;
    try {
      const response = await this.#fetchImpl(url, {
        method: "GET",
        // Never a cookie and never an `Authorization` header: the query
        // string is the whole credential on this surface, and a same-origin
        // default would start attaching more the day vpay's API and this
        // page share an origin.
        credentials: "omit",
        mode: "cors",
      });
      ok = response.ok;
      text = await response.text();
    } catch {
      // The thrown value is not read. In a browser a `fetch` rejection's
      // `cause` can carry the request URL, and that URL holds the session
      // credential.
      return failure("error.network");
    }

    let parsed: unknown;
    try {
      parsed = text.length > 0 ? JSON.parse(text) : undefined;
    } catch {
      parsed = undefined;
    }

    if (!ok) {
      const code = serverErrorCode(parsed);
      // Every credential failure on the browser surface is the same 404
      // (`resource_missing`), by design — see docs/flows/browser-checkout.md.
      // This page repeats that: it does not tell a guesser which half of the
      // link was wrong.
      return code === "resource_missing"
        ? failure("error.session_not_found", code)
        : failure("error.unexpected", code);
    }
    return isSessionEnvelope(parsed)
      ? { ok: true, value: parsed as T }
      : failure("error.unexpected");
  }
}

/**
 * How long {@link fetchCheckoutOrigins} may hold a page's first byte.
 *
 * Two seconds. The API is a pod away on the same network and answers this
 * route from one indexed read; anything slower is a deployment in trouble,
 * and a payer waiting on it learns nothing they could act on. Overridable per
 * call so the timeout itself is a test rather than a wait.
 */
export const ORIGINS_TIMEOUT_MS = 2_000;

/**
 * `GET /v1/browser/checkout/origins?key` — the tenant's registered embedding
 * origins.
 *
 * A free function, not a method, because the caller is `middleware.ts`
 * running on the server against `VPAY_API_URL`, before any of this page's
 * client state exists. It takes no secret: D4's reasoning is that a
 * merchant's own site origins are public by nature and the publishable key
 * already names the tenant.
 *
 * **Fail-closed.** A non-2xx, an unreachable API, a body that is not the
 * documented shape, a lookup that takes too long and an empty list all answer
 * `[]`, which {@link import('./csp.js').frameAncestors} renders as `'none'`.
 * There is no "assume it is fine" branch: an origins lookup that failed and an
 * origins list that is empty are the same instruction to the browser.
 *
 * **Bounded, and that is load-bearing since 2026-09-06.** This call is on the
 * first byte of a page: `middleware.ts` awaits it before the response starts.
 * It used to run for `/e/{id}` alone; it now runs for `/c/{id}` and
 * `/c/{id}/return` as well, because a hosted page in a popup needs an origin
 * to pin. An API that REFUSES costs those pages no channel and nothing else —
 * that was always true — but an API that neither answers nor closes the socket
 * would have held the payment page open for as long as the operating system's
 * own connect timeout, which is minutes. {@link ORIGINS_TIMEOUT_MS} is the
 * ceiling now, and the timeout lands in the same `[]` every other failure
 * does.
 */
export async function fetchCheckoutOrigins(
  baseUrl: string,
  key: string,
  fetchImpl: typeof fetch,
  timeoutMs: number = ORIGINS_TIMEOUT_MS,
): Promise<string[]> {
  const url = `${stripTrailingSlashes(baseUrl.trim())}/v1/browser/checkout/origins?${new URLSearchParams({ key }).toString()}`;
  // A controller rather than `AbortSignal.timeout`, because the timer has to
  // be cleared: a two-second handle left armed on every request would keep the
  // event loop busy for two seconds after a response that took ten
  // milliseconds.
  const controller = new AbortController();
  const timer = setTimeout(() => {
    controller.abort();
  }, timeoutMs);
  try {
    const response = await fetchImpl(url, {
      method: "GET",
      credentials: "omit",
      signal: controller.signal,
    });
    if (!response.ok) {
      return [];
    }
    // Inside the try and before the timer is cleared on purpose: a body that
    // arrives one byte at a time is the same denial as a request that never
    // answers, and the abort reaches this read too.
    const body: unknown = await response.json();
    if (!isObject(body) || !Array.isArray(body["origins"])) {
      return [];
    }
    return (body["origins"] as unknown[]).filter(
      (o): o is string => typeof o === "string",
    );
  } catch {
    // An `AbortError` from the timeout arrives here with everything else, and
    // answers what everything else answers.
    return [];
  } finally {
    clearTimeout(timer);
  }
}
