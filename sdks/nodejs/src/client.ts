import type { KeyObject } from "node:crypto";
import { inspect } from "node:util";
import { TokenManager, validateAssertionLifetimeSeconds } from "./auth.js";
import { VpayConfigError } from "./errors.js";
import { HttpClient } from "./http.js";
import { BalanceResource } from "./resources/balance.js";
import { EventsResource } from "./resources/events.js";
import { PaymentIntentsResource } from "./resources/payment-intents.js";
import { RefundsResource } from "./resources/refunds.js";
import { SDK_VERSION } from "./version.js";

const DEFAULT_AUDIENCE = "vpay:v1";
const DEFAULT_ASSERTION_LIFETIME_SECONDS = 60;
const DEFAULT_TIMEOUT_MS = 30_000;

/**
 * Constructor options for {@link VpayClient} — see the package README's
 * configuration table.
 *
 * Every optional property is declared `?: T | undefined`, not `?: T`. Under
 * `exactOptionalPropertyTypes` (which this repo sets, and which consumers are
 * likely to) the shorter form rejects `{ kid: maybeKid }` where `maybeKid` is
 * `string | undefined` — forcing every consumer into conditional spreads for
 * options that are, semantically, simply optional. `src/types.test.ts` pins
 * this at the type level.
 */
export interface VpayClientOptions {
  /** vpay's base URL, e.g. `https://api.vpay.example`. */
  baseUrl: string;
  /** This merchant's registered OAuth2 `client_id`. */
  clientId: string;
  /** The merchant's RSA private key — PEM text or a `crypto.KeyObject`. Never logged. */
  privateKey: string | KeyObject;
  /** Required only if the merchant registered more than one JWK. */
  kid?: string | undefined;
  /** Default `${baseUrl}/v1/oauth` — the OP's issuer, per docs/flows/merchant-auth.md. */
  issuer?: string | undefined;
  /** Default `${issuer}/token`. Also the client assertion's `aud`. */
  tokenEndpoint?: string | undefined;
  /** Default `vpay:v1` — see docs/flows/merchant-auth.md's note on why this is load-bearing. */
  audience?: string | undefined;
  /** Omitted from the token request unless set. */
  scope?: string | undefined;
  /** Default 60. Must be an integer in `1..=300`; anything else throws `VpayConfigError` at construction. */
  assertionLifetimeSeconds?: number | undefined;
  /** Default 30000. Applies to both the token exchange and every resource call. */
  timeoutMs?: number | undefined;
  /** Injection point for tests and proxies. Defaults to the global `fetch`. */
  fetch?: typeof fetch | undefined;
}

function requireNonEmptyString(
  value: string | undefined,
  field: string,
): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new VpayConfigError(`${field} is required`);
  }
  return value;
}

function stripTrailingSlash(url: string): string {
  return url.endsWith("/") ? url.slice(0, -1) : url;
}

const INSPECT_CUSTOM = Symbol.for("nodejs.util.inspect.custom");

/**
 * The vpay merchant SDK client. Mints `private_key_jwt` assertions, exchanges
 * and caches `/v1` access tokens, and exposes the resource namespaces below.
 *
 * The private key never appears in `util.inspect(client)` or
 * `JSON.stringify(client)` output — it is held in a private class field,
 * which is excluded from both by construction.
 */
export class VpayClient {
  readonly #clientId: string;
  readonly #baseUrl: string;
  readonly #privateKey: string | KeyObject;
  readonly #tokenManager: TokenManager;

  readonly paymentIntents: PaymentIntentsResource;
  readonly refunds: RefundsResource;
  readonly events: EventsResource;
  readonly balance: BalanceResource;

  constructor(options: VpayClientOptions) {
    this.#baseUrl = stripTrailingSlash(
      requireNonEmptyString(options.baseUrl, "baseUrl"),
    );
    this.#clientId = requireNonEmptyString(options.clientId, "clientId");
    if (options.privateKey === undefined || options.privateKey === null) {
      throw new VpayConfigError("privateKey is required");
    }
    this.#privateKey = options.privateKey;

    const assertionLifetimeSeconds =
      options.assertionLifetimeSeconds ?? DEFAULT_ASSERTION_LIFETIME_SECONDS;
    validateAssertionLifetimeSeconds(assertionLifetimeSeconds);

    const issuer = options.issuer ?? `${this.#baseUrl}/v1/oauth`;
    const tokenEndpoint = options.tokenEndpoint ?? `${issuer}/token`;
    const audience = options.audience ?? DEFAULT_AUDIENCE;
    const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    const fetchImpl = options.fetch ?? fetch;
    const userAgent = `vpay-sdk-node/${SDK_VERSION}`;

    this.#tokenManager = new TokenManager({
      clientId: this.#clientId,
      privateKey: this.#privateKey,
      kid: options.kid,
      tokenEndpoint,
      audience,
      scope: options.scope,
      assertionLifetimeSeconds,
      timeoutMs,
      fetchImpl,
      userAgent,
    });

    const httpClient = new HttpClient({
      baseUrl: `${this.#baseUrl}/v1`,
      tokenManager: this.#tokenManager,
      fetchImpl,
      timeoutMs,
      userAgent,
    });

    this.paymentIntents = new PaymentIntentsResource(httpClient);
    this.refunds = new RefundsResource(httpClient);
    this.events = new EventsResource(httpClient);
    this.balance = new BalanceResource(httpClient);
  }

  /** A safe, redacted representation — never the private key. */
  [INSPECT_CUSTOM](): string {
    return `VpayClient ${inspect({ clientId: this.#clientId, baseUrl: this.#baseUrl }, { colors: false })}`;
  }

  /** A safe, redacted representation — never the private key. */
  toJSON(): { object: "vpay_client"; clientId: string; baseUrl: string } {
    return {
      object: "vpay_client",
      clientId: this.#clientId,
      baseUrl: this.#baseUrl,
    };
  }
}
