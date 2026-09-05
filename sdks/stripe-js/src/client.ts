/**
 * The browser client: `loadStripe` and the `Stripe` object it resolves to.
 *
 * Speaks vpay's browser surface
 * (`docs/plans/2026-09-03-step5c-stripejs.md` §1):
 *
 * - `GET  /v1/browser/payment_intents/{id}?key=…&client_secret=…`
 * - `POST /v1/browser/payment_intents/{id}/confirm` — form-encoded `key`,
 *   `client_secret`, `payment_method_data[…]`, `return_url`
 *
 * There is no `create`, no `list` and no `cancel` on that surface, and no
 * `Idempotency-Key` header: a browser `POST` carrying one would be
 * CORS-preflighted, and one-charge-per-intent is enforced by a unique index
 * server-side instead. Nothing here mints, stores or derives a credential —
 * the `client_secret` arrives from the merchant's own page and is passed
 * straight back on each call.
 */
import {
  CLIENT_ERROR_CODES,
  connectionError,
  parseErrorEnvelope,
  stripeError,
  unexpectedResponseError,
  type StripeError,
} from "./errors.js";
import { createEmbeddedCheckout } from "./embedded.js";
import { encodeForm, FormEncodingError, type FormValue } from "./form.js";
import type {
  CheckoutSession,
  CheckoutSessionResult,
  ConfirmPaymentOptions,
  EmbeddedCheckout,
  InitEmbeddedCheckoutOptions,
  MobileMoneyData,
  PaymentIntent,
  PaymentIntentResult,
  Stripe,
  VpayStripeOptions,
  WaitForPaymentIntentOptions,
} from "./types.js";

/** The separator `vpay_core::ids::client_secret` joins an id and its suffix with. */
const SECRET_SEPARATOR = "_secret_";
/** A payment-intent id. */
const INTENT_ID_PREFIX = "pi_";
/** A checkout-session id (Step 9, D1) — a second credential, same shape. */
const SESSION_ID_PREFIX = "cs_";
/** `"/".charCodeAt(0)`, spelled out once for {@link stripTrailingSlashes}. */
const SLASH_CHAR_CODE = 47;

const DEFAULT_TIMEOUT_MS = 180_000;
const DEFAULT_INTERVAL_MS = 2_000;
/** Poll delays are drawn from `[0.75, 1.25] × intervalMs`, so a page of payers does not poll in lockstep. */
const JITTER_SPREAD = 0.5;

const INSPECT_CUSTOM = Symbol.for("nodejs.util.inspect.custom");

/** A `{ error }` result, built once so every failure path reads the same. */
function errorResult(error: StripeError): PaymentIntentResult {
  return { error };
}

/**
 * Splits `{prefix}abc_secret_xyz` into the object's id.
 *
 * Deliberately not a regex over the suffix alphabet: the suffix's shape is
 * the server's business (32–128 characters, per migration `0023`'s CHECK),
 * and a client-side pattern that disagreed with it would refuse a valid
 * secret after a server change. What *is* checked is only what this package
 * needs to build a URL — that there is a separator, that something precedes
 * it, that it carries the expected id prefix, and that something follows it.
 *
 * Parameterised over the prefix because Step 9's Checkout Session is a
 * **second** credential of exactly this shape (`cs_…_secret_…`, D1), and
 * two near-identical parsers would drift the first time either object's id
 * rules moved. `shape` names the object in the refusal message; the
 * offending value itself is never quoted.
 */
function parseClientSecret(
  clientSecret: unknown,
  prefix: string,
  shape: string,
): { id: string } | { error: StripeError } {
  if (typeof clientSecret !== "string" || clientSecret.length === 0) {
    return {
      error: stripeError(
        "invalid_request_error",
        "invalid_request",
        "clientSecret must be a non-empty string.",
        "clientSecret",
      ),
    };
  }
  const separator = clientSecret.indexOf(SECRET_SEPARATOR);
  const id = separator === -1 ? "" : clientSecret.slice(0, separator);
  const suffix =
    separator === -1
      ? ""
      : clientSecret.slice(separator + SECRET_SEPARATOR.length);
  if (
    separator === -1 ||
    !id.startsWith(prefix) ||
    id.length <= prefix.length ||
    suffix.length === 0
  ) {
    // The malformed value is **not** echoed. It is a credential the payer's
    // page just handed us; a message quoting it would put it in whatever
    // renders `error.message`.
    return {
      error: stripeError(
        "invalid_request_error",
        "invalid_request",
        `clientSecret is not a vpay ${shape} client secret.`,
        "clientSecret",
      ),
    };
  }
  return { id };
}

