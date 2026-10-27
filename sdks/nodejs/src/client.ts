import { inspect } from "node:util";
import { HttpClient } from "./http.js";
import { AccountHoldersResource } from "./resources/account-holders.js";
import { BalanceResource } from "./resources/balance.js";
import { CheckoutResource } from "./resources/checkout-sessions.js";
import { CustomersResource } from "./resources/customers.js";
import { EventsResource } from "./resources/events.js";
import {
  InvoiceItemsResource,
  InvoicesResource,
} from "./resources/invoices.js";
import { PaymentIntentsResource } from "./resources/payment-intents.js";
import { RefundsResource } from "./resources/refunds.js";
import { resolveMerchantAuth, type MerchantAuthOptions } from "./sealer.js";

/**
 * Constructor options for {@link VpayClient} — see the package README's
 * configuration table.
 *
 * **This is {@link MerchantAuthOptions}**, which is also what
 * `createStripeHttpClient` takes. The two entry points seal the same calls
 * under the same keys with the same defaults ({@link resolveMerchantAuth} is the
 * single implementation), so they must offer the same knobs — and an interface
 * that merely *listed* the same properties would drift the first time one of
 * them gained another. Extending is what makes "the same options" checked
 * rather than intended.
 *
 * Every optional property is declared `?: T | undefined`, not `?: T`. Under
 * `exactOptionalPropertyTypes` (which this repo sets, and which consumers are
 * likely to) the shorter form rejects `{ audience: maybeAudience }` where
 * `maybeAudience` is `string | undefined` — forcing every consumer into
 * conditional spreads for options that are, semantically, simply optional.
 * `src/types.test.ts` pins this at the type level.
 */
export interface VpayClientOptions extends MerchantAuthOptions {
  /** Default 30000, applied to every resource call. */
  timeoutMs?: number | undefined;
}

const INSPECT_CUSTOM = Symbol.for("nodejs.util.inspect.custom");

/**
 * The vpay merchant SDK client. Seals every `/v1` call as a COSE_Sign1 message
 * under the merchant's Ed25519 key, opens the answer under the pinned server
 * keys, and exposes the resource namespaces below.
 *
 * The private key never appears in `util.inspect(client)` or
 * `JSON.stringify(client)` output — this class never holds it. It is read once
 * by `resolveMerchantAuth` and kept inside the sealer (a seed, in the wasm
 * module's memory) or behind a `crypto.KeyObject`, whose own inspection
 * reveals no key material; the two redacted representations below are built by
 * hand from the two fields that are safe to print.
 */
export class VpayClient {
  readonly #clientId: string;
  readonly #baseUrl: string;

  readonly paymentIntents: PaymentIntentsResource;
  readonly checkout: CheckoutResource;
  /** `/v1/customers` — the merchant-owned record of a payer (S4a). */
  readonly customers: CustomersResource;
  /** `/v1/invoices` — a merchant's bill to one customer (S4b). */
  readonly invoices: InvoicesResource;
  /** `/v1/invoice_items` — the lines of a draft invoice (S4b). */
  readonly invoiceItems: InvoiceItemsResource;
  readonly refunds: RefundsResource;
  readonly events: EventsResource;
  readonly balance: BalanceResource;
  /** `/v1/account_holders` — issue #47. */
  readonly accountHolders: AccountHoldersResource;

  constructor(options: VpayClientOptions) {
    // Validation, defaulting and the sealer itself live in
    // `resolveMerchantAuth`, shared with `@vaam-apps/vpay-sdk/stripe`'s HTTP
    // client: both entry points must seal the same bytes under the same keys
    // with the same defaults, and one copy is how that stays true.
    const auth = resolveMerchantAuth(options);
    this.#baseUrl = auth.baseUrl;
    this.#clientId = auth.clientId;

    const httpClient = new HttpClient(auth);

    this.paymentIntents = new PaymentIntentsResource(httpClient);
    this.checkout = new CheckoutResource(httpClient);
    this.customers = new CustomersResource(httpClient);
    this.invoices = new InvoicesResource(httpClient);
    this.invoiceItems = new InvoiceItemsResource(httpClient);
    this.refunds = new RefundsResource(httpClient);
    this.events = new EventsResource(httpClient);
    this.balance = new BalanceResource(httpClient);
    this.accountHolders = new AccountHoldersResource(httpClient);
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
