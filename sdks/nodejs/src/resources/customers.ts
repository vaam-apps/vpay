/**
 * `/v1/customers` — the five merchant operations on the Customer object
 * (S4a), and the one thing this file does that no other resource does: it
 * distinguishes *clear this field* from *leave it alone* — for four fields
 * now, `address` included. `address` is also the one field on this API that
 * is WIDER than Stripe's: it carries the GPS point beside the six formal
 * components (the maintainer, 2026-09-11), and `Address` in `types.ts` says
 * why — including why the coordinate is a whole count of microdegrees and
 * never a decimal degree.
 *
 * # The three states, and why `update` looks the way it does
 *
 * `undefined` (or an absent key) means "leave it alone"; a string means "set
 * it"; **`null` means "clear it"**, and this file sends that as `name=`,
 * which is what the wire spells "clear". Collapsing `null` and `undefined` —
 * the obvious simplification, and what a `params.name !== undefined` guard
 * alone would do — makes a payer's email unclearable through the API that
 * documents how to clear it. `sdks/rust`'s `UpdateCustomerParams` uses
 * `Option<Option<String>>` for exactly this and the server carries the same
 * three states in `vpay_db::CustomerPatch`.
 *
 * # No redacting `util.inspect`, unlike `checkout-sessions.ts`
 *
 * That file redacts because a `CheckoutSession` carries a *credential*:
 * printing one is a compromise, and the merchant who logged it can do nothing
 * about it afterwards. A `Customer` carries a payer's own name, email and
 * phone number — data the merchant collected, already holds and is
 * responsible for. Redacting it here would hide their own data from them
 * while doing nothing about the copy in their database.
 *
 * The place that judgement is made *for* them is the server: vpay's
 * `CustomerRow` redacts all three in its `Debug`, because vpay's logs are not
 * the merchant's. `docs/flows/customers.md` records the asymmetry so it reads
 * as a decision rather than an oversight, and `sdks/rust`'s `Customer`
 * derives `Debug` for the same reason.
 */
import type { HttpClient } from "../http.js";
import type { FormValue } from "../form.js";
import type {
  AddressParams,
  CreateCustomerParams,
  Customer,
  DeletedCustomer,
  List,
  ListCustomersParams,
  RequestOptions,
  UpdateCustomerParams,
} from "../types.js";

/**
 * One patch field, in the wire's own three-state encoding.
 *
 * Returns `undefined` for "the request did not mention this", which the
 * caller drops from the body entirely, and the empty string for "clear it".
 */
function patch(value: string | null | undefined): FormValue | undefined {
  if (value === undefined) {
    return undefined;
  }
  return value === null ? "" : value;
}

/**
 * The eight address components as a nested body value, with every unset one
 * omitted entirely.
 *
 * Omitted and not sent empty, because on this API those are different
 * requests everywhere else — and because the server replaces the address
 * whole, so an omitted component is cleared either way and `address[city]=`
 * would only add a pair saying the same thing.
 *
 * The two coordinate keys go through the same loop as the six formal ones and
 * are typed `number` rather than `string`: the form encoder renders a number
 * as a decimal integer with no separators and refuses a non-finite one
 * (`form.ts`), so there is no path from this function to a body carrying a
 * float — which is the whole of why the wire field is named for its unit.
 */
function addressBody(address: AddressParams): Record<string, FormValue> {
  const body: Record<string, FormValue> = {};
  for (const key of [
    "line1",
    "line2",
    "city",
    "state",
    "postal_code",
    "country",
    "latitude_microdeg",
    "longitude_microdeg",
  ] as const) {
    const value = address[key];
    if (value !== undefined) {
      body[key] = value;
    }
  }
  return body;
}

/** `client.customers` — see {@link CustomersResource}. */
export class CustomersResource {
  readonly #http: HttpClient;

  constructor(http: HttpClient) {
    this.#http = http;
  }