/** {@link parseClientSecret} for a `pi_…_secret_…`. */
function parseIntentSecret(
  clientSecret: unknown,
): { id: string } | { error: StripeError } {
  return parseClientSecret(clientSecret, INTENT_ID_PREFIX, "payment-intent");
}

/** {@link parseClientSecret} for a `cs_…_secret_…` (Step 9, D1). */
function parseSessionSecret(
  clientSecret: unknown,
): { id: string } | { error: StripeError } {
  return parseClientSecret(clientSecret, SESSION_ID_PREFIX, "checkout-session");
}

/** The URL a `next_action` asks the browser to visit, if this intent has one. */
function redirectTarget(intent: PaymentIntent): string | undefined {
  const nextAction = intent.next_action;
  if (nextAction === null || nextAction.type !== "redirect_to_url") {
    return undefined;
  }
  const url: unknown = nextAction.redirect_to_url.url;
  return typeof url === "string" && url.length > 0 ? url : undefined;
}

/**
 * True only for an absolute `http:`/`https:` URL — the one shape it is safe
 * to hand to `location.assign`.
 *
 * `next_action.redirect_to_url.url` is the rail's own value, echoed back
 * through the server; it is not something this package minted. A
 * `javascript:` URL would execute in the merchant's origin, `data:` and
 * every other scheme are no better, and a relative path — `new URL` throws
 * without a base — would resolve against whatever document happens to be
 * open rather than the rail this intent named. `#followRedirect` refuses
 * every shape but an absolute `http`/`https` URL instead of navigating to it.
 */
function isSafeRedirectUrl(url: string): boolean {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  return parsed.protocol === "http:" || parsed.protocol === "https:";
}

/** Narrows a decoded 2xx body to something that is actually a payment intent. */
function isPaymentIntent(body: unknown): body is PaymentIntent {
  return (
    typeof body === "object" &&
    body !== null &&
    (body as { object?: unknown }).object === "payment_intent" &&
    typeof (body as { id?: unknown }).id === "string"
  );
}

/** Narrows a decoded 2xx body to something that is actually a checkout session. */
function isCheckoutSession(body: unknown): body is CheckoutSession {
  return (
    typeof body === "object" &&
    body !== null &&
    (body as { object?: unknown }).object === "checkout.session" &&
    typeof (body as { id?: unknown }).id === "string"
  );
}

/**
 * `window`, or `undefined` when there is none.
 *
 * This package is typed against `lib.dom`, where `window` is declared as
 * always present; it nonetheless runs under Node in this repo's own tests,
 * in SSR frameworks, and in workers. The `typeof` guard is the only form
 * that does not throw a `ReferenceError` where the binding is absent
 * entirely.
 */
function browserWindow(): Window | undefined {
  return typeof window === "undefined" ? undefined : window;
}

/**
 * Strips every trailing `/` from `value`, with no regex.
 *
 * `options.baseUrl` is caller-supplied — a merchant's own configuration, not
 * something vpay generated — so a backtracking pattern here (`/\/+$/`) is a
 * regular expression running over library input, and a caller who passed a
 * pathological run of slashes could cost more than the O(n) this loop always
 * costs.
 */
function stripTrailingSlashes(value: string): string {
  let end = value.length;
  while (end > 0 && value.charCodeAt(end - 1) === SLASH_CHAR_CODE) {
    end -= 1;
  }
  return value.slice(0, end);
}

/** `Promise<never>` that never settles — see {@link ConfirmPaymentOptions.redirect}. */
function neverSettles(): Promise<PaymentIntentResult> {
  return new Promise<PaymentIntentResult>(() => {
    // Intentionally empty: the browser is unloading this document. Resolving
    // here is what would let a caller's `.then` run a "payment failed" branch
    // during the navigation, which is the bug this shape exists to prevent.
  });
}

/**
 * vpay's `Stripe` object. Constructed only by {@link loadStripe}.
 *
 * Holds a publishable key and a base URL and nothing else — in particular it
 * never retains a `client_secret`, which is why the redacted representations
 * below can be exhaustive rather than a filter.
 */
