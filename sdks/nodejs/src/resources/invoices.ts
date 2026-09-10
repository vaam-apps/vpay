/**
 * `/v1/invoices` and `/v1/invoice_items` — the thirteen merchant operations
 * on the Invoice object and its lines (S4b).
 *
 * # The one thing to know before calling any of them
 *
 * **The state machine is the server's `WHERE` clause, not a check this SDK
 * makes.** A transition that does not apply comes back a `409` naming the
 * invoice's current status, and this SDK does not try to predict which —
 * `draft → open → paid | void | uncollectible`, and every method below says
 * which statuses it accepts. Reading the invoice first to decide whether to
 * call is a race; call, and read the refusal.
 *
 * # Two resources, one file
 *
 * A line has no life of its own: it belongs to exactly one invoice, its
 * currency and tenant are copied off that invoice, and every write to it is
 * refused once the invoice is issued. Splitting the two would put that guard
 * in a file that does not mention the thing it guards. `sdks/rust` keeps them
 * in one module for the same reason.
 */
import type { HttpClient } from "../http.js";
import type { FormValue } from "../form.js";
import { assertIntegerAmount } from "../validate.js";
import type {
  CreateInvoiceItemParams,
  CreateInvoiceParams,
  DeletedInvoice,
  DeletedInvoiceItem,
  Invoice,
  InvoiceLine,
  List,
  ListInvoicesParams,
  PayInvoiceParams,
  RequestOptions,
  UpdateInvoiceItemParams,
  UpdateInvoiceParams,
} from "../types.js";

/**
 * One patch field, in the wire's own three-state encoding.
 *
 * Returns `undefined` for "the request did not mention this", which the
 * caller drops from the body entirely, and the empty string for "clear it".
 * The twin of `customers.ts`'s `patch`, kept per file rather than shared
 * because the two differ in the value type they carry and merging them would
 * cost a generic that says less than the two lines it replaced.
 */
function patch<T extends string | number>(
  value: T | null | undefined,
): FormValue | undefined {
  if (value === undefined) {
    return undefined;
  }
  return value === null ? "" : value;
}

/** `client.invoices` — see the module doc. */
export class InvoicesResource {
  readonly #http: HttpClient;

  constructor(http: HttpClient) {
    this.#http = http;
  }

  /**
   * `POST /v1/invoices` — creates a **draft**, with no number and zero
   * amounts.
   *
   * Field order is the wire order `sdks/rust/tests/resources.rs` pins byte
   * for byte; an unset field is omitted from the body entirely, because
   * `description=` and no `description` are different requests and only the
   * second means "not applicable".
   *
   * A `400` naming `customer` when it is absent, unknown, or another
   * merchant's — one sentence for all three, so the parameter is not an
   * oracle.
   */
  async create(
    params: CreateInvoiceParams,
    options?: RequestOptions,
  ): Promise<Invoice> {
    const body: Record<string, FormValue> = { customer: params.customer };
    if (params.currency !== undefined) {
      body["currency"] = params.currency.toLowerCase();
    }
    if (params.description !== undefined) {
      body["description"] = params.description;
    }
    if (params.due_date !== undefined) {
      body["due_date"] = params.due_date;
    }
    if (params.metadata !== undefined) {
      body["metadata"] = params.metadata;
    }
    return this.#http.request<Invoice>("POST", "/invoices", body, options);
  }