  /**
   * `POST /v1/customers`.
   *
   * Field order is the wire order the Rust SDK pins byte for byte
   * (`sdks/rust/tests/resources.rs`); an unset field is omitted from the body
   * entirely, because `name=` and no `name` are different requests and only
   * the second means "not applicable".
   */
  async create(
    params: CreateCustomerParams,
    options?: RequestOptions,
  ): Promise<Customer> {
    const body: Record<string, FormValue> = {};
    if (params.name !== undefined) {
      body["name"] = params.name;
    }
    if (params.email !== undefined) {
      body["email"] = params.email;
    }
    if (params.phone !== undefined) {
      body["phone"] = params.phone;
    }
    if (params.address !== undefined) {
      body["address"] = addressBody(params.address);
    }
    if (params.metadata !== undefined) {
      body["metadata"] = params.metadata;
    }
    return this.#http.request<Customer>("POST", "/customers", body, options);
  }

  /** `GET /v1/customers/{id}`. */
  async retrieve(id: string): Promise<Customer> {
    return this.#http.request<Customer>(
      "GET",
      `/customers/${encodeURIComponent(id)}`,
    );
  }

  /**
   * `POST /v1/customers/{id}` — the update.
   *
   * A `POST` and not a `PUT`/`PATCH`, because that is what the API is:
   * Stripe has neither verb, and a merchant's existing client sends a `POST`.
   *
   * A patch that would clear the customer's **last** identifier is refused
   * `400` naming `name` — a customer must always have one of `name`, `email`
   * and `phone`.
   */
  async update(
    id: string,
    params: UpdateCustomerParams,
    options?: RequestOptions,
  ): Promise<Customer> {
    const body: Record<string, FormValue> = {};
    const name = patch(params.name);
    if (name !== undefined) {
      body["name"] = name;
    }
    const email = patch(params.email);
    if (email !== undefined) {
      body["email"] = email;
    }
    const phone = patch(params.phone);
    if (phone !== undefined) {
      body["phone"] = phone;
    }
    if (params.address !== undefined) {
      // `address=` — the wire's own "remove this", the same spelling `name=`
      // uses. NOT six empty components: the server reads a *scalar* `address`
      // as the clear, and one spelling means a merchant reading the body sees
      // what they asked for.
      body["address"] =
        params.address === null ? "" : addressBody(params.address);
    }
    if (params.metadata !== undefined) {
      body["metadata"] = params.metadata;
    }
    return this.#http.request<Customer>(
      "POST",
      `/customers/${encodeURIComponent(id)}`,
      body,
      options,
    );
  }

  /** `GET /v1/customers`. */
  async list(params?: ListCustomersParams): Promise<List<Customer>> {
    return this.#http.request<List<Customer>>("GET", "/customers", params);
  }

  /**
   * `DELETE /v1/customers/{id}` — an **erasure**, in one of two shapes.
   *
   * `del` and not `delete`: `delete` is a reserved word in older JavaScript
   * object literals, which is why Stripe's own SDKs spell it this way, and
   * `sdks/rust`'s `CustomersResource::del` matches so a merchant who read one
   * SDK's docs can use the other.
   *
   * A customer with **no payment history** is removed outright: a subsequent
   * {@link retrieve} answers the same 404 as an id that never existed.
   *
   * A customer any PaymentIntent, Checkout Session or Invoice references
   * cannot be removed — vpay keeps a payment attached to the payer it was
   * taken from — so it is **anonymised** instead. The row and the payment
   * record stay; every identifier on the customer becomes `[redacted]`, and
   * so does every copy vpay kept elsewhere. A later {@link retrieve} answers
   * `200` with {@link Customer.deleted} `true`, so your own stored `cus_…`
   * keeps resolving. `metadata` is untouched: that is your data, not the
   * payer's.
   *
   * Either way this answers `{deleted: true}`, and a second call is a no-op
   * rather than a second erasure. Until 2026-09-10 the second case answered
   * `409` instead, with advice — clear `name`, `email` and `phone` — that the
   * server's own one-of rule refuses.
   *
   * Carries an `Idempotency-Key` like every other write on this API, which is
   * what makes a retried delete answer the original `{deleted: true}` rather
   * than a `404`.
   */
  async del(id: string, options?: RequestOptions): Promise<DeletedCustomer> {
    return this.#http.request<DeletedCustomer>(
      "DELETE",
      `/customers/${encodeURIComponent(id)}`,
      undefined,
      options,
    );
  }
}