export class VpayStripe implements Stripe {
  readonly #publishableKey: string;
  readonly #baseUrl: string;
  readonly #checkoutBaseUrl: string | undefined;
  readonly #fetchImpl: typeof fetch;

  constructor(
    publishableKey: string,
    baseUrl: string,
    checkoutBaseUrl: string | undefined,
    fetchImpl: typeof fetch,
  ) {
    this.#publishableKey = publishableKey;
    this.#baseUrl = baseUrl;
    this.#checkoutBaseUrl = checkoutBaseUrl;
    this.#fetchImpl = fetchImpl;
  }

  async retrievePaymentIntent(
    clientSecret: string,
  ): Promise<PaymentIntentResult> {
    const parsed = parseIntentSecret(clientSecret);
    if ("error" in parsed) {
      return errorResult(parsed.error);
    }
    const query = encodeForm({
      key: this.#publishableKey,
      client_secret: clientSecret,
    });
    return this.#request(
      "GET",
      `${this.#intentUrl(parsed.id)}?${query}`,
      undefined,
    );
  }

  async confirmPayment(
    options: ConfirmPaymentOptions,
  ): Promise<PaymentIntentResult> {
    const parsed = parseIntentSecret(options.clientSecret);
    if ("error" in parsed) {
      return errorResult(parsed.error);
    }

    const params: Record<string, FormValue> = {
      key: this.#publishableKey,
      client_secret: options.clientSecret,
    };
    const paymentMethodData = options.confirmParams?.payment_method_data;
    if (paymentMethodData !== undefined) {
      params["payment_method_data"] = paymentMethodData as FormValue;
    }
    const returnUrl = options.confirmParams?.return_url;
    if (returnUrl !== undefined) {
      params["return_url"] = returnUrl;
    }

    let body: string;
    try {
      body = encodeForm(params);
    } catch (err) {
      // The only values this package adds are strings it was handed; every
      // encodable-value rule can therefore only be broken by
      // `payment_method_data`, which is why `param` is a constant.
      if (err instanceof FormEncodingError) {
        return errorResult(
          stripeError(
            "invalid_request_error",
            "invalid_request",
            err.message,
            "payment_method_data",
          ),
        );
      }
      throw err;
    }

    const result = await this.#request(
      "POST",
      `${this.#intentUrl(parsed.id)}/confirm`,
      body,
    );
    return this.#followRedirect(result, options.redirect);
  }

  async handleNextAction(options: {
    clientSecret: string;
  }): Promise<PaymentIntentResult> {
    const result = await this.retrievePaymentIntent(options.clientSecret);
    // No `redirect` knob: a caller reaching for `handleNextAction` has
    // already decided to perform the action. `redirect: 'if_required'` on
    // `confirmPayment` is how a caller opts out.
    return this.#followRedirect(result, undefined);
  }

  async confirmMobileMoneyPayment(
    clientSecret: string,
    data: MobileMoneyData,
  ): Promise<PaymentIntentResult> {
    // `{ type: 'mtn_momo', mtn_momo: { msisdn } }` — Stripe's shape, and what
    // `/v1`'s `confirm` already accepts. The rail code appears twice on the
    // wire (as `[type]` and as the nested key) because that is the contract;
    // it is not duplicated here as a convenience.
    return this.confirmPayment({
      clientSecret,
      confirmParams: {
        payment_method_data: {
          type: data.type,
          [data.type]: { msisdn: data.msisdn },
        },
      },
    });
  }

  async waitForPaymentIntent(
    clientSecret: string,
    options?: WaitForPaymentIntentOptions,
  ): Promise<PaymentIntentResult> {
    const timeoutMs = options?.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    const intervalMs = options?.intervalMs ?? DEFAULT_INTERVAL_MS;
    if (!Number.isFinite(timeoutMs) || timeoutMs < 0) {
      return errorResult(
        stripeError(
          "invalid_request_error",
          "invalid_request",
          "timeoutMs must be a non-negative, finite number of milliseconds.",
          "timeoutMs",
        ),
      );
    }
    if (!Number.isFinite(intervalMs) || intervalMs <= 0) {
      return errorResult(
        stripeError(
          "invalid_request_error",
          "invalid_request",
          "intervalMs must be a positive, finite number of milliseconds.",
          "intervalMs",
        ),
      );
    }

    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const result = await this.retrievePaymentIntent(clientSecret);
      // An `{ error }` ends the poll rather than being retried until the
      // deadline. A 404 or a malformed `clientSecret` would never come good,
      // and an `api_connection_error` is something the *caller* must decide
      // about — swallowing three minutes of connection failures and then
      // reporting `polling_timeout` would describe the wrong fault.
      if (result.error !== undefined) {
        return result;
      }
      if (hasStoppedMoving(result.paymentIntent)) {
        return result;
      }
      const remaining = deadline - Date.now();
      if (remaining <= 0) {
        return errorResult(
          stripeError(
            "api_error",
            CLIENT_ERROR_CODES.pollingTimeout,
            "Timed out waiting for the payment intent to reach a final state.",
          ),
        );
      }
      await sleep(Math.min(jitter(intervalMs), remaining));
    }
  }

  async retrieveCheckoutSession(
    clientSecret: string,
  ): Promise<CheckoutSessionResult> {
    const parsed = parseSessionSecret(clientSecret);
    if ("error" in parsed) {
      return { error: parsed.error };
    }
    const query = encodeForm({
      key: this.#publishableKey,
      client_secret: clientSecret,
    });
    const answer = await this.#fetchJson(
      "GET",
      `${this.#baseUrl}/v1/browser/checkout/sessions/${encodeURIComponent(parsed.id)}?${query}`,
      undefined,
    );
    if ("error" in answer) {
      return { error: answer.error };
    }
    if (!answer.ok) {
      return {
        error:
          parseErrorEnvelope(answer.parsed) ??
          unexpectedResponseError(answer.status),
      };
    }
    return isCheckoutSession(answer.parsed)
      ? { checkoutSession: answer.parsed }
      : { error: unexpectedResponseError(answer.status) };
  }

  async initEmbeddedCheckout(
    options: InitEmbeddedCheckoutOptions,
  ): Promise<EmbeddedCheckout> {
    const checkoutBaseUrl = this.#checkoutBaseUrl;
    if (checkoutBaseUrl === undefined) {
      throw new TypeError(
        "initEmbeddedCheckout: loadStripe was not given options.checkoutBaseUrl, so there is no page to frame",
      );
    }
    if (typeof options?.fetchClientSecret !== "function") {
      throw new TypeError(
        "initEmbeddedCheckout: options.fetchClientSecret must be a function returning a Promise<string>",
      );
    }
    // Awaited before anything is built, and its rejection is *not* wrapped:
    // this is the merchant's own call to its own server, and a message this
    // package invented on top of it would hide the fault the merchant needs
    // to see.
    const clientSecret = await options.fetchClientSecret();
    const parsed = parseSessionSecret(clientSecret);
    if ("error" in parsed) {
      // The offending value is a live credential; `parseSessionSecret`'s
      // message never quotes it, and neither does this.
      throw new TypeError(
        "initEmbeddedCheckout: fetchClientSecret did not return a vpay checkout-session client secret",
      );
    }
    return createEmbeddedCheckout({
      publishableKey: this.#publishableKey,
      checkoutBaseUrl,
      sessionId: parsed.id,
      clientSecret,
      onComplete: options.onComplete,
    });
  }

  /** A safe, redacted representation. This object holds no secret to redact. */
  [INSPECT_CUSTOM](): string {
    const checkout =
      this.#checkoutBaseUrl === undefined
        ? ""
        : `, checkoutBaseUrl: '${this.#checkoutBaseUrl}'`;
    return `VpayStripe { publishableKey: '${this.#publishableKey}', baseUrl: '${this.#baseUrl}'${checkout} }`;
  }

  /** A safe, redacted representation. This object holds no secret to redact. */
  toJSON(): {
    object: "vpay_stripe";
    publishableKey: string;
    baseUrl: string;
    checkoutBaseUrl?: string;
  } {
    const rendered: {
      object: "vpay_stripe";
      publishableKey: string;
      baseUrl: string;
      checkoutBaseUrl?: string;
    } = {
      object: "vpay_stripe",
      publishableKey: this.#publishableKey,
      baseUrl: this.#baseUrl,
    };
    if (this.#checkoutBaseUrl !== undefined) {
      rendered.checkoutBaseUrl = this.#checkoutBaseUrl;
    }
    return rendered;
  }

  #intentUrl(id: string): string {
    return `${this.#baseUrl}/v1/browser/payment_intents/${encodeURIComponent(id)}`;
  }

  /**
   * One request, one `PaymentIntentResult`. Never throws and never rejects.
   *
   * Neither `url` nor the thrown value reaches a message: the query string
   * holds the `client_secret`.
   */
  async #request(
    method: "GET" | "POST",
    url: string,
    body: string | undefined,
  ): Promise<PaymentIntentResult> {
    const answer = await this.#fetchJson(method, url, body);
    if ("error" in answer) {
      return errorResult(answer.error);
    }
    if (answer.ok) {
      return isPaymentIntent(answer.parsed)
        ? { paymentIntent: answer.parsed }
        : errorResult(unexpectedResponseError(answer.status));
    }
    return errorResult(
      parseErrorEnvelope(answer.parsed) ??
        unexpectedResponseError(answer.status),
    );
  }

  /**
   * The transport, shared by every route this package speaks.
   *
   * Returns the decoded body rather than an object, because the two
   * surfaces decode to different objects (a payment intent, a checkout
   * session) and the *transport* rules — `credentials: 'omit'`, the one
   * header, the body read inside the same `try`, and above all that no
   * thrown value ever reaches a message — must be written once. A second
   * copy of this method for sessions is exactly how one of the two would
   * quietly stop omitting credentials.
   */
  async #fetchJson(
    method: "GET" | "POST",
    url: string,
    body: string | undefined,
  ): Promise<
    { status: number; ok: boolean; parsed: unknown } | { error: StripeError }
  > {
    let status: number;
    let ok: boolean;
    let text: string;
    try {
      // `credentials: 'omit'` and `mode: 'cors'` are set on every request,
      // not left to the fetch implementation's default: this package never
      // wants a cookie or an `Authorization` header attached on the
      // merchant's behalf (the publishable key and client secret in the URL
      // are the only credential this surface uses), and a same-origin
      // default would silently start sending them if vpay's API and the
      // merchant's page ever shared an origin.
      const init: RequestInit = { method, credentials: "omit", mode: "cors" };
      if (body !== undefined) {
        init.body = body;
        // The only header this package sets. Anything beyond
        // `Content-Type: application/x-www-form-urlencoded` (a `Idempotency-Key`,
        // an `Authorization`) would turn a CORS simple request into a
        // preflighted one — see §0 S4 of the design.
        init.headers = {
          "Content-Type": "application/x-www-form-urlencoded",
        };
      }
      const response = await this.#fetchImpl(url, init);
      status = response.status;
      ok = response.ok;
      // Read inside the same `try`: `fetch` resolves once the headers
      // arrive, so a truncated or stalled body rejects here.
      text = await response.text();
    } catch {
      return { error: connectionError() };
    }

    let parsed: unknown;
    try {
      parsed = text.length > 0 ? JSON.parse(text) : undefined;
    } catch {
      parsed = undefined;
    }

    return { status, ok, parsed };
  }

  /**
   * Stripe.js's redirect rule, in one place so `confirmPayment` and
   * `handleNextAction` cannot drift.
   *
   * When the intent carries `next_action.redirect_to_url` and the caller did
   * not ask for `if_required`, the browser navigates and the returned
   * promise **never settles**. With no `window` — Node, SSR, a worker —
   * there is nothing to navigate, and inventing a resolution would tell the
   * caller the payment was handled when the payer never saw the rail's page.
   */
  #followRedirect(
    result: PaymentIntentResult,
    redirect: "always" | "if_required" | undefined,
  ): Promise<PaymentIntentResult> {
    if (result.error !== undefined) {
      return Promise.resolve(result);
    }
    const url = redirectTarget(result.paymentIntent);
    if (url === undefined || redirect === "if_required") {
      return Promise.resolve(result);
    }
    if (!isSafeRedirectUrl(url)) {
      // Refused, not navigated: this string reached us from the rail via
      // the server, and a `javascript:`/`data:`/relative value is refused
      // rather than handed to `location.assign`.
      return Promise.resolve(
        errorResult(
          stripeError(
            "api_error",
            CLIENT_ERROR_CODES.invalidRedirect,
            "The rail requested a redirect to a URL vpay will not navigate to.",
          ),
        ),
      );
    }
    const location = browserWindow()?.location;
    if (location === undefined || typeof location.assign !== "function") {
      return Promise.resolve(
        errorResult(
          stripeError(
            "api_error",
            CLIENT_ERROR_CODES.redirectUnavailable,
            "This payment requires a redirect, but there is no browser window to navigate. Pass `redirect: 'if_required'` and handle `next_action.redirect_to_url` yourself.",
          ),
        ),
      );
    }
    location.assign(url);
    return neverSettles();
  }
}