  /**
   * `GET /v1/invoices/{id}` — the invoice **with its lines**.
   *
   * This is the read a webhook handler falls back to: an `invoice.*` event
   * body carries an empty `lines.data`, and this response always carries the
   * lines.
   *
   * Another merchant's `in_…` is the same `404`, byte for byte, as one that
   * never existed.
   */
  async retrieve(id: string): Promise<Invoice> {
    return this.#http.request<Invoice>(
      "GET",
      `/invoices/${encodeURIComponent(id)}`,
    );
  }

  /**
   * `POST /v1/invoices/{id}` — patches a **draft**.
   *
   * A `POST` and not a `PATCH`, for `customers.update`'s reason, even though
   * the server mounts both verbs on the one handler: `POST` is what a
   * merchant's existing Stripe client sends, so it is the one that has to
   * work.
   *
   * A `409` naming the status once the invoice is no longer a draft — an
   * issued document's terms are what was sent to the payer. Void it and issue
   * a new one.
   */
  async update(
    id: string,
    params: UpdateInvoiceParams,
    options?: RequestOptions,
  ): Promise<Invoice> {
    const body: Record<string, FormValue> = {};
    const description = patch(params.description);
    if (description !== undefined) {
      body["description"] = description;
    }
    const dueDate = patch(params.due_date);
    if (dueDate !== undefined) {
      body["due_date"] = dueDate;
    }
    if (params.metadata !== undefined) {
      body["metadata"] = params.metadata;
    }
    return this.#http.request<Invoice>(
      "POST",
      `/invoices/${encodeURIComponent(id)}`,
      body,
      options,
    );
  }

  /** `GET /v1/invoices` — newest first. */
  async list(params?: ListInvoicesParams): Promise<List<Invoice>> {
    return this.#http.request<List<Invoice>>("GET", "/invoices", params);
  }

  /**
   * `DELETE /v1/invoices/{id}` — **draft only**, and its lines go with it.
   *
   * An issued invoice is {@link InvoicesResource.void}ed, never deleted: a
   * voided draft would be a non-draft row with no number, which the database
   * refuses outright, and a document that vanished is a hole an accountant
   * reads as a destroyed one.
   *
   * `del` and not `delete`, for `customers.del`'s reason. Carries an
   * `Idempotency-Key` like every other write, which is what makes a retried
   * delete answer the original `{deleted: true}` rather than a `404`.
   */
  async del(id: string, options?: RequestOptions): Promise<DeletedInvoice> {
    return this.#http.request<DeletedInvoice>(
      "DELETE",
      `/invoices/${encodeURIComponent(id)}`,
      undefined,
      options,
    );
  }

  /**
   * `POST /v1/invoices/{id}/finalize` — issues the document. **Draft only.**
   *
   * The money transition: the invoice takes the next number out of this
   * merchant's own sequence under a row lock, its lines freeze, its amounts
   * are computed once, and it moves to `"open"` — all in one transaction,
   * with `invoice.finalized` inside it. Numbers are consecutive and have no
   * holes, unlike Stripe's.
   *
   * A `400` naming `invoice` when it has no lines, or when its total is past
   * `Number.MAX_SAFE_INTEGER` minor units; a `409` when it is not a draft.
   */
  async finalize(id: string, options?: RequestOptions): Promise<Invoice> {
    return this.#transition(id, "finalize", options);
  }

  /**
   * `POST /v1/invoices/{id}/void` — cancels an issued document. **Open
   * only.**
   *
   * The invoice **keeps its number**. Terminal.
   *
   * A `409` when it is not open, and a `409` while a payment intent is
   * attached and not yet canceled — cancel that intent first.
   */
  async void(id: string, options?: RequestOptions): Promise<Invoice> {
    return this.#transition(id, "void", options);
  }

  /**
   * `POST /v1/invoices/{id}/mark_uncollectible` — writes an issued document
   * off. **Open only.**
   *
   * Still owed, never expected. Terminal, and it **emits no event**: vpay
   * does not write `invoice.marked_uncollectible`, so nothing arrives on a
   * webhook and a merchant reconciling write-offs reads them with
   * `list({ status: "uncollectible" })`.
   *
   * # `markUncollectible` here, `mark_uncollectible` in `sdks/rust`
   *
   * The only capability the two SDKs spell differently, and the difference is
   * each language's own casing rather than a divergence: `sdks/rust` may not
   * spell `markUncollectible` (`non_snake_case` is a rustc lint, not a
   * preference) and this package should not spell `mark_uncollectible` —
   * Stripe's own Node SDK, which merchants arrive from, says
   * `markUncollectible`, and the other spelling would be the one snake_case
   * method in a camelCase package.
   *
   * [ADR-0015](../../../docs/adr/0015-sdk-parity.md) decision 1 is that
   * parity is **per capability, not per method name**, and its alternatives
   * reject method-name parity in as many words. Until 2026-09-08
   * `verify-sdk-parity` could not express that — it keyed a row on one
   * spelling — and this method was named `mark_uncollectible` to satisfy the
   * gate. It is `markUncollectible` now because the gate learned the
   * distinction instead: `PARITY_COLUMN_SPELLINGS` in `.xtask/src/main.rs`
   * carries the one entry, and an entry that exempts nothing fails the build.
   */
  async markUncollectible(
    id: string,
    options?: RequestOptions,
  ): Promise<Invoice> {
    return this.#transition(id, "mark_uncollectible", options);
  }

  /**
   * `POST /v1/invoices/{id}/pay` — mints a payment intent and a hosted
   * checkout for what is left. **Open only.**
   *
   * **This does not charge anybody.** Stripe's `pay` charges a stored payment
   * method; vpay has none, so this answers the invoice with
   * {@link Invoice.payment_intent} and {@link Invoice.hosted_invoice_url}
   * set — a page to send the payer to. The invoice stays `"open"` until the
   * settlement moves it, which is when `invoice.paid` is emitted.
   *
   * One payment at a time: calling this again while the attached intent is
   * live is a `409`, and cancelling that intent is the way back. Two
   * concurrent calls attach exactly one intent.
   *
   * A `500 checkout_not_configured` when this deployment serves no checkout
   * page.
   */
  async pay(
    id: string,
    params: PayInvoiceParams,
    options?: RequestOptions,
  ): Promise<Invoice> {
    return this.#http.request<Invoice>(
      "POST",
      `/invoices/${encodeURIComponent(id)}/pay`,
      { success_url: params.success_url, cancel_url: params.cancel_url },
      options,
    );
  }

  /**
   * The three transitions that take no parameters, which are one shape:
   * a `POST` with an **empty body** to the invoice's own sub-path, carrying
   * an `Idempotency-Key` like every other write.
   *
   * Private, so it is not a capability the parity gate has to record — and
   * written once rather than three times, because the empty body is the part
   * that is easy to get wrong in only one of them.
   */
  async #transition(
    id: string,
    name: "finalize" | "void" | "mark_uncollectible",
    options?: RequestOptions,
  ): Promise<Invoice> {
    return this.#http.request<Invoice>(
      "POST",
      `/invoices/${encodeURIComponent(id)}/${name}`,
      {},
      options,
    );
  }
}

