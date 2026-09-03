import { inspect } from "node:util";
import {
  resolveMerchantAuth,
  type MerchantAuthOptions,
  type TokenManager,
} from "./auth.js";
import { HttpClient } from "./http.js";
import { BalanceResource } from "./resources/balance.js";
import { EventsResource } from "./resources/events.js";
import { PaymentIntentsResource } from "./resources/payment-intents.js";
import { RefundsResource } from "./resources/refunds.js";

/**
 * Constructor options for {@link VpayClient} — see the package README's
 * configuration table.
 *
 * **This is {@link MerchantAuthOptions}**, which is also what
 * `createStripeAuthenticator` takes. The two entry points authenticate the
 * same way against the same endpoint with the same defaults
 * ({@link resolveMerchantAuth} is the single implementation), so they must
 * offer the same knobs — and an interface that merely *listed* the same
 * eleven properties would drift the first time one of them gained a twelfth.
 * Extending is what makes "the same options" checked rather than intended.
 *
 * Every optional property is declared `?: T | undefined`, not `?: T`. Under
 * `exactOptionalPropertyTypes` (which this repo sets, and which consumers are
 * likely to) the shorter form rejects `{ kid: maybeKid }` where `maybeKid` is
 * `string | undefined` — forcing every consumer into conditional spreads for
 * options that are, semantically, simply optional. `src/types.test.ts` pins
 * this at the type level.
 */
export interface VpayClientOptions extends MerchantAuthOptions {
  /**
   * Default 30000. Applies to **both** the token exchange and every resource
   * call — the one place this client's reading of an inherited option is
   * narrower than the base's, because `VpayClient` owns its resource
   * transport and `createStripeAuthenticator` does not.
   */
  timeoutMs?: number | undefined;
}

const INSPECT_CUSTOM = Symbol.for("nodejs.util.inspect.custom");

/**
 * The vpay merchant SDK client. Mints `private_key_jwt` assertions, exchanges
 * and caches `/v1` access tokens, and exposes the resource namespaces below.
 *
 * The private key never appears in `util.inspect(client)` or
 * `JSON.stringify(client)` output — this class never holds it. It is parsed
 * once by `resolveMerchantAuth` and kept by the {@link TokenManager} as a
 * `crypto.KeyObject`, whose own inspection reveals no key material; the two
 * redacted representations below are built by hand from the two fields that
 * are safe to print.
 */
export class VpayClient {
  readonly #clientId: string;
  readonly #baseUrl: string;
  readonly #tokenManager: TokenManager;

  readonly paymentIntents: PaymentIntentsResource;
  readonly refunds: RefundsResource;
  readonly events: EventsResource;
  readonly balance: BalanceResource;

  constructor(options: VpayClientOptions) {
    // Validation, defaulting and the `TokenManager` itself live in
    // `resolveMerchantAuth`, shared with `@vpay/sdk/stripe`'s authenticator:
    // both entry points must mint the same assertion against the same
    // endpoint with the same defaults, and one copy is how that stays true.
    const auth = resolveMerchantAuth(options);
    this.#baseUrl = auth.baseUrl;
    this.#clientId = auth.clientId;
    this.#tokenManager = auth.tokenManager;

    const httpClient = new HttpClient({
      baseUrl: `${this.#baseUrl}/v1`,
      tokenManager: this.#tokenManager,
      fetchImpl: auth.fetchImpl,
      timeoutMs: auth.timeoutMs,
      userAgent: auth.userAgent,
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