/**
 * True once the intent will not change again without a new request.
 *
 * `requires_payment_method` is terminal **only** with a
 * `last_payment_error`: it is also the status of an intent nobody has
 * confirmed yet, and treating that as final would make
 * `waitForPaymentIntent` return the instant it was called.
 */
function hasStoppedMoving(intent: PaymentIntent): boolean {
  if (intent.status === "succeeded" || intent.status === "canceled") {
    return true;
  }
  return (
    intent.status === "requires_payment_method" &&
    intent.last_payment_error !== null
  );
}

/** `[0.75, 1.25] × base`, rounded to whole milliseconds. */
function jitter(base: number): number {
  return Math.round(
    base * (1 - JITTER_SPREAD / 2 + Math.random() * JITTER_SPREAD),
  );
}

function sleep(ms: number): Promise<void> {
  return new Promise<void>((resolve) => {
    setTimeout(resolve, ms);
  });
}

/**
 * Builds a {@link Stripe} bound to one vpay deployment and one publishable
 * key.
 *
 * Asynchronous purely for source compatibility with `@stripe/stripe-js`'s
 * `loadStripe`, whose promise covers a `<script>` download. This one has
 * nothing to download — the whole point of the package is that
 * `js.stripe.com` is not in the picture — so it resolves immediately.
 *
 * **This is the one function here that rejects.** A blank publishable key or
 * base URL is an integration mistake visible on the merchant's first page
 * load, not a payer-facing failure, and the `Stripe` object's methods keep
 * their never-rejects contract precisely because the arguments were checked
 * once, here.
 */