/**
 * `client.invoiceItems` — the four operations on `/v1/invoice_items`: the
 * lines of a **draft** invoice.
 *
 * There is deliberately no `list`: an invoice's lines are read off the
 * invoice ({@link Invoice.lines}, expanded on every render), and a
 * merchant-wide list of lines across invoices answers no question anybody
 * asked. The server mounts no collection `GET` either, so this is not an
 * omission here.
 *
 * Every write requires the parent to still be a `"draft"`;
 * {@link InvoiceItemsResource.retrieve} does not.
 */
export class InvoiceItemsResource {
  readonly #http: HttpClient;

  constructor(http: HttpClient) {
    this.#http = http;
  }

  /**
   * `POST /v1/invoice_items` — adds a line to a draft, and the invoice's
   * `amount_due` moves with it.
   *
   * Throws `TypeError` before any request when `unit_amount` is not a
   * non-negative safe integer; a `400` naming `invoice` when it is not one of
   * your drafts.
   */
  async create(
    params: CreateInvoiceItemParams,
    options?: RequestOptions,
  ): Promise<InvoiceLine> {
    assertIntegerAmount(params.unit_amount, "unit_amount");
    const body: Record<string, FormValue> = {
      invoice: params.invoice,
      description: params.description,
    };
    if (params.quantity !== undefined) {
      body["quantity"] = params.quantity;
    }
    body["unit_amount"] = params.unit_amount;
    return this.#http.request<InvoiceLine>(
      "POST",
      "/invoice_items",
      body,
      options,
    );
  }

  /** `GET /v1/invoice_items/{id}` — readable whatever the parent's status. */
  async retrieve(id: string): Promise<InvoiceLine> {
    return this.#http.request<InvoiceLine>(
      "GET",
      `/invoice_items/${encodeURIComponent(id)}`,
    );
  }

  /**
   * `POST /v1/invoice_items/{id}` — **draft parent only**, with `amount`
   * recomputed.
   *
   * Throws `TypeError` before any request for a `unit_amount` outside the
   * bound; a `409` once the parent is no longer a draft.
   */
  async update(
    id: string,
    params: UpdateInvoiceItemParams,
    options?: RequestOptions,
  ): Promise<InvoiceLine> {
    if (params.unit_amount !== undefined) {
      assertIntegerAmount(params.unit_amount, "unit_amount");
    }
    const body: Record<string, FormValue> = {};
    if (params.description !== undefined) {
      body["description"] = params.description;
    }
    if (params.quantity !== undefined) {
      body["quantity"] = params.quantity;
    }
    if (params.unit_amount !== undefined) {
      body["unit_amount"] = params.unit_amount;
    }
    return this.#http.request<InvoiceLine>(
      "POST",
      `/invoice_items/${encodeURIComponent(id)}`,
      body,
      options,
    );
  }

  /**
   * `DELETE /v1/invoice_items/{id}` — **draft parent only**, and the invoice
   * re-totals.
   */
  async del(id: string, options?: RequestOptions): Promise<DeletedInvoiceItem> {
    return this.#http.request<DeletedInvoiceItem>(
      "DELETE",
      `/invoice_items/${encodeURIComponent(id)}`,
      undefined,
      options,
    );
  }
}
