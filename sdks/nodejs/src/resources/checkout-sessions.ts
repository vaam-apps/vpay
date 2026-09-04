/**
 * `/v1/checkout/sessions` — the four merchant operations Step 9's wire
 * contract defines, and the one thing this file does that no other resource
 * does: it redacts the object's `client_secret` from `util.inspect`.
 *
 * ADR-0015's matrix records, as a dated gap, that `PaymentIntent` is a plain
 * interface here and so `console.log(intent)` prints a live payer
 * credential, while the Rust SDK's hand-written `Debug` redacts it. That gap
 * is not repeated for a **new** object: every `CheckoutSession` this SDK
 * hands back carries a custom inspect representation that prints
 * `[N chars redacted]` in place of the secret, which is exactly what
 * `vpay_sdk::CheckoutSession`'s `impl Debug` prints.
 *
 * `JSON.stringify` is deliberately **not** redacted, and that asymmetry is
 * the same one Rust has (`Debug` redacts, `Serialize` does not): an embedded
 * integration must serialise the secret to get it to the browser at all, so
 * a redacting `toJSON` would break the one thing the field exists for. The
 * hazard being closed here is the accidental one — a session that reaches a
 * log line because something logged the whole object.
 */
import type { HttpClient } from "../http.js";
import type { FormValue } from "../form.js";
import type {
  CheckoutSession,
  CreateCheckoutSessionParams,
  List,
  ListCheckoutSessionsParams,
  RequestOptions,
} from "../types.js";

const INSPECT_CUSTOM = Symbol.for("nodejs.util.inspect.custom");

/**
 * The `url` of a hosted session with its fragment redacted.
 *
 * D6 puts the session's `client_secret` in the hosted page's URL fragment
 * (`https://checkout.example/c/cs_1#cs_1_secret_…`), so redacting
 * `client_secret` alone and printing `url` verbatim would leak exactly the
 * value the redaction exists to hide — measured, not assumed: the first
 * version of this file did that, and the test that now pins it failed.
 */
function redactUrlFragment(url: string): string {
  const hash = url.indexOf("#");
  if (hash === -1) {
    return url;
  }
  const fragment = url.slice(hash + 1);
  return `${url.slice(0, hash + 1)}[${fragment.length} chars redacted]`;
}

/**
 * Attaches a redacting `util.inspect` representation to one decoded
 * session, in place.
 *
 * Non-enumerable, so the property is invisible to `Object.keys`,
 * `JSON.stringify`, spread and deep-equality — the object a merchant
 * receives is still, in every observable way, the decoded response body.
 * The representation itself is built from a **spread copy**, which does not
 * carry the symbol, so `inspect` inside it cannot recurse.
 */
function withRedactedInspect<T extends { client_secret?: string }>(
  session: T,
): T {
  if (typeof session !== "object" || session === null) {
    return session;
  }
  Object.defineProperty(session, INSPECT_CUSTOM, {
    value: function redactedInspect(
      this: T,
      depth: number,
      options: unknown,
      inspect: (value: unknown, options: unknown) => string,
    ): string {
      const secret = this.client_secret;
      const copy: Record<string, unknown> = { ...this };
      if (typeof secret === "string") {
        copy["client_secret"] = `[${secret.length} chars redacted]`;
      }
      const url: unknown = copy["url"];
      if (typeof url === "string") {
        copy["url"] = redactUrlFragment(url);
      }
      // `inspect` is handed in by `node:util` itself, so this works
      // whatever options the caller passed (colours, depth, breakLength)
      // without this file importing `node:util` at all.
      return `CheckoutSession ${inspect(copy, options)}`;
    },
    enumerable: false,
    writable: false,
    configurable: true,
  });
  return session;
}

/** `client.checkout.sessions` — see {@link CheckoutResource}. */
export class CheckoutSessionsResource {
  readonly #http: HttpClient;

  constructor(http: HttpClient) {
    this.#http = http;
  }

  /**
   * `POST /v1/checkout/sessions`. Answers the session **with**
   * `client_secret`, and with `url` when `ui_mode` is `hosted`.
   */
  async create(
    params: CreateCheckoutSessionParams,
    options?: RequestOptions,
  ): Promise<CheckoutSession> {
    // Field order is the wire order the Rust SDK pins byte for byte
    // (`sdks/rust/tests/resources.rs`); an unset field is omitted from the
    // body entirely, because `success_url=` and no `success_url` are
    // different requests and only the second means "not applicable".
    const body: Record<string, FormValue> = {
      payment_intent: params.payment_intent,
    };
    if (params.ui_mode !== undefined) {
      body["ui_mode"] = params.ui_mode;
    }
    if (params.success_url !== undefined) {
      body["success_url"] = params.success_url;
    }
    if (params.cancel_url !== undefined) {
      body["cancel_url"] = params.cancel_url;
    }
    if (params.return_url !== undefined) {
      body["return_url"] = params.return_url;
    }
    return withRedactedInspect(
      await this.#http.request<CheckoutSession>(
        "POST",
        "/checkout/sessions",
        body,
        options,
      ),
    );
  }

  /** `GET /v1/checkout/sessions/{id}`. Answers the session with `client_secret`. */
  async retrieve(id: string): Promise<CheckoutSession> {
    return withRedactedInspect(
      await this.#http.request<CheckoutSession>(
        "GET",
        `/checkout/sessions/${encodeURIComponent(id)}`,
      ),
    );
  }

  /** `GET /v1/checkout/sessions`. List items never carry `client_secret`. */
  async list(
    params?: ListCheckoutSessionsParams,
  ): Promise<List<CheckoutSession>> {
    const page = await this.#http.request<List<CheckoutSession>>(
      "GET",
      "/checkout/sessions",
      params,
    );
    if (Array.isArray(page?.data)) {
      for (const session of page.data) {
        withRedactedInspect(session);
      }
    }
    return page;
  }

  /**
   * `POST /v1/checkout/sessions/{id}/expire`. `open` → `expired`; a session
   * whose intent already has a live charge is refused `409`.
   */
  async expire(id: string, options?: RequestOptions): Promise<CheckoutSession> {
    return withRedactedInspect(
      await this.#http.request<CheckoutSession>(
        "POST",
        `/checkout/sessions/${encodeURIComponent(id)}/expire`,
        {},
        options,
      ),
    );
  }
}

/**
 * `client.checkout` — a namespace, not a resource with operations of its
 * own.
 *
 * It exists so the call reads `client.checkout.sessions.create(…)`, which is
 * the path the wire contract uses (`/v1/checkout/sessions`) and the shape
 * `vpay_sdk`'s `client.checkout().sessions()` mirrors. A flat
 * `client.checkoutSessions` would have been shorter and would have made the
 * two SDKs read differently for the same route.
 */
export class CheckoutResource {
  readonly sessions: CheckoutSessionsResource;

  constructor(http: HttpClient) {
    this.sessions = new CheckoutSessionsResource(http);
  }
}