// async purely for source compatibility with `@stripe/stripe-js`'s
// `loadStripe`; see the doc comment above.
// eslint-disable-next-line @typescript-eslint/require-await
export async function loadStripe(
  publishableKey: string,
  options: VpayStripeOptions,
): Promise<Stripe> {
  if (
    typeof publishableKey !== "string" ||
    publishableKey.trim().length === 0
  ) {
    throw new TypeError(
      "loadStripe: publishableKey must be a non-empty string",
    );
  }
  if (
    typeof options?.baseUrl !== "string" ||
    options.baseUrl.trim().length === 0
  ) {
    throw new TypeError(
      "loadStripe: options.baseUrl must be a non-empty string",
    );
  }
  const fetchImpl =
    options.fetch ??
    (typeof globalThis.fetch === "function"
      ? // Bound: an unbound native `fetch` called as a method of anything
        // other than the global throws `Illegal invocation` in browsers.
        globalThis.fetch.bind(globalThis)
      : undefined);
  if (fetchImpl === undefined) {
    throw new TypeError(
      "loadStripe: no global fetch is available; pass options.fetch",
    );
  }
  // Trailing slashes stripped once, so `${baseUrl}/v1/browser/...` cannot
  // produce a `//` that some ingresses redirect and others 404.
  const baseUrl = stripTrailingSlashes(options.baseUrl.trim());
  // Checked here rather than at `initEmbeddedCheckout`, for the same reason
  // the two above are: a misconfigured checkout origin is an integration
  // mistake, and `event.origin` is compared against this value — a bad one
  // does not fail loudly at message time, it silently accepts nothing.
  let checkoutBaseUrl: string | undefined;
  if (options.checkoutBaseUrl !== undefined) {
    if (
      typeof options.checkoutBaseUrl !== "string" ||
      options.checkoutBaseUrl.trim().length === 0
    ) {
      throw new TypeError(
        "loadStripe: options.checkoutBaseUrl must be a non-empty string",
      );
    }
    checkoutBaseUrl = stripTrailingSlashes(options.checkoutBaseUrl.trim());
    if (!isSafeRedirectUrl(checkoutBaseUrl)) {
      throw new TypeError(
        "loadStripe: options.checkoutBaseUrl must be an absolute http(s) URL",
      );
    }
  }
  return new VpayStripe(publishableKey, baseUrl, checkoutBaseUrl, fetchImpl);
}
